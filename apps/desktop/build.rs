use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/app.rc");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
        let obj_path = out_dir.join("app_icon.o");

        let status = Command::new("windres")
            .args([
                "-I",
                "assets",
                "-i",
                "assets/app.rc",
                "-o",
                obj_path.to_str().expect("utf-8 obj path"),
                "-O",
                "coff",
            ])
            .status()
            .expect("failed to invoke windres to embed Windows icon");

        assert!(status.success(), "windres failed with status {status}");
        println!("cargo:rustc-link-arg={}", obj_path.display());
    }
}
