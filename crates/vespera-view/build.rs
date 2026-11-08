use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if env::var_os("CARGO_FEATURE_GRAPHICS").is_none()
        || env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
    {
        return;
    }
    let icon = "../../assets/branding/vespera.ico";
    println!("cargo:rerun-if-changed={icon}");
    println!("cargo:rerun-if-env-changed=RC");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    fs::copy(icon, out.join("vespera.ico")).expect("stage the official Windows icon");
    fs::write(out.join("vespera.rc"), "1 ICON \"vespera.ico\"\n")
        .expect("write Windows resource script");
    let target = env::var("TARGET").expect("Cargo build target");
    let (mut command, resource) = if target.ends_with("-msvc") {
        let command = env::var_os("RC").map_or_else(
            || {
                cc::windows_registry::find_tool(&target, "rc.exe")
                    .expect("Windows SDK rc.exe is required to build Vespera graphics")
                    .to_command()
            },
            Command::new,
        );
        (command, "vespera.res")
    } else {
        let command = Command::new(env::var_os("RC").unwrap_or_else(|| "windres".into()));
        (command, "vespera-icon.o")
    };
    if target.ends_with("-msvc") {
        command.args(["/nologo", "/fo", resource, "vespera.rc"]);
    } else {
        command.args(["-O", "coff", "-i", "vespera.rc", "-o", resource]);
    }
    assert!(
        command
            .current_dir(&out)
            .status()
            .expect("run Windows resource compiler")
            .success(),
        "compile the Vespera icon resource",
    );
    for binary in ["vespera-view", "vespera-launcher"] {
        println!(
            "cargo:rustc-link-arg-bin={binary}={}",
            out.join(resource).display()
        );
    }
}
