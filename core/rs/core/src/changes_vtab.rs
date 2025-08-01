extern crate alloc;
use crate::alloc::string::ToString;
use crate::changes_vtab_write::crsql_merge_insert;
use crate::stmt_cache::reset_cached_stmt;
use crate::tableinfo::{TableInfo, crsql_ensure_table_infos_are_up_to_date};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::{CStr, c_char, c_int};
use core::mem::{self, forget};
use core::ptr::null_mut;

use alloc::ffi::CString;
#[cfg(not(feature = "std"))]
use num_traits::FromPrimitive;
use sqlite::{ColumnType, Connection, Context, Stmt, Value};
use sqlite_nostd as sqlite;
use sqlite_nostd::ResultCode;

use crate::c::{
    ChangeRowType, ClockUnionColumn, CrsqlChangesColumn, crsql_Changes_cursor, crsql_Changes_vtab,
};
use crate::changes_vtab_read::changes_union_query;
use crate::pack_columns::bind_package_to_stmt;
use crate::pack_columns::unpack_columns;

fn changes_crsr_finalize(crsr: *mut crsql_Changes_cursor) -> c_int {
    // Assign pointers to null after freeing
    // since we can get into this twice for the same cursor object.
    unsafe {
        let mut rc = 0;
        rc += match (*crsr).pChangesStmt.finalize() {
            Ok(rc) => rc as c_int,
            Err(rc) => rc as c_int,
        };
        (*crsr).pChangesStmt = null_mut();
        let reset_rc = reset_cached_stmt((*crsr).pRowStmt);
        match reset_rc {
            Ok(r) | Err(r) => rc += r as c_int,
        }
        (*crsr).pRowStmt = null_mut();
        (*crsr).dbVersion = crate::consts::MIN_POSSIBLE_DB_VERSION;

        rc
    }
}

// A very c-style port. We can get more idiomatic once we finish the rust port and have test and perf parity
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crsql_changes_best_index(
    vtab: *mut sqlite::vtab,
    index_info: *mut sqlite::index_info,
) -> c_int {
    match changes_best_index(vtab, index_info) {
        Ok(rc) => rc as c_int,
        Err(rc) => rc as c_int,
    }
}

fn changes_best_index(
    _vtab: *mut sqlite::vtab,
    index_info: *mut sqlite::index_info,
) -> Result<ResultCode, ResultCode> {
    let mut idx_num: i32 = 0;

    let mut first_constraint = true;
    let mut str = String::new();
    let constraints = sqlite::args!((*index_info).nConstraint, (*index_info).aConstraint);
    let constraint_usage =
        sqlite::args_mut!((*index_info).nConstraint, (*index_info).aConstraintUsage);
    let mut arg_v_index = 1;
    for (i, constraint) in constraints.iter().enumerate() {
        if !constraint_is_usable(constraint) {
            continue;
        }
        let col = CrsqlChangesColumn::from_i32(constraint.iColumn);
        if let Some(col_name) = get_clock_table_col_name(&col) {
            if let Some(op_string) = get_operator_string(constraint.op) {
                if first_constraint {
                    str.push_str("WHERE ");
                    first_constraint = false
                } else {
                    str.push_str(" AND ");
                }

                if constraint.op == sqlite::INDEX_CONSTRAINT_ISNOTNULL as u8
                    || constraint.op == sqlite::INDEX_CONSTRAINT_ISNULL as u8
                {
                    str.push_str(&format!("{} {}", col_name, op_string));
                    constraint_usage[i].argvIndex = 0;
                    constraint_usage[i].omit = 1;
                } else {
                    str.push_str(&format!("{} {} ?", col_name, op_string));
                    constraint_usage[i].argvIndex = arg_v_index;
                    constraint_usage[i].omit = 1;
                    arg_v_index += 1;
                }
            }
        }

        // idx bit mask
        match col {
            Some(CrsqlChangesColumn::DbVrsn) => idx_num |= 2,
            Some(CrsqlChangesColumn::SiteId) => idx_num |= 4,
            _ => {}
        }
    }

    let mut desc = 0;
    let order_bys = sqlite::args!((*index_info).nOrderBy, (*index_info).aOrderBy);
    let mut order_by_consumed = true;
    if !order_bys.is_empty() {
        str.push_str(" ORDER BY ");
    } else {
        // The user didn't provide an ordering? Tack on a default one that will
        // retrieve changes in-order
        str.push_str(" ORDER BY db_vrsn, seq ASC");
    }
    first_constraint = true;
    for order_by in order_bys {
        desc = order_by.desc;
        let col = CrsqlChangesColumn::from_i32(order_by.iColumn);
        if let Some(col_name) = get_clock_table_col_name(&col) {
            if first_constraint {
                first_constraint = false;
            } else {
                str.push_str(", ");
            }
            str.push_str(&col_name);
        } else {
            // TODO: test we're consuming
            order_by_consumed = false;
        }
    }

    if !order_bys.is_empty() {
        if desc != 0 {
            str.push_str(" DESC");
        } else {
            str.push_str(" ASC");
        }
    }

    // manual null-term since we'll pass to C
    str.push('\0');

    // TODO: update your order by py test to explain query plans to ensure correct indices are selected
    // both constraints are present. Also to check that order by is consumed.
    if idx_num & 6 == 6 {
        unsafe {
            (*index_info).estimatedCost = 1.0;
            (*index_info).estimatedRows = 1;
        }
    }
    // only the version constraint is present
    else if idx_num & 2 == 2 {
        unsafe {
            (*index_info).estimatedCost = 10.0;
            (*index_info).estimatedRows = 10;
        }
    }
    // no constraints are present or only the requestor constraint is present
    else {
        unsafe {
            (*index_info).estimatedCost = 2147483647.0;
            (*index_info).estimatedRows = 2147483647;
        }
    }

    unsafe {
        (*index_info).idxNum = idx_num;
        (*index_info).orderByConsumed = if order_by_consumed { 1 } else { 0 };
        // forget str
        let ptr = str.as_ptr();
        core::mem::forget(str);
        // pass to c. We've manually null terminated the string.
        // sqlite will free it for us.
        (*index_info).idxStr = ptr as *mut c_char;
        (*index_info).needToFreeIdxStr = 1;
    }

    Ok(ResultCode::OK)
}

