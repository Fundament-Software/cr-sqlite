use eyre::eyre;
use std::{env, fs::OpenOptions, io::Read, io::Write, path::PathBuf};

static EXT_FILES: &[&str] = &["./crsqlite.c", "./changes-vtab.c", "./ext-data.c"];

/*static EXT_HEADERS: &[&str] = &[
    "./crsqlite.h",
    "./util.h",
    "./changes-vtab.h",
    "./ext-data.h",
];*/

fn main() -> eyre::Result<()> {
    let mut build = cc::Build::new();
    let out_dir: PathBuf = env::var_os("OUT_DIR")
        .ok_or_else(|| eyre!("OUT_DIR not set"))?
        .into();

    // Note, the original script calls make shell.c and make sqlite3.c, but this does nothing. The purpose of this is unclear.

    /*
    println!("cargo:rerun-if-changed={}", "./sqlite/sqlite3.c");
    let sqlite3_c = out_dir.join("sqlite3.c");
    std::fs::copy("./sqlite/sqlite3.c", &sqlite3_c)?;

    {
        let mut sqlite3 = OpenOptions::new()
            .write(true)
            .append(true)
            .open(&sqlite3_c)?;

        let mut core_init = String::new();
        std::fs::File::open("core_init.c")?.read_to_string(&mut core_init)?;
        writeln!(sqlite3, "{core_init}")?;
    }
    build.file(sqlite3_c);
    */

    for file in EXT_FILES {
        build.file(file);
        println!("cargo:rerun-if-changed={file}");
    }

    build.define("HAVE_GETHOSTUUID", "0");
    build.include(".");
    build.include("./sqlite");
    build.pic(true);
    build.std("c11"); // Note: original code wants C99
    build.opt_level(3);

    // Note: Android/iOS support will need a sysroot option

    build.compile("crsqlite");

    // cd $(prefix)/temp && ar -x ../libcrsql_$(bundle)-static.a && ar -rc crsqlite.a *.o && mv crsqlite.a ../crsqlite-$(CI_MAYBE_TARGET).a

    Ok(())
}
