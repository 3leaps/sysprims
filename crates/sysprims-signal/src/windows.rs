use sysprims_core::{SysprimsError, SysprimsResult};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER,
};
use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_C_EVENT};
use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

pub fn kill_impl(pid: u32, signal: i32) -> SysprimsResult<()> {
    // Windows does not support POSIX signals. For v0.1.0 we:
    // - Map SIGTERM/SIGKILL to TerminateProcess
    // - Best-effort SIGINT via GenerateConsoleCtrlEvent
    match signal {
        rsfulmen::foundry::signals::SIGTERM | rsfulmen::foundry::signals::SIGKILL => unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if handle == 0 {
                let error = GetLastError();
                return match error {
                    ERROR_ACCESS_DENIED => Err(SysprimsError::permission_denied(pid, "terminate")),
                    ERROR_INVALID_PARAMETER => Err(SysprimsError::not_found(pid)),
                    _ => Err(SysprimsError::system(
                        "OpenProcess failed".to_string(),
                        error as i32,
                    )),
                };
            }

            // Exit code `1` is arbitrary; callers should use sysprims-timeout if they
            // need nuanced outcome semantics.
            let ok = TerminateProcess(handle, 1);
            let term_error = GetLastError();
            CloseHandle(handle);

            if ok != 0 {
                Ok(())
            } else {
                Err(SysprimsError::system(
                    "TerminateProcess failed".to_string(),
                    term_error as i32,
                ))
            }
        },
        rsfulmen::foundry::signals::SIGINT => unsafe {
            let ok = GenerateConsoleCtrlEvent(CTRL_C_EVENT, pid);
            if ok != 0 {
                Ok(())
            } else {
                let error = GetLastError();
                match error {
                    ERROR_ACCESS_DENIED => Err(SysprimsError::permission_denied(pid, "signal")),
                    ERROR_INVALID_PARAMETER => Err(SysprimsError::not_found(pid)),
                    _ => Err(SysprimsError::system(
                        "GenerateConsoleCtrlEvent failed".to_string(),
                        error as i32,
                    )),
                }
            }
        },
        _ => Err(SysprimsError::not_supported(
            format!("signal {signal}"),
            "windows",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_signal_is_not_supported() {
        let err = kill_impl(1234, rsfulmen::foundry::signals::SIGHUP).unwrap_err();
        assert!(matches!(err, SysprimsError::NotSupported { .. }));
    }
}
