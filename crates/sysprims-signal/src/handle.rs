// SPDX-License-Identifier: MIT OR Apache-2.0

//! Process-bound signal and exit-watch handle (`ProcessHandle`).
//!
//! `ProcessHandle` brackets handle acquisition with native process birth/start-time
//! verification (`birth_before -> open -> birth_after == birth_before`) so the
//! resulting handle is tied to the verified process incarnation rather than an
//! unverified numeric PID.
//!
//! ## Platform Binding Matrix
//!
//! | Platform | Exit Observation | Signal Delivery | [`SignalBinding`] |
//! |---|---|---|---|
//! | Linux >= 5.3 (`pidfd`) | `poll(pidfd, POLLIN, 0)` (whole-process exit) | `pidfd_send_signal(pidfd, sig, NULL, 0)` | [`SignalBinding::Bound`] |
//! | Linux < 5.3 (`ENOSYS` fallback) | `/proc/<pid>/stat` birth + state check | birth/exit re-check then `kill(pid, sig)` | [`SignalBinding::CheckedNumeric`] |
//! | macOS | `kqueue` `EVFILT_PROC` `NOTE_EXIT` (`EV_ONESHOT`) + `kinfo_proc` birth check | exit + `kinfo_proc` birth re-check then `kill(pid, sig)` | [`SignalBinding::CheckedNumeric`] |
//! | Windows | `WaitForSingleObject(handle, 0)` | `TerminateProcess(handle, 1)` on bound `HANDLE` | [`SignalBinding::Bound`] |
//!
//! ## macOS `CheckedNumeric` Residual
//!
//! Public XNU exposes no kernel-bound signal primitive equivalent to Linux's
//! `pidfd_send_signal`. On macOS, `ProcessHandle::signal` checks the latched
//! `kqueue` `EVFILT_PROC`/`NOTE_EXIT` watch and re-verifies `kinfo_proc`
//! `p_starttime` immediately before calling `kill(pid, sig)`. A narrow
//! exit -> parent reap -> PID reassignment window remains between that check
//! and `kill(pid, sig)` (launchd reaps orphans immediately). Callers that
//! require kernel-bound guarantees for unattended force-escalation can inspect
//! [`ProcessHandle::binding`] / the [`SignalBinding`] returned by
//! [`ProcessHandle::signal`].

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use sysprims_core::{SysprimsError, SysprimsResult};

use crate::{resolve_signal_name, validate_pid, FORCE_KILL_SIGNAL, TERMINATE_SIGNAL};

/// How tightly signal delivery on a [`ProcessHandle`] is bound to the target
/// process incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignalBinding {
    /// Kernel-bound: signal delivery uses an OS descriptor/handle (`pidfd` on
    /// Linux >= 5.3, `HANDLE` on Windows) and cannot reach a later holder of
    /// the numeric PID.
    Bound,
    /// Exit watch and process start-identity are verified immediately before a
    /// numeric `kill(pid, sig)` call; the narrow exit-reap-reuse window remains
    /// (macOS; Linux < 5.3 `ENOSYS` fallback).
    CheckedNumeric,
}

impl SignalBinding {
    /// Canonical `snake_case` wire representation (`"bound"` or `"checked_numeric"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bound => "bound",
            Self::CheckedNumeric => "checked_numeric",
        }
    }
}

impl fmt::Display for SignalBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(target_os = "linux")]
static FORCE_LINUX_CHECKED_NUMERIC: AtomicBool = AtomicBool::new(false);

/// Test-only helper on Linux to exercise the `ENOSYS` (`CheckedNumeric`) fallback
/// path on kernels that support `pidfd_open`.
#[doc(hidden)]
pub fn with_forced_checked_numeric_for_test<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    #[cfg(target_os = "linux")]
    {
        struct ResetGuard;
        impl Drop for ResetGuard {
            fn drop(&mut self) {
                FORCE_LINUX_CHECKED_NUMERIC.store(false, Ordering::SeqCst);
            }
        }
        FORCE_LINUX_CHECKED_NUMERIC.store(true, Ordering::SeqCst);
        let _guard = ResetGuard;
        f()
    }
    #[cfg(not(target_os = "linux"))]
    {
        f()
    }
}

/// A signal and exit-watch handle bound to a verified process incarnation.
///
/// Opening a `ProcessHandle` reads the target process's native start-identity,
/// acquires the OS exit/process handle (`pidfd` on Linux, `kqueue` `EVFILT_PROC`
/// `NOTE_EXIT` on macOS, `HANDLE` on Windows) with `FD_CLOEXEC` / non-inheritable
/// flags, and re-reads the native start-identity to prove the PID was neither
/// exited nor reused during registration.
///
/// Raw OS descriptors/handles are never exposed and are closed automatically on
/// [`Drop`] or [`ProcessHandle::close`].
pub struct ProcessHandle {
    pid: u32,
    start_identity: String,
    binding: SignalBinding,
    exited: AtomicBool,
    closed: AtomicBool,
    inner: imp::PlatformProcessHandle,
}

