//! Unix implementation of session management.
//!
//! Implementation derived from POSIX specifications:
//! - setsid(2): https://pubs.opengroup.org/onlinepubs/9699919799/functions/setsid.html
//! - nohup: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/nohup.html

use std::os::unix::process::CommandExt;
use std::process::Command;
use std::{
    fs::{File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd},
        unix::fs::OpenOptionsExt,
    },
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        Arc,
    },
};

use sysprims_core::{SysprimsError, SysprimsResult};

use crate::{NohupConfig, NohupOutcome, SetsidConfig, SetsidOutcome};

const SESSION_ACK_MAGIC: [u8; 4] = *b"SYSA";
const SESSION_ACK_VERSION: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
struct SessionAcknowledgement {
    magic: [u8; 4],
    version: u32,
    child_pid: u32,
    session_id: u32,
}

const SESSION_ACK_LEN: usize = std::mem::size_of::<SessionAcknowledgement>();

struct SessionHookState {
    writer_fd: AtomicI32,
    token_reader_fd: AtomicI32,
    invoked: AtomicBool,
}

impl SessionHookState {
    fn close_parent_descriptors(&self) {
        let writer_fd = self.writer_fd.swap(-1, Ordering::AcqRel);
        if writer_fd >= 0 {
            // SAFETY: the atomic swap gives this call sole ownership of the raw
            // writer descriptor in this process.
            unsafe {
                libc::close(writer_fd);
            }
        }
        let token_reader_fd = self.token_reader_fd.swap(-1, Ordering::AcqRel);
        if token_reader_fd >= 0 {
            // SAFETY: the atomic swap gives this call sole ownership of the
            // raw token-reader descriptor in this process.
            unsafe {
                libc::close(token_reader_fd);
            }
        }
    }
}

impl Drop for SessionHookState {
    fn drop(&mut self) {
        self.close_parent_descriptors();
    }
}

/// A non-cloneable Unix child hook that acquires a new session exactly once.
///
/// The hook is prepared before fork and performs only async-signal-safe
/// operations when invoked: atomic state transition, `setsid(2)`, `getpid(2)`,
/// and one fixed-size nonblocking `write(2)`.
pub struct UnixSessionAcquisitionHook {
    state: Arc<SessionHookState>,
}

impl std::fmt::Debug for UnixSessionAcquisitionHook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UnixSessionAcquisitionHook")
            .finish_non_exhaustive()
    }
}

impl UnixSessionAcquisitionHook {
    /// Acquire a dedicated session and process group in a post-fork,
    /// pre-exec child.
    ///
    /// # Safety
    ///
    /// This changes the calling process's session. Call it only from the child
    /// side of a prepared spawn, such as a `CommandExt::pre_exec` callback. The
    /// hook must replace any other session/group acquirer for that spawn.
    pub unsafe fn acquire(&self) -> std::io::Result<()> {
        if self
            .state
            .invoked
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(std::io::Error::from_raw_os_error(libc::EALREADY));
        }

        let token_reader_fd = self.state.token_reader_fd.load(Ordering::Acquire);
        if token_reader_fd < 0 {
            return Err(std::io::Error::from_raw_os_error(libc::EBADF));
        }
        let mut token = 0_u8;
        // SAFETY: the prepared nonblocking token pipe contains exactly one
        // byte shared across every fork of this hook. Consuming it makes this
        // hook single-spawn even though `Command` itself can be reused.
        let token_read =
            unsafe { libc::read(token_reader_fd, std::ptr::addr_of_mut!(token).cast(), 1) };
        if token_read != 1 {
            return Err(if token_read < 0 {
                std::io::Error::last_os_error()
            } else {
                std::io::Error::from_raw_os_error(libc::EALREADY)
            });
        }

        // SAFETY: the caller guarantees this runs in the post-fork/pre-exec
        // child, where `setsid` is the single configured session acquirer.
        let session_id = unsafe { libc::setsid() };
        if session_id < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `getpid` has no preconditions and is async-signal-safe.
        let child_pid = unsafe { libc::getpid() };
        if child_pid <= 0 || session_id != child_pid {
            return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
        }

        let writer_fd = self.state.writer_fd.load(Ordering::Acquire);
        if writer_fd < 0 {
            return Err(std::io::Error::from_raw_os_error(libc::EBADF));
        }

        let acknowledgement = SessionAcknowledgement {
            magic: SESSION_ACK_MAGIC,
            version: SESSION_ACK_VERSION,
            child_pid: child_pid as u32,
            session_id: session_id as u32,
        };