fn constraint_is_usable(constraint: &sqlite::bindings::sqlite3_index_constraint) -> bool {
    if constraint.usable == 0 {
        return false;
    }
    if let Some(col) = CrsqlChangesColumn::from_i32(constraint.iColumn) {
        !matches!(
            col,
            CrsqlChangesColumn::Tbl | CrsqlChangesColumn::Pk | CrsqlChangesColumn::Cval
        )
    } else {
        false
    }
}

// Note: this is really the col name post-select from the clock table.
fn get_clock_table_col_name(col: &Option<CrsqlChangesColumn>) -> Option<String> {
    match col {
        Some(CrsqlChangesColumn::Tbl) => Some("tbl".to_string()),
        Some(CrsqlChangesColumn::Pk) => Some("pks".to_string()),
        Some(CrsqlChangesColumn::Cid) => Some("cid".to_string()),
        Some(CrsqlChangesColumn::Cval) => None,
        Some(CrsqlChangesColumn::ColVrsn) => Some("col_vrsn".to_string()),
        Some(CrsqlChangesColumn::DbVrsn) => Some("db_vrsn".to_string()),
        Some(CrsqlChangesColumn::SiteId) => Some("site_id".to_string()),
        Some(CrsqlChangesColumn::Seq) => Some("seq".to_string()),
        Some(CrsqlChangesColumn::Cl) => Some("cl".to_string()),
        Some(CrsqlChangesColumn::Ts) => Some("ts".to_string()),
        None => None,
    }
}

fn get_operator_string(op: u8) -> Option<String> {
    // TODO: convert to proper enum
    match op as i32 {
        sqlite::INDEX_CONSTRAINT_EQ => Some("=".to_string()),
        sqlite::INDEX_CONSTRAINT_GT => Some(">".to_string()),
        sqlite::INDEX_CONSTRAINT_LE => Some("<=".to_string()),
        sqlite::INDEX_CONSTRAINT_LT => Some("<".to_string()),
        sqlite::INDEX_CONSTRAINT_GE => Some(">=".to_string()),
        sqlite::INDEX_CONSTRAINT_MATCH => Some("MATCH".to_string()),
        sqlite::INDEX_CONSTRAINT_LIKE => Some("LIKE".to_string()),
        sqlite::INDEX_CONSTRAINT_GLOB => Some("GLOB".to_string()),
        sqlite::INDEX_CONSTRAINT_REGEXP => Some("REGEXP".to_string()),
        sqlite::INDEX_CONSTRAINT_NE => Some("!=".to_string()),
        sqlite::INDEX_CONSTRAINT_ISNOT => Some("IS NOT".to_string()),
        sqlite::INDEX_CONSTRAINT_ISNOTNULL => Some("IS NOT NULL".to_string()),
        sqlite::INDEX_CONSTRAINT_ISNULL => Some("IS NULL".to_string()),
        sqlite::INDEX_CONSTRAINT_IS => Some("IS".to_string()),
        _ => None,
    }
}

