pub const TBL_SITE_ID: &'static str = "crsql_site_id";
pub const TBL_SCHEMA: &'static str = "crsql_master";
// pub const CRSQLITE_VERSION_0_15_0: i32 = 15_00_00;
// pub const CRSQLITE_VERSION_0_13_0: i32 = 13_00_00;
// MM_mm_pp_xx
// so a 1.0.0 release is:
// 01_00_00_00 -> 1000000
// a 0.5 release is:
// 00_05_00_00 ->   50000
// a 0.5.1 is:
// 00_05_01_00
// and, if we ever need it, we can track individual builds of a patch release
// 00_05_01_01
pub const CRSQLITE_VERSION: i32 = 17_00_00;
//pub const CRSQLITE_VERSION_STR: &'static str = "0.17.0";
//pub const CRSQLITE_VERSION_0_15_0: i32 = 15_00_00;
pub const CRSQLITE_VERSION_0_17_0: i32 = 17_00_00;

pub const SITE_ID_LEN: i32 = 16;
pub const ROWID_SLAB_SIZE: i64 = 10000000000000;
// db version is a signed 64bit int since sqlite doesn't support saving and
// retrieving unsigned 64bit ints. (2^64 / 2) is a big enough number to write 1
// million entries per second for 3,000 centuries.
pub const MIN_POSSIBLE_DB_VERSION: i64 = 0;
pub const MAX_TBL_NAME_LEN: i32 = 2048;
