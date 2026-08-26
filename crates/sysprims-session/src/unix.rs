//! Unix implementation of session management.
//!
//! Implementation derived from POSIX specifications:
//! - setsid(2): https://pubs.opengroup.org/onlinepubs/9699919799/functions/setsid.html
//! - nohup: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/nohup.html

use std::os::unix::process::CommandExt;
use std::process::Command;
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

use sysprims_core::{SysprimsError, SysprimsResult};

use crate::{NohupConfig, NohupOutcome, SetsidConfig, SetsidOutcome};

// ============================================================================
// setsid implementation
// ============================================================================

pub fn run_setsid_impl(
    command: &str,
    args: &[&str],
    config: &SetsidConfig,
) -> SysprimsResult<SetsidOutcome> {
    let mut cmd = Command::new(command);
    cmd.args(args);
    apply_child_config(&mut cmd, config.cwd.as_deref(), config.env.as_ref());

    // Set up setsid in the child process after fork
    // SAFETY: setsid() is async-signal-safe per POSIX and safe to call after fork
    unsafe {
        cmd.pre_exec(|| {
            // Create new session - the child becomes:
            // 1. Session leader of a new session
            // 2. Process group leader of a new process group
            // 3. Has no controlling terminal
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    // Spawn the child
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SysprimsError::not_found_command(command)
        } else if e.kind() == std::io::ErrorKind::PermissionDenied {
            SysprimsError::permission_denied_command(command)
        } else {
            SysprimsError::spawn_failed(command, e.to_string())
        }
    })?;

    let child_pid = child.id();

    if config.wait {
        // Wait for child to complete
        let status = child.wait().map_err(|e| {
            SysprimsError::system(format!("wait failed: {}", e), e.raw_os_error().unwrap_or(0))
        })?;

        Ok(SetsidOutcome::Completed {
            child_pid,
            exit_status: status,
        })
    } else {
        // Return immediately, child continues in background
        Ok(SetsidOutcome::Spawned { child_pid })
    }
}

// ============================================================================
// nohup implementation
// ============================================================================

pub fn run_nohup_impl(
    command: &str,
    args: &[&str],
    config: &NohupConfig,
) -> SysprimsResult<NohupOutcome> {
    let mut cmd = Command::new(command);
    cmd.args(args);
    apply_child_config(&mut cmd, config.cwd.as_deref(), config.env.as_ref());

    // Determine output file for stdout redirection
    let explicit_output_file = config.output_file.is_some();
    let output_file = determine_nohup_output(config)?;

    // Check if stdout is a terminal
    let stdout_is_tty = unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 };
    let stderr_is_tty = unsafe { libc::isatty(libc::STDERR_FILENO) == 1 };

    // Set up output redirection if needed
    if explicit_output_file || stdout_is_tty {
        if let Some(ref path) = output_file {
            let file = open_no_follow_append(Path::new(path))?;
            cmd.stdout(
                file.try_clone()
                    .map_err(|e| SysprimsError::system(format!("cannot dup stdout: {}", e), 0))?,
            );

            // If the caller chose an explicit target, redirect stderr there too.
            // Otherwise preserve the POSIX nohup behavior of redirecting tty
            // stderr alongside tty stdout.
            if explicit_output_file || stderr_is_tty {
                cmd.stderr(file);
            }
        }
    }

    // Set up SIGHUP ignore in the child
    // SAFETY: signal() is async-signal-safe per POSIX
    unsafe {
        cmd.pre_exec(|| {
            // Ignore SIGHUP so the process survives terminal close
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
            Ok(())
        });
    }

    // Spawn the child
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SysprimsError::not_found_command(command)
        } else if e.kind() == std::io::ErrorKind::PermissionDenied {
            SysprimsError::permission_denied_command(command)
        } else {
            SysprimsError::spawn_failed(command, e.to_string())
        }
    })?;

    let child_pid = child.id();

    if config.wait {
        let status = child.wait().map_err(|e| {
            SysprimsError::system(format!("wait failed: {}", e), e.raw_os_error().unwrap_or(0))
        })?;

        Ok(NohupOutcome::Completed {
            child_pid,
            exit_status: status,
            output_file,
        })
    } else {
        Ok(NohupOutcome::Spawned {
            child_pid,
            output_file,
        })
    }
}