// This'll become safe once more code is moved over to Rust
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crsql_changes_filter(
    cursor: *mut sqlite::vtab_cursor,
    _idx_num: c_int,
    idx_str: *const c_char,
    argc: c_int,
    argv: *mut *mut sqlite::value,
) -> c_int {
    let args = sqlite::args!(argc, argv);
    let cursor = cursor.cast::<crsql_Changes_cursor>();
    let idx_str = unsafe { CStr::from_ptr(idx_str).to_str() };
    match idx_str {
        Ok(idx_str) => match unsafe { changes_filter(cursor, idx_str, args) } {
            Err(rc) | Ok(rc) => rc as c_int,
        },
        Err(_) => ResultCode::FORMAT as c_int,
    }
}

unsafe fn changes_filter(
    cursor: *mut crsql_Changes_cursor,
    idx_str: &str,
    args: &[*mut sqlite::value],
) -> Result<ResultCode, ResultCode> {
    unsafe {
        let tab = (*cursor).pTab;
        let db = (*tab).db;
        // This should never happen. pChangesStmt should be finalized
        // before filter is ever invoked.
        if !(*cursor).pChangesStmt.is_null() {
            (*cursor).pChangesStmt.finalize()?;
            (*cursor).pChangesStmt = null_mut();
        }

        let c_rc = crsql_ensure_table_infos_are_up_to_date(
            db,
            (*tab).pExtData,
            &mut (*tab).base.zErrMsg as *mut _,
        );
        if c_rc != 0 {
            if let Some(rc) = ResultCode::from_i32(c_rc) {
                return Err(rc);
            } else {
                return Err(ResultCode::ERROR);
            }
        }

        // nothing to fetch, no crrs exist.
        let tbl_infos = mem::ManuallyDrop::new(Box::from_raw(
            (*(*tab).pExtData).tableInfos as *mut Vec<TableInfo>,
        ));
        if tbl_infos.len() == 0 {
            return Ok(ResultCode::OK);
        }

        let sql = changes_union_query(&tbl_infos, idx_str)?;

        let stmt = db.prepare_v2(&sql)?;
        for (i, arg) in args.iter().enumerate() {
            stmt.bind_value(i as i32 + 1, *arg)?;
        }
        (*cursor).pChangesStmt = stmt.stmt;
        // forget the stmt. it will be managed by the vtab
        forget(stmt);
        changes_next(cursor, (*cursor).pTab.cast::<sqlite::vtab>())
    }
}

