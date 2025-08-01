unsafe extern "C" {
    fn cr_sqlite_ext_autoinit() -> std::ffi::c_int;
}

pub fn init_cr_sqlite_ext() -> i32 {
    unsafe { cr_sqlite_ext_autoinit() }
}
