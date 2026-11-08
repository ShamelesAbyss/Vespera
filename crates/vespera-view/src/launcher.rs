#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match launch() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Could not launch Vespera: {error}");
            ExitCode::FAILURE
        }
    }
}

fn launch() -> io::Result<()> {
    let launcher = std::env::current_exe()?;
    let dir = launcher
        .parent()
        .ok_or_else(|| io::Error::other("missing executable directory"))?;
    let mut view = Command::new(dir.join(format!("vespera-view{}", std::env::consts::EXE_SUFFIX)));
    view.args(std::env::args_os().skip(1));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        view.creation_flags(CREATE_NO_WINDOW);
    }
    view.spawn()?;
    Ok(())
}