impl fmt::Debug for ProcessHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessHandle")
            .field("pid", &self.pid)
            .field("start_identity", &self.start_identity)
            .field("binding", &self.binding)
            .field("exited", &self.exited.load(Ordering::Relaxed))
            .field("closed", &self.closed.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl ProcessHandle {
    /// Open a [`ProcessHandle`] for `pid`, bracketing registration with native
    /// start-identity checks before and after opening the OS handle.
    ///
    /// # Errors
    ///
    /// - [`SysprimsError::InvalidArgument`] if `pid == 0` or `pid > MAX_SAFE_PID`
    /// - [`SysprimsError::NotFound`] if the process does not exist, is already
    ///   exited/zombie, or changed incarnation during registration
    /// - [`SysprimsError::PermissionDenied`] if the caller is not permitted to
    ///   signal/inspect `pid`
    pub fn open(pid: u32) -> SysprimsResult<Self> {
        Self::open_inner(pid, None)
    }

    /// Open a [`ProcessHandle`] for `pid` and require its bracketed native
    /// start-identity to equal `expected_start_identity`.
    ///
    /// If `pid` is currently held by a different process incarnation (different
    /// start-identity), returns [`SysprimsError::NotFound`].
    pub fn open_verified(pid: u32, expected_start_identity: &str) -> SysprimsResult<Self> {
        let trimmed = expected_start_identity.trim();
        if trimmed.is_empty() {
            return Err(SysprimsError::invalid_argument(
                "expected_start_identity must not be empty",
            ));
        }
        Self::open_inner(pid, Some(trimmed))
    }

    fn open_inner(pid: u32, expected_start_identity: Option<&str>) -> SysprimsResult<Self> {
        validate_pid(pid)?;
        let (start_identity, binding, inner) =
            imp::open_platform_handle(pid, expected_start_identity)?;
        Ok(Self {
            pid,
            start_identity,
            binding,
            exited: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            inner,
        })
    }

    /// Numeric PID (`1..=MAX_SAFE_PID`) verified at handle creation.
    pub const fn pid(&self) -> u32 {
        self.pid
    }

    /// Canonical native start-identity string captured at handle creation
    /// (`"<tv_sec>.<tv_usec:06>"` on macOS, `"<starttime_ticks>"` on Linux,
    /// `"<creation_filetime_100ns>"` on Windows).
    pub fn start_identity(&self) -> &str {
        &self.start_identity
    }

    /// Binding strength for signals sent through this handle.
    pub const fn binding(&self) -> SignalBinding {
        self.binding
    }

    /// Whether [`ProcessHandle::close`] has been called on this handle.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn ensure_open(&self) -> SysprimsResult<()> {
        if self.is_closed() {
            return Err(SysprimsError::invalid_argument("process handle is closed"));
        }
        Ok(())
    }

    /// Check whether whole-process exit has been observed for the bound process.
    ///
    /// This result is latched: once `Ok(true)` is observed, subsequent calls on
    /// the same open handle always return `Ok(true)` even after the process is
    /// reaped or its numeric PID is reused by another process.
    pub fn exited(&self) -> SysprimsResult<bool> {
        self.ensure_open()?;
        if self.exited.load(Ordering::Acquire) {
            return Ok(true);
        }
        let now_exited = self.inner.check_exited(self.pid, &self.start_identity)?;
        if now_exited {
            self.exited.store(true, Ordering::Release);
        }
        Ok(now_exited)
    }

    /// Deliver `sig` to the bound process only, returning the [`SignalBinding`]
    /// strength used for delivery.
    ///
    /// Once exit is observed, no signal is sent and [`SysprimsError::NotFound`]
    /// is returned.
    pub fn signal(&self, sig: i32) -> SysprimsResult<SignalBinding> {
        self.ensure_open()?;
        if self.exited()? {
            return Err(SysprimsError::not_found(self.pid));
        }
        match self.inner.send_signal(self.pid, &self.start_identity, sig) {
            Ok(binding) => Ok(binding),
            Err(SysprimsError::NotFound { pid }) => {
                self.exited.store(true, Ordering::Release);
                Err(SysprimsError::NotFound { pid })
            }
            Err(err) => Err(err),
        }
    }

    /// Resolve `signal_name` (e.g. `"SIGTERM"`, `"TERM"`, `"kill"`) and deliver
    /// it to the bound process.
    pub fn signal_by_name(&self, signal_name: &str) -> SysprimsResult<SignalBinding> {
        let sig = resolve_signal_name(signal_name)?;
        self.signal(sig)
    }

    /// Convenience wrapper: deliver `SIGTERM` to the bound process.
    pub fn terminate(&self) -> SysprimsResult<SignalBinding> {
        self.signal(TERMINATE_SIGNAL)
    }

    /// Convenience wrapper: deliver `SIGKILL` to the bound process.
    pub fn force_kill(&self) -> SysprimsResult<SignalBinding> {
        self.signal(FORCE_KILL_SIGNAL)
    }

    /// Close the underlying OS resource early (idempotent).
    pub fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.inner.close_handle();
        }
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        self.close();
    }
}

