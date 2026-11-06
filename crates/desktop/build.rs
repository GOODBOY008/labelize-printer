use std::{env, path::PathBuf, process::Command};

fn main() {
    if cfg!(windows) || env::var("TARGET").as_deref() != Ok("x86_64-pc-windows-gnu") {
        return;
    }

    // GPUI's manifest build runs only on Windows hosts. Link the same
    // Common Controls v6 and DPI settings when cross-compiling with MinGW.
    let gpui = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/gpui");
    let resource = "resources/windows/gpui.rc";
    println!("cargo:rerun-if-changed={}", gpui.join(resource).display());
    println!(
        "cargo:rerun-if-changed={}",
        gpui.join("resources/windows/gpui.manifest.xml").display()
    );
    let object =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR")).join("gpui-manifest.o");
    let status = Command::new("x86_64-w64-mingw32-windres")
        .current_dir(gpui)
        .args(["-O", "coff", "-i", resource, "-o"])
        .arg(&object)
        .status()
        .expect("MinGW windres is required to embed the GPUI Windows manifest");
    assert!(
        status.success(),
        "Failed to compile the GPUI Windows manifest"
    );
    println!(
        "cargo:rustc-link-arg-bin=labelize-printer={}",
        object.display()
    );
}