/**
 * Advances our Changes_cursor to its next row of output.
 * TODO: this'll get more idiomatic as we move dependencies to Rust
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crsql_changes_next(cursor: *mut sqlite::vtab_cursor) -> c_int {
    unsafe {
        let cursor = cursor.cast::<crsql_Changes_cursor>();
        let vtab = (*cursor).pTab.cast::<sqlite::vtab>();
        match changes_next(cursor, vtab) {
            Ok(rc) => rc as c_int,
            Err(rc) => {
                changes_crsr_finalize(cursor);
                rc as c_int
            }
        }
    }
}

// We'll get more idiomatic once we have more Rust and less C
unsafe fn changes_next(
    cursor: *mut crsql_Changes_cursor,
    vtab: *mut sqlite::vtab,
) -> Result<ResultCode, ResultCode> {
    unsafe {
        if (*cursor).pChangesStmt.is_null() {
            let err = CString::new("pChangesStmt is null in changes_next")?;
            (*vtab).zErrMsg = err.into_raw();
            return Err(ResultCode::ABORT);
        }

        if !(*cursor).pRowStmt.is_null() {
            let rc = reset_cached_stmt((*cursor).pRowStmt);
            (*cursor).pRowStmt = null_mut();
            rc?;
        }

        let rc = (*cursor).pChangesStmt.step()?;
        if rc == ResultCode::DONE {
            let c_rc = changes_crsr_finalize(cursor);
            if c_rc == 0 {
                return Ok(ResultCode::OK);
            } else {
                return Err(ResultCode::ERROR);
            }
        }

        // we had a row... we can do the rest
        let tbl = (*cursor)
            .pChangesStmt
            .column_text(ClockUnionColumn::Tbl as i32);
        let pks = (*cursor)
            .pChangesStmt
            .column_value(ClockUnionColumn::Pks as i32);
        let cid = (*cursor)
            .pChangesStmt
            .column_text(ClockUnionColumn::Cid as i32);
        let db_version = (*cursor)
            .pChangesStmt
            .column_int64(ClockUnionColumn::DbVrsn as i32);
        let changes_rowid = (*cursor)
            .pChangesStmt
            .column_int64(ClockUnionColumn::RowId as i32);
        (*cursor).dbVersion = db_version;

        let tbl_infos = mem::ManuallyDrop::new(Box::from_raw(
            (*(*(*cursor).pTab).pExtData).tableInfos as *mut Vec<TableInfo>,
        ));
        // TODO: will this work given `insert_tbl` is null termed?
        let tbl_info_index = tbl_infos.iter().position(|x| x.tbl_name == tbl);

        if tbl_info_index.is_none() {
            let err = CString::new(format!("could not find schema for table {}", tbl))?;
            (*vtab).zErrMsg = err.into_raw();
            return Err(ResultCode::ERROR);
        }
        // TODO: technically safe since we checked `is_none` but this should be more idiomatic
        let tbl_info_index = tbl_info_index.unwrap();

        let tbl_info = &tbl_infos[tbl_info_index];
        (*cursor).changesRowid = changes_rowid;
        (*cursor).tblInfoIdx = tbl_info_index as i32;

        if tbl_info.pks.is_empty() {
            let err = CString::new(format!("crr {} is missing primary keys", tbl))?;
            (*vtab).zErrMsg = err.into_raw();
            return Err(ResultCode::ERROR);
        }

        if cid == crate::c::DELETE_SENTINEL {
            (*cursor).rowType = ChangeRowType::Delete as c_int;
            return Ok(ResultCode::OK);
        } else if cid == crate::c::INSERT_SENTINEL {
            (*cursor).rowType = ChangeRowType::PkOnly as c_int;
            return Ok(ResultCode::OK);
        } else {
            (*cursor).rowType = ChangeRowType::Update as c_int;
        }

        let row_stmt_ref = tbl_info.get_row_patch_data_stmt((*(*cursor).pTab).db, cid)?;
        let row_stmt = row_stmt_ref.as_ref().ok_or(ResultCode::ERROR)?;

        let packed_pks = pks.blob();
        let unpacked_pks = unpack_columns(packed_pks)?;
        bind_package_to_stmt(row_stmt.stmt, &unpacked_pks, 0)?;

        match row_stmt.step() {
            Ok(ResultCode::DONE) => {
                reset_cached_stmt(row_stmt.stmt)?;
            }
            Ok(_) => {}
            Err(rc) => {
                reset_cached_stmt(row_stmt.stmt)?;
                return Err(rc);
            }
        }

        (*cursor).pRowStmt = row_stmt.stmt;
        Ok(ResultCode::OK)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn crsql_changes_eof(cursor: *mut sqlite::vtab_cursor) -> c_int {
    let cursor = cursor.cast::<crsql_Changes_cursor>();
    if unsafe { (*cursor).pChangesStmt.is_null() } {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn crsql_changes_column(
    cursor: *mut sqlite::vtab_cursor, /* The cursor */
    ctx: *mut sqlite::context,        /* First argument to sqlite3_result_...() */
    i: c_int,                         /* Which column to return */
) -> c_int {
    match column_impl(cursor, ctx, i) {
        Ok(code) | Err(code) => code as c_int,
    }
}