        // SAFETY: `writer_fd` is the pre-created nonblocking pipe writer. The
        // packet is a fixed-size stack value and remains valid for this call.
        let written = unsafe {
            libc::write(
                writer_fd,
                std::ptr::addr_of!(acknowledgement).cast(),
                SESSION_ACK_LEN,
            )
        };
        if written < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if written as usize != SESSION_ACK_LEN {
            return Err(std::io::Error::from_raw_os_error(libc::EIO));
        }
        Ok(())
    }
}

/// Parent-side capability awaiting a positive child-hook acknowledgement.
///
/// This value is single-use and cannot be cloned. Converting it into a receipt
/// closes the parent's writer copy and requires one complete, valid
/// acknowledgement carrying the spawned child PID.
pub struct PendingUnixSessionReceipt {
    reader: OwnedFd,
    state: Arc<SessionHookState>,
}

impl std::fmt::Debug for PendingUnixSessionReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingUnixSessionReceipt")
            .finish_non_exhaustive()
    }
}

impl PendingUnixSessionReceipt {
    /// Consume the pending capability and validate its acknowledgement against
    /// the PID returned by the prepared spawn.
    ///
    /// This parses the private acknowledgement; the external child's
    /// exclusive ownership contract is established later by
    /// `sysprims_timeout::contain_acquired_session`.
    pub fn into_receipt(self, spawned_child_pid: u32) -> SysprimsResult<UnixSessionReceipt> {
        self.state.close_parent_descriptors();
        if spawned_child_pid == 0 || spawned_child_pid > i32::MAX as u32 {
            return Err(SysprimsError::invalid_argument(
                "owned child pid is outside the safe Unix process range",
            ));
        }

        let mut acknowledgement = [0_u8; SESSION_ACK_LEN * 2];
        // SAFETY: `reader` owns a valid nonblocking descriptor and the stack
        // buffer is writable for its full declared length.
        let read = unsafe {
            libc::read(
                self.reader.as_raw_fd(),
                acknowledgement.as_mut_ptr().cast(),
                acknowledgement.len(),
            )
        };
        if read < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EAGAIN)
                || error.raw_os_error() == Some(libc::EWOULDBLOCK)
            {
                return Err(SysprimsError::invalid_argument(
                    "session acquisition acknowledgement is missing or malformed",
                ));
            }
            return Err(SysprimsError::system(
                format!("session acquisition acknowledgement read failed: {error}"),
                error.raw_os_error().unwrap_or(0),
            ));
        }
        if read as usize != SESSION_ACK_LEN {
            return Err(SysprimsError::invalid_argument(
                "session acquisition acknowledgement is missing or malformed",
            ));
        }

        // SAFETY: the read length was validated above. `read_unaligned` avoids
        // assuming the byte buffer has the acknowledgement type's alignment.
        let packet = unsafe {
            std::ptr::read_unaligned(acknowledgement.as_ptr().cast::<SessionAcknowledgement>())
        };
        if packet.magic != SESSION_ACK_MAGIC || packet.version != SESSION_ACK_VERSION {
            return Err(SysprimsError::invalid_argument(
                "session acquisition acknowledgement is missing or malformed",
            ));
        }
        if packet.child_pid != spawned_child_pid || packet.session_id != spawned_child_pid {
            return Err(SysprimsError::invalid_argument(
                "session acquisition acknowledgement does not match the owned child",
            ));
        }

        Ok(UnixSessionReceipt {
            child_pid: packet.child_pid,
            process_group_id: packet.session_id,
            session_id: packet.session_id,
        })
    }
}

impl Drop for PendingUnixSessionReceipt {
    fn drop(&mut self) {
        self.state.close_parent_descriptors();
    }
}

/// Opaque, single-use proof that the prepared child acquired a new session.
///
/// The receipt is minted only from the private acknowledgement channel. It
/// cannot be constructed from a PID, boolean, enum, or post-spawn observation.
#[derive(Debug)]
pub struct UnixSessionReceipt {
    child_pid: u32,
    process_group_id: u32,
    session_id: u32,
}

impl UnixSessionReceipt {
    pub fn child_pid(&self) -> u32 {
        self.child_pid
    }

    pub fn process_group_id(&self) -> u32 {
        self.process_group_id
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub fn session_kind(&self) -> &'static str {
        "new_session"
    }

