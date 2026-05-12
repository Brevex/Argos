use crate::error::ArgosError;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
compile_error!(
    "Argos targets only Linux and Windows. See docs/decisions/0009-privilege-elevation.md."
);

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "windows")]
use windows as platform;

#[derive(Debug)]
pub enum Outcome {
    AlreadyElevated,
    Relaunched { exit_code: i32 },
}

pub fn ensure() -> Result<Outcome, ArgosError> {
    if platform::is_elevated() {
        Ok(Outcome::AlreadyElevated)
    } else {
        let exit_code = platform::relaunch_elevated()?;
        Ok(Outcome::Relaunched { exit_code })
    }
}