/// Determine the output file for nohup.
///
/// Per POSIX: Try "nohup.out" in current directory, then "$HOME/nohup.out"
fn determine_nohup_output(config: &NohupConfig) -> SysprimsResult<Option<String>> {
    if let Some(ref path) = config.output_file {
        return Ok(Some(path.clone()));
    }

    // Check if stdout is a terminal - if not, no redirection needed
    let stdout_is_tty = unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 };
    if !stdout_is_tty {
        return Ok(None);
    }

    // Try current directory first
    let cwd_path = "nohup.out";
    if open_no_follow_append(Path::new(cwd_path)).is_ok() {
        return Ok(Some(cwd_path.to_string()));
    }

    // Fall back to $HOME/nohup.out
    if let Some(home) = std::env::var_os("HOME") {
        let home_path = format!("{}/nohup.out", home.to_string_lossy());
        return Ok(Some(home_path));
    }

    // Can't determine output file
    Ok(Some(cwd_path.to_string()))
}

fn apply_child_config(
    cmd: &mut Command,
    cwd: Option<&Path>,
    env: Option<&std::collections::BTreeMap<String, String>>,
) {
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    if let Some(env) = env {
        cmd.envs(env);
    }
}

fn open_no_follow_append(path: &Path) -> SysprimsResult<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| map_nohup_output_open_error(path, e))
}

fn map_nohup_output_open_error(path: &Path, err: std::io::Error) -> SysprimsError {
    if err.kind() == std::io::ErrorKind::PermissionDenied || err.raw_os_error() == Some(libc::ELOOP)
    {
        return SysprimsError::permission_denied_path(
            path.display().to_string(),
            "open nohup output_file",
        );
    }

    SysprimsError::system(
        format!("cannot open {}: {}", path.display(), err),
        err.raw_os_error().unwrap_or(0),
    )
}

// ============================================================================
// Low-level session/process group APIs
// ============================================================================

pub fn setsid_impl() -> SysprimsResult<u32> {
    let result = unsafe { libc::setsid() };
    if result == -1 {
        let errno = std::io::Error::last_os_error();
        Err(SysprimsError::system(
            "setsid failed",
            errno.raw_os_error().unwrap_or(0),
        ))
    } else {
        Ok(result as u32)
    }
}

pub fn getsid_impl(pid: u32) -> SysprimsResult<u32> {
    let result = unsafe { libc::getsid(pid as libc::pid_t) };
    if result == -1 {
        let errno = std::io::Error::last_os_error();
        if errno.raw_os_error() == Some(libc::ESRCH) {
            Err(SysprimsError::not_found(pid))
        } else {
            Err(SysprimsError::system(
                "getsid failed",
                errno.raw_os_error().unwrap_or(0),
            ))
        }
    } else {
        Ok(result as u32)
    }
}

pub fn setpgid_impl(pid: u32, pgid: u32) -> SysprimsResult<()> {
    let result = unsafe { libc::setpgid(pid as libc::pid_t, pgid as libc::pid_t) };
    if result == -1 {
        let errno = std::io::Error::last_os_error();
        if errno.raw_os_error() == Some(libc::ESRCH) {
            Err(SysprimsError::not_found(pid))
        } else if errno.raw_os_error() == Some(libc::EPERM) {
            Err(SysprimsError::permission_denied(pid, "setpgid"))
        } else {
            Err(SysprimsError::system(
                "setpgid failed",
                errno.raw_os_error().unwrap_or(0),
            ))
        }
    } else {
        Ok(())
    }
}

