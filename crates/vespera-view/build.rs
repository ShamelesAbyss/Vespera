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
    let target = env::var("TARGET").expect("Cargo build target");
    let msvc = target.ends_with("-msvc");
    let mut resources = String::from("1 ICON \"vespera.ico\"\n");
    if env::var_os("CARGO_FEATURE_STORE").is_some() {
        let manifest = "windows-store.manifest";
        println!("cargo:rerun-if-changed={manifest}");
        let staged = out.join(manifest);
        fs::copy(manifest, &staged).expect("stage the Windows Store application manifest");
        if msvc {
            println!("cargo:rustc-link-arg-bin=vespera-view=/MANIFEST:EMBED");
            println!(
                "cargo:rustc-link-arg-bin=vespera-view=/MANIFESTINPUT:{}",
                staged.display()
            );
        } else {
            resources.push_str("1 24 \"windows-store.manifest\"\n");
        }
    }
    fs::write(out.join("vespera.rc"), resources).expect("write Windows resource script");
    let (mut command, resource) = if msvc {
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
    if msvc {
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