/// Open a [`ProcessHandle`] bound to the current live process incarnation at `pid`.
pub fn open_process_handle(pid: u32) -> SysprimsResult<ProcessHandle> {
    ProcessHandle::open(pid)
}

/// Open a [`ProcessHandle`] bound to `pid`, requiring its native start-identity
/// to match `expected_start_identity`.
pub fn open_process_handle_verified(
    pid: u32,
    expected_start_identity: &str,
) -> SysprimsResult<ProcessHandle> {
    ProcessHandle::open_verified(pid, expected_start_identity)
}

// ============================================================================
// Platform Implementations
// ============================================================================

#[cfg(target_os = "macos")]
mod imp {
    use std::io;
    use std::ptr;
    use std::sync::atomic::{AtomicI32, Ordering};

    use libc::{c_int, c_void};
    use sysprims_core::{SysprimsError, SysprimsResult};

    use super::SignalBinding;

    const KINFO_PROC_SIZE_64: usize = 648;
    const OFF_P_STARTTIME_SEC: usize = 0;
    const OFF_P_STARTTIME_USEC: usize = 8;
    const OFF_P_STAT: usize = 36;
    const OFF_P_PID: usize = 40;

    const SIDL: u8 = 1;
    const SRUN: u8 = 2;
    const SSLEEP: u8 = 3;
    const SSTOP: u8 = 4;
    const SZOMB: u8 = 5;

    pub(super) struct PlatformProcessHandle {
        kq_fd: AtomicI32,
    }

    fn close_raw_fd(fd: c_int) {
        if fd >= 0 {
            unsafe {
                let _ = libc::close(fd);
            }
        }
    }

