pub use crsql_bundle::sqlite3_crsqlrustbundle_init;

unsafe extern "C" {
    pub unsafe fn sqlite3_crsqlite_init(
        db: *mut crsql_bundle::sqlite::bindings::sqlite3,
        pz_err_msg: *mut *mut u8,
        p_api: *mut crsql_bundle::sqlite::bindings::sqlite3_api_routines,
    ) -> std::ffi::c_int;
}

pub fn init_cr_sqlite_ext() -> i32 {
    unsafe {
        crsql_bundle::sqlite::bindings::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite3_crsqlite_init as *const (),
        )))
    }
}
