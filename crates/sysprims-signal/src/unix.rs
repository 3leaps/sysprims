use std::io;

use libc::{kill as libc_kill, killpg as libc_killpg, EINVAL, EPERM, ESRCH};

use sysprims_core::{SysprimsError, SysprimsResult};

pub fn kill_impl(pid: u32, signal: i32) -> SysprimsResult<()> {
    // Safe: libc expects pid_t (signed), but we reject pid==0 at API boundary.
    let result = unsafe { libc_kill(pid as i32, signal) };

    if result == 0 {
        return Ok(());
    }

    let os_error = io::Error::last_os_error();
    let errno = os_error.raw_os_error().unwrap_or(0);

    match errno {
        EPERM => Err(SysprimsError::permission_denied(pid, "signal")),
        ESRCH => Err(SysprimsError::not_found(pid)),
        EINVAL => Err(SysprimsError::invalid_argument(format!(
            "invalid signal: {signal}"
        ))),
        _ => Err(SysprimsError::system(os_error.to_string(), errno)),
    }
}

pub fn killpg_impl(pgid: u32, signal: i32) -> SysprimsResult<()> {
    let result = unsafe { libc_killpg(pgid as i32, signal) };

    if result == 0 {
        return Ok(());
    }

    let os_error = io::Error::last_os_error();
    let errno = os_error.raw_os_error().unwrap_or(0);

    match errno {
        EPERM => Err(SysprimsError::permission_denied(pgid, "signal_group")),
        ESRCH => Err(SysprimsError::not_found(pgid)),
        EINVAL => Err(SysprimsError::invalid_argument(format!(
            "invalid signal: {signal}"
        ))),
        _ => Err(SysprimsError::system(os_error.to_string(), errno)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_nonexistent_pid_returns_not_found_or_permission_denied() {
        // Use a high but safe PID that's extremely unlikely to exist.
        //
        // SAFETY: We use 99999 instead of u32::MAX because:
        // - u32::MAX (4294967295) wraps to -1 when cast to i32
        // - kill(-1, sig) is POSIX for "signal ALL processes you can signal"
        // - This would terminate Finder, Terminal, and everything else!
        //
        // See docs/safety/signal-dispatch.md for full explanation.
        //
        // Some systems may return EPERM if pid exists but is protected.
        let result = kill_impl(99999, rsfulmen::foundry::signals::SIGTERM);
        assert!(matches!(
            result,
            Err(SysprimsError::NotFound { .. } | SysprimsError::PermissionDenied { .. })
        ));
    }

    #[test]
    fn kill_invalid_signal_returns_invalid_argument_or_system() {
        // Test with current process to avoid touching system processes.
        //
        // SAFETY: We use our own PID instead of PID 1 because:
        // - PID 1 is launchd (macOS) or init (Linux) - the system init process
        // - Signaling PID 1 could have unexpected system-wide effects
        //
        // Signal -1 is invalid on most systems and should return EINVAL.
        // We accept System as an escape hatch for platform variance.
        let our_pid = std::process::id();
        let result = kill_impl(our_pid, -1);
        assert!(matches!(
            result,
            Err(SysprimsError::InvalidArgument { .. } | SysprimsError::System { .. })
        ));
    }
}