    pub fn identifier_provenance(&self) -> &'static str {
        "setsid_structural_child_pid"
    }
}

/// Prepare a session-acquisition hook and its parent-side receipt capability.
///
/// All allocation and descriptor setup happens here, before fork. The returned
/// hook must be installed in place of any other `setsid`/`setpgid` acquirer.
pub fn prepare_session_acquisition(
) -> SysprimsResult<(UnixSessionAcquisitionHook, PendingUnixSessionReceipt)> {
    let (reader, writer) = create_acknowledgement_pipe()?;
    let (token_reader, token_writer) = match create_acknowledgement_pipe() {
        Ok(pipe) => pipe,
        Err(error) => {
            // SAFETY: `writer` is still uniquely owned here.
            unsafe {
                libc::close(writer);
            }
            return Err(error);
        }
    };
    let token = 0xa5_u8;
    // SAFETY: the token writer is valid and the one-byte stack value remains
    // live for this fixed nonblocking write.
    let token_written = unsafe { libc::write(token_writer, std::ptr::addr_of!(token).cast(), 1) };
    let token_write_error = (token_written != 1).then(std::io::Error::last_os_error);
    // SAFETY: the token writer is no longer needed after preloading.
    unsafe {
        libc::close(token_writer);
    }
    if let Some(error) = token_write_error {
        // SAFETY: `writer` is still uniquely owned here.
        unsafe {
            libc::close(writer);
        }
        return Err(SysprimsError::system(
            format!("cannot preload session acquisition token: {error}"),
            error.raw_os_error().unwrap_or(libc::EIO),
        ));
    }
    let state = Arc::new(SessionHookState {
        writer_fd: AtomicI32::new(writer),
        token_reader_fd: AtomicI32::new(token_reader.into_raw_fd()),
        invoked: AtomicBool::new(false),
    });
    Ok((
        UnixSessionAcquisitionHook {
            state: Arc::clone(&state),
        },
        PendingUnixSessionReceipt { reader, state },
    ))
}