fn column_impl(
    cursor: *mut sqlite::vtab_cursor,
    ctx: *mut sqlite::context,
    i: c_int,
) -> Result<ResultCode, ResultCode> {
    let cursor = cursor.cast::<crsql_Changes_cursor>();
    let column = CrsqlChangesColumn::from_i32(i);
    // TODO: only de-reference where needed?
    let changes_stmt = unsafe { (*cursor).pChangesStmt };
    match column {
        Some(CrsqlChangesColumn::Tbl) => {
            ctx.result_value(changes_stmt.column_value(ClockUnionColumn::Tbl as i32));
        }
        Some(CrsqlChangesColumn::Pk) => {
            ctx.result_value(changes_stmt.column_value(ClockUnionColumn::Pks as i32));
        }
        Some(CrsqlChangesColumn::Cval) => unsafe {
            if (*cursor).pRowStmt.is_null() {
                ctx.result_null();
            } else {
                ctx.result_value((*cursor).pRowStmt.column_value(0));
            }
        },
        Some(CrsqlChangesColumn::Cid) => unsafe {
            let row_type = ChangeRowType::from_i32((*cursor).rowType);
            match row_type {
                Some(ChangeRowType::PkOnly) => ctx.result_text_static(crate::c::INSERT_SENTINEL),
                Some(ChangeRowType::Delete) => ctx.result_text_static(crate::c::DELETE_SENTINEL),
                Some(ChangeRowType::Update) => {
                    if (*cursor).pRowStmt.is_null() {
                        ctx.result_text_static(crate::c::DELETE_SENTINEL);
                    } else {
                        ctx.result_value(changes_stmt.column_value(ClockUnionColumn::Cid as i32));
                    }
                }
                None => return Err(ResultCode::ABORT),
            }
        },
        Some(CrsqlChangesColumn::ColVrsn) => {
            ctx.result_value(changes_stmt.column_value(ClockUnionColumn::ColVrsn as i32));
        }
        Some(CrsqlChangesColumn::DbVrsn) => {
            ctx.result_value(changes_stmt.column_value(ClockUnionColumn::DbVrsn as i32));
        }
        Some(CrsqlChangesColumn::SiteId) => {
            // todo: short circuit null? if col type null bind null rather than value?
            // sholdn't matter..
            ctx.result_value(changes_stmt.column_value(ClockUnionColumn::SiteId as i32));
        }
        Some(CrsqlChangesColumn::Seq) => {
            ctx.result_value(changes_stmt.column_value(ClockUnionColumn::Seq as i32));
        }
        Some(CrsqlChangesColumn::Cl) => {
            ctx.result_value(changes_stmt.column_value(ClockUnionColumn::Cl as i32))
        }
        Some(CrsqlChangesColumn::Ts) => {
            ctx.result_value(changes_stmt.column_value(ClockUnionColumn::Ts as i32));
        }
        None => return Err(ResultCode::MISUSE),
    }

    Ok(ResultCode::OK)
}

#[unsafe(no_mangle)]
pub extern "C" fn crsql_changes_rowid(
    cursor: *mut sqlite::vtab_cursor,
    rowid: *mut sqlite::int64,
) -> c_int {
    let cursor = cursor.cast::<crsql_Changes_cursor>();
    unsafe {
        *rowid = crate::util::slab_rowid((*cursor).tblInfoIdx, (*cursor).changesRowid);
        if *rowid < 0 {
            return ResultCode::ERROR as c_int;
        }
    }
    ResultCode::OK as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn crsql_changes_update(
    vtab: *mut sqlite::vtab,
    argc: c_int,
    argv: *mut *mut sqlite::value,
    row_id: *mut sqlite::int64,
) -> c_int {
    let args = sqlite::args!(argc, argv);
    let arg = args[0];
    if args.len() > 1 && arg.value_type() == ColumnType::Null {
        // insert statement
        // argv[1] is the rowid.. but why would it ever be filled for us?
        let mut err_msg = null_mut();
        let rc = unsafe { crsql_merge_insert(vtab, argc, argv, row_id, &mut err_msg as *mut _) };
        if rc != ResultCode::OK as c_int {
            unsafe {
                (*vtab).zErrMsg = err_msg;
            }
        }
        rc
    } else if let Ok(err) = CString::new(
        "Only INSERT and SELECT statements are allowed against the crsql changes table",
    ) {
        unsafe {
            (*vtab).zErrMsg = err.into_raw();
        }
        ResultCode::MISUSE as c_int
    } else {
        ResultCode::NOMEM as c_int
    }
}

// If xBegin is not defined xCommit is not called.
#[unsafe(no_mangle)]
pub extern "C" fn crsql_changes_begin(_vtab: *mut sqlite::vtab) -> c_int {
    ResultCode::OK as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn crsql_changes_commit(vtab: *mut sqlite::vtab) -> c_int {
    let tab = vtab.cast::<crsql_Changes_vtab>();
    unsafe {
        (*(*tab).pExtData).rowsImpacted = 0;
    }
    ResultCode::OK as c_int
}
