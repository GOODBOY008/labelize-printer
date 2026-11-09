use std::{env, fs, path::PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let icon = root.join("platform/windows/labelize-printer.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let resource = out.join("labelize-printer.rc");
    let mut script = format!("1 ICON \"{}\"\n", icon.to_string_lossy().replace('\\', "/"));

    // GPUI links this manifest itself on Windows hosts.
    if !cfg!(windows) {
        let manifest = root.join("vendor/gpui/resources/windows/gpui.manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        script.push_str(&format!("1 24 \"{}\"\n", manifest.display()));
    }
    fs::write(&resource, script).expect("Write the Windows resource script");

    #[cfg(windows)]
    embed_resource::compile_for(&resource, ["labelize-printer"], embed_resource::NONE)
        .manifest_required()
        .expect("Embed the Labelize Studio icon");

    #[cfg(not(windows))]
    {
        assert_eq!(env::var("TARGET").as_deref(), Ok("x86_64-pc-windows-gnu"));
        let object = out.join("labelize-printer-resources.o");
        let status = std::process::Command::new("x86_64-w64-mingw32-windres")
            .args(["-O", "coff", "-i"])
            .arg(&resource)
            .arg("-o")
            .arg(&object)
            .status()
            .expect("MinGW windres is required to embed Windows resources");
        assert!(status.success(), "Failed to compile Windows resources");
        println!(
            "cargo:rustc-link-arg-bin=labelize-printer={}",
            object.display()
        );
    }
}
