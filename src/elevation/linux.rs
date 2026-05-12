use std::process::Command;

use crate::error::ArgosError;

const PKEXEC: &str = "pkexec";

pub fn is_elevated() -> bool {
    rustix::process::geteuid().is_root()
}

pub fn relaunch_elevated() -> Result<i32, ArgosError> {
    let exe = std::env::current_exe()?;
    let status = Command::new(PKEXEC)
        .arg(exe)
        .args(std::env::args_os().skip(1))
        .status()?;
    Ok(status.code().unwrap_or(1))
}
