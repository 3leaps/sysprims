use sysprims_core::{SysprimsError, SysprimsResult};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER,
};
use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

pub fn kill_impl(pid: u32, signal: i32) -> SysprimsResult<()> {
    // Windows does not support POSIX signals. For v0.1.0 we treat SIGTERM and
    // SIGKILL as "terminate"; other signals are explicit NotSupported.
    let supported = [
        rsfulmen::foundry::signals::SIGTERM,
        rsfulmen::foundry::signals::SIGKILL,
    ];

    if !supported.contains(&signal) {
        return Err(SysprimsError::not_supported(
            format!("signal {signal}"),
            "windows",
        ));
    }

    unsafe {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_signal_is_not_supported() {
        let err = kill_impl(1234, rsfulmen::foundry::signals::SIGINT).unwrap_err();
        assert!(matches!(err, SysprimsError::NotSupported { .. }));
    }
}