    fn last_errno() -> i32 {
        io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EIO)
    }

    /// Read `(start_identity, is_zombie)` from `sysctl(KERN_PROC_PID)`.
    fn read_kinfo_start_identity(pid: u32, op: &str) -> SysprimsResult<(String, bool)> {
        let mut mib: [c_int; 4] = [
            libc::CTL_KERN,
            libc::KERN_PROC,
            libc::KERN_PROC_PID,
            pid as c_int,
        ];
        let mut buf = [0u8; KINFO_PROC_SIZE_64];
        let mut size = buf.len();
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                4,
                buf.as_mut_ptr() as *mut c_void,
                &mut size,
                ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            let errno = last_errno();
            return match errno {
                libc::ESRCH => Err(SysprimsError::not_found(pid)),
                libc::EPERM | libc::EACCES => Err(SysprimsError::permission_denied(pid, op)),
                _ => Err(SysprimsError::system("sysctl(KERN_PROC_PID) failed", errno)),
            };
        }
        if size == 0 {
            return Err(SysprimsError::not_found(pid));
        }
        if size != KINFO_PROC_SIZE_64 {
            return Err(SysprimsError::system(
                "unexpected kinfo_proc size",
                libc::EINVAL,
            ));
        }

        let tv_sec = i64::from_ne_bytes(
            buf[OFF_P_STARTTIME_SEC..OFF_P_STARTTIME_SEC + 8]
                .try_into()
                .unwrap(),
        );
        let tv_usec = i32::from_ne_bytes(
            buf[OFF_P_STARTTIME_USEC..OFF_P_STARTTIME_USEC + 4]
                .try_into()
                .unwrap(),
        );
        let p_stat = buf[OFF_P_STAT];
        let p_pid = i32::from_ne_bytes(buf[OFF_P_PID..OFF_P_PID + 4].try_into().unwrap());

        if p_pid != (pid as i32) {
            return Err(SysprimsError::not_found(pid));
        }
        if tv_sec < 0 || !(0..=999_999).contains(&tv_usec) {
            return Err(SysprimsError::system(
                "invalid kinfo_proc p_starttime",
                libc::EINVAL,
            ));
        }

        let is_zombie = match p_stat {
            SZOMB => true,
            SRUN | SSLEEP | SSTOP => false,
            SIDL => return Err(SysprimsError::not_found(pid)),
            _ => {
                return Err(SysprimsError::system(
                    "unexpected kinfo_proc p_stat",
                    libc::EINVAL,
                ))
            }
        };

        Ok((format!("{tv_sec}.{tv_usec:06}"), is_zombie))
    }

    pub(super) fn open_platform_handle(
        pid: u32,
        expected_start_identity: Option<&str>,
    ) -> SysprimsResult<(String, SignalBinding, PlatformProcessHandle)> {
        let (birth0, zombie0) = read_kinfo_start_identity(pid, "open_process_handle")?;
        if zombie0 {
            return Err(SysprimsError::not_found(pid));
        }
        if let Some(expected) = expected_start_identity {
            if birth0 != expected {
                return Err(SysprimsError::not_found(pid));
            }
        }

        // Enforce the same UID/signal permission rule as kill(pid, 0) without
        // sending a signal.
        let kill0 = unsafe { libc::kill(pid as c_int, 0) };
        if kill0 != 0 {
            let errno = last_errno();
            return match errno {
                libc::ESRCH => Err(SysprimsError::not_found(pid)),
                libc::EPERM | libc::EACCES => {
                    Err(SysprimsError::permission_denied(pid, "open_process_handle"))
                }
                _ => Err(SysprimsError::system("kill(pid, 0) failed", errno)),
            };
        }

        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            let errno = last_errno();
            return Err(SysprimsError::system("kqueue() failed", errno));
        }
        if unsafe { libc::fcntl(kq, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            let errno = last_errno();
            close_raw_fd(kq);
            return Err(SysprimsError::system("fcntl(FD_CLOEXEC) failed", errno));
        }

        let change = libc::kevent {
            ident: pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: ptr::null_mut(),
        };
        let mut out_ev: libc::kevent = unsafe { std::mem::zeroed() };
        let zero_ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let n = loop {
            let rc = unsafe { libc::kevent(kq, &change, 1, &mut out_ev, 1, &zero_ts) };
            if rc >= 0 {
                break rc;
            }
            let errno = last_errno();
            if errno == libc::EINTR {
                continue;
            }
            close_raw_fd(kq);
            return match errno {
                libc::ESRCH => Err(SysprimsError::not_found(pid)),
                libc::EPERM | libc::EACCES => {
                    Err(SysprimsError::permission_denied(pid, "open_process_handle"))
                }
                _ => Err(SysprimsError::system("kevent registration failed", errno)),
            };
        };

        if n > 0 {
            if (out_ev.flags & libc::EV_ERROR) != 0 {
                let err_code = out_ev.data as i32;
                close_raw_fd(kq);
                return match err_code {
                    libc::ESRCH => Err(SysprimsError::not_found(pid)),
                    libc::EPERM | libc::EACCES => {
                        Err(SysprimsError::permission_denied(pid, "open_process_handle"))
                    }
                    _ => Err(SysprimsError::system(
                        "kevent registration EV_ERROR",
                        err_code,
                    )),
                };
            }
            if (out_ev.fflags & libc::NOTE_EXIT) != 0 {
                close_raw_fd(kq);
                return Err(SysprimsError::not_found(pid));
            }
        }

        // Post-registration birth check: verify the PID still belongs to the
        // exact same live incarnation after kqueue registration.
        let (birth1, zombie1) = match read_kinfo_start_identity(pid, "open_process_handle") {
            Ok(v) => v,
            Err(err) => {
                close_raw_fd(kq);
                return Err(err);
            }
        };
        if zombie1 || birth1 != birth0 {
            close_raw_fd(kq);
            return Err(SysprimsError::not_found(pid));
        }

        Ok((
            birth0,
            SignalBinding::CheckedNumeric,
            PlatformProcessHandle {
                kq_fd: AtomicI32::new(kq),
            },
        ))
    }

    impl PlatformProcessHandle {
        pub(super) fn close_handle(&self) {
            let fd = self.kq_fd.swap(-1, Ordering::AcqRel);
            close_raw_fd(fd);
        }

        pub(super) fn check_exited(&self, pid: u32, start_identity: &str) -> SysprimsResult<bool> {
            let kq = self.kq_fd.load(Ordering::Acquire);
            if kq < 0 {
                return Err(SysprimsError::invalid_argument("process handle is closed"));
            }

            let zero_ts = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            loop {
                let mut ev: libc::kevent = unsafe { std::mem::zeroed() };
                let rc = unsafe { libc::kevent(kq, ptr::null(), 0, &mut ev, 1, &zero_ts) };
                if rc > 0 {
                    if (ev.flags & libc::EV_ERROR) != 0 {
                        return Err(SysprimsError::system(
                            "kevent poll EV_ERROR",
                            ev.data as i32,
                        ));
                    }
                    if (ev.fflags & libc::NOTE_EXIT) != 0 {
                        return Ok(true);
                    }
                    break;
                }
                if rc == 0 {
                    break;
                }
                let errno = last_errno();
                if errno == libc::EINTR {
                    continue;
                }
                return Err(SysprimsError::system("kevent poll failed", errno));
            }

            // Also check kinfo_proc: if the process has transitioned to SZOMB,
            // been reaped (NotFound), or had its PID reused (birth != start_identity),
            // treat the bound incarnation as exited.
            match read_kinfo_start_identity(pid, "exited") {
                Ok((birth, is_zombie)) => Ok(is_zombie || birth != start_identity),
                Err(SysprimsError::NotFound { .. }) => Ok(true),
                Err(err) => Err(err),
            }
        }

        pub(super) fn send_signal(
            &self,
            pid: u32,
            start_identity: &str,
            sig: i32,
        ) -> SysprimsResult<SignalBinding> {
            // Re-verify birth and live state immediately before numeric kill.
            match read_kinfo_start_identity(pid, "signal") {
                Ok((birth, is_zombie)) => {
                    if is_zombie || birth != start_identity {
                        return Err(SysprimsError::not_found(pid));
                    }
                }
                Err(SysprimsError::NotFound { .. }) => return Err(SysprimsError::not_found(pid)),
                Err(err) => return Err(err),
            }

            crate::unix::kill_impl(pid, sig)?;
            Ok(SignalBinding::CheckedNumeric)
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::fs::OpenOptions;
    use std::io::{self, Read};
    use std::os::unix::fs::OpenOptionsExt;
    use std::ptr;
    use std::sync::atomic::{AtomicI32, Ordering};

    use libc::c_int;
    use sysprims_core::{SysprimsError, SysprimsResult};

    use super::{SignalBinding, FORCE_LINUX_CHECKED_NUMERIC};

    pub(super) struct PlatformProcessHandle {
        /// `>= 0` when backed by a kernel `pidfd` (`SignalBinding::Bound`);
        /// `-1` when using the Linux `< 5.3` `ENOSYS` start-time fallback
        /// (`SignalBinding::CheckedNumeric`).
        pidfd: AtomicI32,
        bound: bool,
    }

    fn close_raw_fd(fd: c_int) {
        if fd >= 0 {
            unsafe {
                let _ = libc::close(fd);
            }
        }
    }

    fn last_errno() -> i32 {
        io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EIO)
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct LinuxStatSnapshot {
        start_ticks: String,
        state: char,
        num_threads: u64,
    }

    impl LinuxStatSnapshot {
        fn is_whole_process_dead(&self) -> bool {
            matches!(self.state, 'X' | 'x') || (self.state == 'Z' && self.num_threads <= 1)
        }
    }

    fn parse_proc_stat_snapshot(stat_text: &str) -> Option<LinuxStatSnapshot> {
        let (_, after_comm) = stat_text.rsplit_once(") ")?;
        let mut fields = after_comm.split_whitespace();
        let state = fields.next()?.chars().next()?;
        // Field 3 is state (index 0 after ") ").
        // Field 20 is num_threads (index 17 after ") ", so 16 steps after state).
        let num_threads_str = fields.nth(16)?;
        let num_threads = num_threads_str.parse::<u64>().ok()?;
        // Field 22 is starttime (index 19 after ") ", so 1 step after num_threads).
        let start_ticks_str = fields.nth(1)?;
        let _ticks = start_ticks_str.parse::<u64>().ok()?;
        Some(LinuxStatSnapshot {
            start_ticks: start_ticks_str.to_string(),
            state,
            num_threads,
        })
    }

    fn read_linux_stat(pid: u32, op: &str) -> SysprimsResult<LinuxStatSnapshot> {
        let path = format!("/proc/{pid}/stat");
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|err| match err.raw_os_error() {
                Some(libc::ENOENT | libc::ESRCH) => SysprimsError::not_found(pid),
                Some(libc::EPERM | libc::EACCES) => SysprimsError::permission_denied(pid, op),
                Some(errno) => SysprimsError::system("open(/proc/<pid>/stat) failed", errno),
                None => SysprimsError::system("open(/proc/<pid>/stat) failed", libc::EIO),
            })?;

        let mut buf = String::with_capacity(512);
        file.read_to_string(&mut buf)
            .map_err(|err| match err.raw_os_error() {
                Some(libc::ENOENT | libc::ESRCH) => SysprimsError::not_found(pid),
                Some(libc::EPERM | libc::EACCES) => SysprimsError::permission_denied(pid, op),
                Some(errno) => SysprimsError::system("read(/proc/<pid>/stat) failed", errno),
                None => SysprimsError::system("read(/proc/<pid>/stat) failed", libc::EIO),
            })?;

        parse_proc_stat_snapshot(&buf).ok_or_else(|| {
            SysprimsError::system("malformed /proc/<pid>/stat payload", libc::EINVAL)
        })
    }

    fn poll_pidfd_exited(fd: c_int) -> SysprimsResult<bool> {
        loop {
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let rc = unsafe { libc::poll(&mut pfd, 1, 0) };
            if rc > 0 {
                if (pfd.revents & libc::POLLNVAL) != 0 {
                    return Err(SysprimsError::system("poll(pidfd) POLLNVAL", libc::EBADF));
                }
                if (pfd.revents & (libc::POLLIN | libc::POLLHUP)) != 0 {
                    return Ok(true);
                }
                if (pfd.revents & libc::POLLERR) != 0 {
                    return Err(SysprimsError::system("poll(pidfd) POLLERR", libc::EIO));
                }
                return Ok(false);
            }
            if rc == 0 {
                return Ok(false);
            }
            let errno = last_errno();
            if errno == libc::EINTR {
                continue;
            }
            return Err(SysprimsError::system("poll(pidfd) failed", errno));
        }
    }

    pub(super) fn open_platform_handle(
        pid: u32,
        expected_start_identity: Option<&str>,
    ) -> SysprimsResult<(String, SignalBinding, PlatformProcessHandle)> {
        let snap0 = read_linux_stat(pid, "open_process_handle")?;
        if snap0.is_whole_process_dead() {
            return Err(SysprimsError::not_found(pid));
        }
        if let Some(expected) = expected_start_identity {
            if snap0.start_ticks != expected {
                return Err(SysprimsError::not_found(pid));
            }
        }

        // Enforce the same UID/signal permission rule as kill(pid, 0) without
        // sending a signal.
        let kill0 = unsafe { libc::kill(pid as c_int, 0) };
        if kill0 != 0 {
            let errno = last_errno();
            return match errno {
                libc::ESRCH => Err(SysprimsError::not_found(pid)),
                libc::EPERM | libc::EACCES => {
                    Err(SysprimsError::permission_denied(pid, "open_process_handle"))
                }
                _ => Err(SysprimsError::system("kill(pid, 0) failed", errno)),
            };
        }

        let force_fallback = FORCE_LINUX_CHECKED_NUMERIC.load(Ordering::SeqCst);
        let (pidfd, binding) = if force_fallback {
            if snap0.state == 'Z' {
                return Err(SysprimsError::not_found(pid));
            }
            (-1, SignalBinding::CheckedNumeric)
        } else {
            let raw_fd = loop {
                let rc = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as c_int, 0u32) };
                if rc >= 0 {
                    break rc as c_int;
                }
                let errno = last_errno();
                if errno == libc::EINTR {
                    continue;
                }
                if errno == libc::ENOSYS {
                    break -1;
                }
                return match errno {
                    libc::ESRCH => Err(SysprimsError::not_found(pid)),
                    libc::EPERM | libc::EACCES => {
                        Err(SysprimsError::permission_denied(pid, "open_process_handle"))
                    }
                    _ => Err(SysprimsError::system("pidfd_open failed", errno)),
                };
            };

            if raw_fd >= 0 {
                if unsafe { libc::fcntl(raw_fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                    let errno = last_errno();
                    close_raw_fd(raw_fd);
                    return Err(SysprimsError::system("fcntl(FD_CLOEXEC) failed", errno));
                }
                // Whole-process exit check via pidfd: rejects unreaped zombies
                // whose entire thread group has already exited, while allowing
                // a thread group whose leader exited with a live worker thread.
                match poll_pidfd_exited(raw_fd) {
                    Ok(true) => {
                        close_raw_fd(raw_fd);
                        return Err(SysprimsError::not_found(pid));
                    }
                    Ok(false) => {}
                    Err(err) => {
                        close_raw_fd(raw_fd);
                        return Err(err);
                    }
                }
                (raw_fd, SignalBinding::Bound)
            } else {
                if snap0.state == 'Z' {
                    return Err(SysprimsError::not_found(pid));
                }
                (-1, SignalBinding::CheckedNumeric)
            }
        };

        // Post-registration birth check: verify /proc/<pid>/stat still belongs
        // to the exact same start_ticks incarnation.
        let snap1 = match read_linux_stat(pid, "open_process_handle") {
            Ok(s) => s,
            Err(err) => {
                close_raw_fd(pidfd);
                return Err(err);
            }
        };
        if snap1.start_ticks != snap0.start_ticks || snap1.is_whole_process_dead() {
            close_raw_fd(pidfd);
            return Err(SysprimsError::not_found(pid));
        }
        if pidfd >= 0 {
            if let Ok(true) = poll_pidfd_exited(pidfd) {
                close_raw_fd(pidfd);
                return Err(SysprimsError::not_found(pid));
            }
        }

        Ok((
            snap0.start_ticks,
            binding,
            PlatformProcessHandle {
                pidfd: AtomicI32::new(pidfd),
                bound: binding == SignalBinding::Bound,
            },
        ))
    }

    impl PlatformProcessHandle {
        pub(super) fn close_handle(&self) {
            let fd = self.pidfd.swap(-1, Ordering::AcqRel);
            close_raw_fd(fd);
        }

        pub(super) fn check_exited(&self, pid: u32, start_identity: &str) -> SysprimsResult<bool> {
            if self.bound {
                let fd = self.pidfd.load(Ordering::Acquire);
                if fd < 0 {
                    return Err(SysprimsError::invalid_argument("process handle is closed"));
                }
                return poll_pidfd_exited(fd);
            }

            match read_linux_stat(pid, "exited") {
                Ok(snap) => Ok(snap.is_whole_process_dead() || snap.start_ticks != start_identity),
                Err(SysprimsError::NotFound { .. }) => Ok(true),
                Err(err) => Err(err),
            }
        }

        pub(super) fn send_signal(
            &self,
            pid: u32,
            start_identity: &str,
            sig: i32,
        ) -> SysprimsResult<SignalBinding> {
            if self.bound {
                let fd = self.pidfd.load(Ordering::Acquire);
                if fd < 0 {
                    return Err(SysprimsError::invalid_argument("process handle is closed"));
                }
                loop {
                    let rc = unsafe {
                        libc::syscall(
                            libc::SYS_pidfd_send_signal,
                            fd,
                            sig,
                            ptr::null::<libc::siginfo_t>(),
                            0u32,
                        )
                    };
                    if rc == 0 {
                        return Ok(SignalBinding::Bound);
                    }
                    let errno = last_errno();
                    if errno == libc::EINTR {
                        continue;
                    }
                    return match errno {
                        libc::ESRCH => Err(SysprimsError::not_found(pid)),
                        libc::EPERM | libc::EACCES => {
                            Err(SysprimsError::permission_denied(pid, "signal"))
                        }
                        libc::EINVAL => Err(SysprimsError::invalid_argument(format!(
                            "invalid signal: {sig}"
                        ))),
                        _ => Err(SysprimsError::system("pidfd_send_signal failed", errno)),
                    };
                }
            }

            // CheckedNumeric fallback (Linux < 5.3): verify start_ticks + state
            // immediately before numeric kill.
            match read_linux_stat(pid, "signal") {
                Ok(snap) => {
                    if snap.is_whole_process_dead() || snap.start_ticks != start_identity {
                        return Err(SysprimsError::not_found(pid));
                    }
                }
                Err(SysprimsError::NotFound { .. }) => return Err(SysprimsError::not_found(pid)),
                Err(err) => return Err(err),
            }

            crate::unix::kill_impl(pid, sig)?;
            Ok(SignalBinding::CheckedNumeric)
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::sync::atomic::{AtomicIsize, Ordering};

    use sysprims_core::{SysprimsError, SysprimsResult};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, SetHandleInformation, ERROR_ACCESS_DENIED,
        ERROR_INVALID_PARAMETER, HANDLE, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_C_EVENT};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, TerminateProcess, WaitForSingleObject,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    use super::SignalBinding;

    pub(super) struct PlatformProcessHandle {
        handle: AtomicIsize,
    }

    fn close_raw_handle(raw: isize) {
        if raw != 0 {
            unsafe {
                let _ = CloseHandle(raw as HANDLE);
            }
        }
    }

    fn read_handle_creation_time(handle: HANDLE, pid: u32) -> SysprimsResult<String> {
        let mut creation = unsafe { std::mem::zeroed() };
        let mut exit = unsafe { std::mem::zeroed() };
        let mut kernel = unsafe { std::mem::zeroed() };
        let mut user = unsafe { std::mem::zeroed() };
        let ok =
            unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return match err {
                ERROR_ACCESS_DENIED => {
                    Err(SysprimsError::permission_denied(pid, "open_process_handle"))
                }
                ERROR_INVALID_PARAMETER => Err(SysprimsError::not_found(pid)),
                _ => Err(SysprimsError::system("GetProcessTimes failed", err as i32)),
            };
        }
        let ticks = ((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64);
        Ok(ticks.to_string())
    }

    fn read_pid_creation_time(pid: u32, op: &str) -> SysprimsResult<String> {
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if h.is_null() {
            let err = unsafe { GetLastError() };
            return match err {
                ERROR_ACCESS_DENIED => Err(SysprimsError::permission_denied(pid, op)),
                ERROR_INVALID_PARAMETER => Err(SysprimsError::not_found(pid)),
                _ => Err(SysprimsError::system("OpenProcess failed", err as i32)),
            };
        }
        let res = read_handle_creation_time(h, pid);
        unsafe {
            let _ = CloseHandle(h);
        }
        res
    }

    pub(super) fn open_platform_handle(
        pid: u32,
        expected_start_identity: Option<&str>,
    ) -> SysprimsResult<(String, SignalBinding, PlatformProcessHandle)> {
        let birth0 = read_pid_creation_time(pid, "open_process_handle")?;
        if let Some(expected) = expected_start_identity {
            if birth0 != expected {
                return Err(SysprimsError::not_found(pid));
            }
        }

        let access = PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle.is_null() {
            let err = unsafe { GetLastError() };
            return match err {
                ERROR_ACCESS_DENIED => {
                    Err(SysprimsError::permission_denied(pid, "open_process_handle"))
                }
                ERROR_INVALID_PARAMETER => Err(SysprimsError::not_found(pid)),
                _ => Err(SysprimsError::system("OpenProcess failed", err as i32)),
            };
        }
        unsafe {
            let _ = SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
        }

        let handle_birth = match read_handle_creation_time(handle, pid) {
            Ok(b) => b,
            Err(err) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(err);
            }
        };
        let wait_rc = unsafe { WaitForSingleObject(handle, 0) };
        if wait_rc == WAIT_OBJECT_0 || handle_birth != birth0 {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(SysprimsError::not_found(pid));
        }

        // Post-open verification against current numeric PID holder.
        let birth1 = match read_pid_creation_time(pid, "open_process_handle") {
            Ok(b) => b,
            Err(err) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(err);
            }
        };
        if birth1 != birth0 {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(SysprimsError::not_found(pid));
        }

        Ok((
            birth0,
            SignalBinding::Bound,
            PlatformProcessHandle {
                handle: AtomicIsize::new(handle as isize),
            },
        ))
    }

    impl PlatformProcessHandle {
        pub(super) fn close_handle(&self) {
            let raw = self.handle.swap(0, Ordering::AcqRel);
            close_raw_handle(raw);
        }

        pub(super) fn check_exited(
            &self,
            _pid: u32,
            _start_identity: &str,
        ) -> SysprimsResult<bool> {
            let raw = self.handle.load(Ordering::Acquire);
            if raw == 0 {
                return Err(SysprimsError::invalid_argument("process handle is closed"));
            }
            let rc = unsafe { WaitForSingleObject(raw as HANDLE, 0) };
            Ok(rc == WAIT_OBJECT_0)
        }

        pub(super) fn send_signal(
            &self,
            pid: u32,
            start_identity: &str,
            sig: i32,
        ) -> SysprimsResult<SignalBinding> {
            let raw = self.handle.load(Ordering::Acquire);
            if raw == 0 {
                return Err(SysprimsError::invalid_argument("process handle is closed"));
            }
            let handle = raw as HANDLE;
            if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
                return Err(SysprimsError::not_found(pid));
            }

            match sig {
                rsfulmen::foundry::signals::SIGTERM | rsfulmen::foundry::signals::SIGKILL => {
                    let ok = unsafe { TerminateProcess(handle, 1) };
                    if ok != 0 {
                        Ok(SignalBinding::Bound)
                    } else {
                        let err = unsafe { GetLastError() };
                        if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
                            return Err(SysprimsError::not_found(pid));
                        }
                        Err(SysprimsError::system("TerminateProcess failed", err as i32))
                    }
                }
                rsfulmen::foundry::signals::SIGINT => {
                    let current_birth = read_pid_creation_time(pid, "signal")?;
                    if current_birth != start_identity {
                        return Err(SysprimsError::not_found(pid));
                    }
                    let ok = unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, pid) };
                    if ok != 0 {
                        Ok(SignalBinding::CheckedNumeric)
                    } else {
                        let err = unsafe { GetLastError() };
                        match err {
                            ERROR_ACCESS_DENIED => {
                                Err(SysprimsError::permission_denied(pid, "signal"))
                            }
                            ERROR_INVALID_PARAMETER => Err(SysprimsError::not_found(pid)),
                            _ => Err(SysprimsError::system(
                                "GenerateConsoleCtrlEvent failed",
                                err as i32,
                            )),
                        }
                    }
                }
                _ => Err(SysprimsError::not_supported(
                    format!("signal {sig}"),
                    "windows",
                )),
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod imp {
    use sysprims_core::{SysprimsError, SysprimsResult};

    use super::SignalBinding;

    pub(super) struct PlatformProcessHandle;

    pub(super) fn open_platform_handle(
        _pid: u32,
        _expected_start_identity: Option<&str>,
    ) -> SysprimsResult<(String, SignalBinding, PlatformProcessHandle)> {
        Err(SysprimsError::not_supported(
            "open_process_handle",
            std::env::consts::OS,
        ))
    }

    impl PlatformProcessHandle {
        pub(super) fn close_handle(&self) {}

        pub(super) fn check_exited(
            &self,
            _pid: u32,
            _start_identity: &str,
        ) -> SysprimsResult<bool> {
            Err(SysprimsError::not_supported(
                "open_process_handle",
                std::env::consts::OS,
            ))
        }

        pub(super) fn send_signal(
            &self,
            _pid: u32,
            _start_identity: &str,
            _sig: i32,
        ) -> SysprimsResult<SignalBinding> {
            Err(SysprimsError::not_supported(
                "open_process_handle",
                std::env::consts::OS,
            ))
        }
    }
}
