static EXT_FILES: &[&str] = &["./c/crsqlite.c", "./c/changes-vtab.c", "./c/ext-data.c"];

/*static EXT_HEADERS: &[&str] = &[
    "./c/crsqlite.h",
    "./c/util.h",
    "./c/changes-vtab.h",
    "./c/ext-data.h",
];*/

fn main() {
    let mut build = cc::Build::new();

    for file in EXT_FILES {
        build.file(file);
        println!("cargo:rerun-if-changed={file}");
    }

    build.define("HAVE_GETHOSTUUID", "0");
    build.include(".");
    build.include("./c/sqlite");
    build.pic(true);
    build.std("c11"); // Note: original code wants C99
    build.opt_level(3);

    // Note: Android/iOS support will need a sysroot option

    //let root = env!("CARGO_MANIFEST_DIR");
    //let profile = std::env::var("PROFILE").unwrap();
    //println!("cargo:rustc-link-lib=./target/{profile}/libcrsql_bundle.rlib");
    build.compile("crsqlite");
}
