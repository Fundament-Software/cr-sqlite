use core::{
    ffi::{c_int, c_void},
    ptr::null,
};

use sqlite_nostd::ResultCode;

use crate::c::crsql_ExtData;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn crsql_commit_hook(user_data: *mut c_void) -> c_int {
    unsafe {
        let ext_data = user_data as *mut crsql_ExtData;

        if (*ext_data).pendingDbVersion > -1 {
            (*ext_data).dbVersion = (*ext_data).pendingDbVersion;
        }

        commit_or_rollback_reset(ext_data);

        ResultCode::OK as c_int
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn crsql_rollback_hook(user_data: *mut c_void) -> *const c_void {
    unsafe {
        commit_or_rollback_reset(user_data as *mut crsql_ExtData);
        null()
    }
}

pub unsafe fn commit_or_rollback_reset(ext_data: *mut crsql_ExtData) {
    unsafe {
        (*ext_data).pendingDbVersion = -1;
        (*ext_data).seq = 0;
        (*ext_data).timestamp = 0;
        (*ext_data).updatedTableInfosThisTx = 0;
    }
}
