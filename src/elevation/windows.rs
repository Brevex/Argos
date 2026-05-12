use std::mem::size_of;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::error::ArgosError;

struct ProcessToken(HANDLE);

impl ProcessToken {
    fn for_current_process() -> Result<Self, std::io::Error> {
        let mut handle: HANDLE = std::ptr::null_mut();
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    fn is_elevated(&self) -> Result<bool, std::io::Error> {
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned: u32 = 0;
        let ok = unsafe {
            GetTokenInformation(
                self.0,
                TokenElevation,
                &mut elevation as *mut TOKEN_ELEVATION as *mut _,
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(elevation.TokenIsElevated != 0)
    }
}

impl Drop for ProcessToken {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub fn is_elevated() -> bool {
    ProcessToken::for_current_process()
        .and_then(|t| t.is_elevated())
        .unwrap_or(false)
}

pub fn relaunch_elevated() -> Result<i32, ArgosError> {
    Err(ArgosError::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "Argos requires administrator privileges. Launch the installed shortcut so Windows can prompt for elevation via the embedded UAC manifest.",
    )))
}