pub fn getpgid_impl(pid: u32) -> SysprimsResult<u32> {
    let result = unsafe { libc::getpgid(pid as libc::pid_t) };
    if result == -1 {
        let errno = std::io::Error::last_os_error();
        if errno.raw_os_error() == Some(libc::ESRCH) {
            Err(SysprimsError::not_found(pid))
        } else {
            Err(SysprimsError::system(
                "getpgid failed",
                errno.raw_os_error().unwrap_or(0),
            ))
        }
    } else {
        Ok(result as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setsid_spawns_process() {
        let result = run_setsid_impl("echo", &["hello"], &SetsidConfig::default());
        assert!(result.is_ok());
        if let Ok(SetsidOutcome::Spawned { child_pid }) = result {
            assert!(child_pid > 0);
        }
    }

    #[test]
    fn setsid_wait_returns_status() {
        let result = run_setsid_impl(
            "sh",
            &["-c", "exit 42"],
            &SetsidConfig {
                wait: true,
                ..Default::default()
            },
        );
        assert!(result.is_ok());
        if let Ok(SetsidOutcome::Completed {
            child_pid,
            exit_status,
        }) = result
        {
            assert!(child_pid > 0);
            assert_eq!(exit_status.code(), Some(42));
        }
    }

    #[test]
    fn setsid_honors_cwd_and_env() {
        let dir = unique_temp_dir("setsid-cwd-env");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("marker"), "ok").unwrap();

        let mut env = std::collections::BTreeMap::new();
        env.insert("SYSPRIMS_SESSION_TEST".to_string(), "expected".to_string());

        let result = run_setsid_impl(
            "sh",
            &[
                "-c",
                "test -f marker && test \"$SYSPRIMS_SESSION_TEST\" = expected",
            ],
            &SetsidConfig {
                wait: true,
                cwd: Some(dir.clone()),
                env: Some(env),
                ..Default::default()
            },
        );

        let _ = std::fs::remove_dir_all(&dir);
        assert!(result.is_ok());
        if let Ok(SetsidOutcome::Completed { exit_status, .. }) = result {
            assert_eq!(exit_status.code(), Some(0));
        }
    }

    #[test]
    fn setsid_not_found_command() {
        let result = run_setsid_impl("nonexistent_command_xyz", &[], &SetsidConfig::default());
        assert!(matches!(result, Err(SysprimsError::NotFoundCommand { .. })));
    }

    #[test]
    fn getpgid_current_process() {
        let pgid = getpgid_impl(0);
        assert!(pgid.is_ok());
        assert!(pgid.unwrap() > 0);
    }

    #[test]
    fn getsid_current_process() {
        let sid = getsid_impl(0);
        assert!(sid.is_ok());
        assert!(sid.unwrap() > 0);
    }

    #[test]
    fn nohup_honors_cwd_and_env() {
        let dir = unique_temp_dir("nohup-cwd-env");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("marker"), "ok").unwrap();
        let output_file = dir.join("nohup.out");

        let mut env = std::collections::BTreeMap::new();
        env.insert("SYSPRIMS_SESSION_TEST".to_string(), "expected".to_string());

        let result = run_nohup_impl(
            "sh",
            &[
                "-c",
                "test -f marker && test \"$SYSPRIMS_SESSION_TEST\" = expected",
            ],
            &NohupConfig {
                wait: true,
                cwd: Some(dir.clone()),
                output_file: Some(output_file.display().to_string()),
                env: Some(env),
            },
        );

        let _ = std::fs::remove_dir_all(&dir);
        assert!(result.is_ok());
        if let Ok(NohupOutcome::Completed { exit_status, .. }) = result {
            assert_eq!(exit_status.code(), Some(0));
        }
    }

    #[test]
    fn nohup_rejects_symlink_output_file() {
        let dir = unique_temp_dir("nohup-symlink");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target.log");
        let link = dir.join("link.log");
        std::fs::write(&target, "existing").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let result = run_nohup_impl(
            "sh",
            &["-c", "exit 0"],
            &NohupConfig {
                wait: true,
                output_file: Some(link.display().to_string()),
                ..Default::default()
            },
        );

        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(
            result,
            Err(SysprimsError::PermissionDeniedPath { .. })
        ));
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sysprims-{label}-{}-{nanos}", std::process::id()))
    }
}