fn create_acknowledgement_pipe() -> SysprimsResult<(OwnedFd, RawFd)> {
    let mut descriptors = [-1; 2];
    // SAFETY: `descriptors` has space for both descriptors. Linux and Android
    // set close-on-exec and nonblocking atomically; platforms without pipe2
    // use the portable fcntl fallback below.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let pipe_result =
        unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let pipe_result = unsafe { libc::pipe(descriptors.as_mut_ptr()) };
    if pipe_result != 0 {
        return Err(last_system_error(
            "cannot create session acknowledgement pipe",
        ));
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    for descriptor in descriptors {
        // SAFETY: both descriptors came from the successful pipe call and
        // remain open throughout configuration.
        let descriptor_flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
        let status_flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
        if descriptor_flags < 0
            || status_flags < 0
            || unsafe {
                libc::fcntl(
                    descriptor,
                    libc::F_SETFD,
                    descriptor_flags | libc::FD_CLOEXEC,
                )
            } < 0
            || unsafe { libc::fcntl(descriptor, libc::F_SETFL, status_flags | libc::O_NONBLOCK) }
                < 0
        {
            let error = last_system_error("cannot configure session acknowledgement pipe");
            // SAFETY: pipe creation succeeded and ownership has not yet moved
            // into `OwnedFd`/`SessionHookState`.
            unsafe {
                libc::close(descriptors[0]);
                libc::close(descriptors[1]);
            }
            return Err(error);
        }
    }

    // SAFETY: ownership of the configured reader moves into `OwnedFd`; the
    // writer remains uniquely owned by `SessionHookState`.
    Ok((
        unsafe { OwnedFd::from_raw_fd(descriptors[0]) },
        descriptors[1],
    ))
}

fn last_system_error(context: &str) -> SysprimsError {
    let error = std::io::Error::last_os_error();
    SysprimsError::system(
        format!("{context}: {error}"),
        error.raw_os_error().unwrap_or(0),
    )
}

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
    fn prepared_hook_mints_same_spawn_receipt() {
        let (hook, pending) = prepare_session_acquisition().unwrap();
        let mut command = Command::new("sleep");
        command.arg("60");
        // SAFETY: this test installs the prepared hook as the sole child
        // session acquirer.
        unsafe {
            command.pre_exec(move || hook.acquire());
        }

        let mut child = command.spawn().unwrap();
        let receipt = pending.into_receipt(child.id()).unwrap();
        assert_eq!(receipt.child_pid(), child.id());
        assert_eq!(receipt.process_group_id(), child.id());
        assert_eq!(receipt.session_id(), child.id());
        assert_eq!(receipt.session_kind(), "new_session");
        assert_eq!(
            receipt.identifier_provenance(),
            "setsid_structural_child_pid"
        );

        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn receipt_requires_hook_invocation() {
        let (_hook, pending) = prepare_session_acquisition().unwrap();
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();

        let error = pending.into_receipt(child.id()).unwrap_err();
        assert!(matches!(
            error,
            SysprimsError::InvalidArgument { message }
                if message.contains("acknowledgement is missing or malformed")
        ));

        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn dropping_pending_closes_hook_descriptors() {
        let (hook, pending) = prepare_session_acquisition().unwrap();
        assert!(hook.state.writer_fd.load(Ordering::Acquire) >= 0);
        assert!(hook.state.token_reader_fd.load(Ordering::Acquire) >= 0);

        drop(pending);

        assert_eq!(hook.state.writer_fd.load(Ordering::Acquire), -1);
        assert_eq!(hook.state.token_reader_fd.load(Ordering::Acquire), -1);
    }

    #[test]
    fn hook_rejects_second_invocation_before_spawn_can_exec() {
        let (hook, _pending) = prepare_session_acquisition().unwrap();
        let mut command = Command::new("true");
        // SAFETY: both calls run in the same pre-exec child to verify the
        // hook's one-shot failure path.
        unsafe {
            command.pre_exec(move || {
                hook.acquire()?;
                hook.acquire()
            });
        }

        assert!(command.spawn().is_err());
    }

    #[test]
    fn prepared_hook_is_single_spawn_even_when_command_is_reused() {
        let (hook, pending) = prepare_session_acquisition().unwrap();
        let mut command = Command::new("sleep");
        command.arg("60");
        // SAFETY: the prepared hook is the command's sole session acquirer.
        unsafe {
            command.pre_exec(move || hook.acquire());
        }

        let mut first_child = command.spawn().unwrap();
        assert!(
            command.spawn().is_err(),
            "shared token must reject a second fork"
        );
        let receipt = pending.into_receipt(first_child.id()).unwrap();
        assert_eq!(receipt.child_pid(), first_child.id());
        first_child.kill().unwrap();
        first_child.wait().unwrap();
    }

    #[test]
    fn second_session_acquirer_fails_closed_in_either_order() {
        let (hook_first, _pending_first) = prepare_session_acquisition().unwrap();
        let mut after_hook = Command::new("true");
        // SAFETY: the child intentionally attempts two session acquisitions;
        // the second must fail before exec.
        unsafe {
            after_hook.pre_exec(move || {
                hook_first.acquire()?;
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        assert!(after_hook.spawn().is_err());

        let (hook_second, _pending_second) = prepare_session_acquisition().unwrap();
        let mut before_hook = Command::new("true");
        // SAFETY: the child intentionally attempts the reverse acquisition
        // order; the sysprims hook must fail after the first setsid.
        unsafe {
            before_hook.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                hook_second.acquire()
            });
        }
        assert!(before_hook.spawn().is_err());
    }

    #[test]
    fn later_pre_exec_failure_does_not_produce_a_child() {
        let (hook, _pending) = prepare_session_acquisition().unwrap();
        let mut command = Command::new("true");
        // SAFETY: the prepared hook is the sole acquirer; the later injected
        // error verifies that no failed spawn can mint a receipt.
        unsafe {
            command.pre_exec(move || {
                hook.acquire()?;
                Err(std::io::Error::from_raw_os_error(libc::EACCES))
            });
        }

        assert!(command.spawn().is_err());
    }

    #[test]
    fn malformed_acknowledgement_cannot_mint_receipt() {
        let (hook, pending) = prepare_session_acquisition().unwrap();
        let malformed = [0x5a_u8; 3];
        // SAFETY: the test owns the valid pipe writer and the fixed stack
        // buffer remains live for the write.
        let written = unsafe {
            libc::write(
                hook.state.writer_fd.load(Ordering::Acquire),
                malformed.as_ptr().cast(),
                malformed.len(),
            )
        };
        assert_eq!(written, malformed.len() as isize);

        let error = pending.into_receipt(std::process::id()).unwrap_err();
        assert!(matches!(
            error,
            SysprimsError::InvalidArgument { message }
                if message.contains("acknowledgement is missing or malformed")
        ));
    }

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
