// SPDX-License-Identifier: MIT OR Apache-2.0

//! macOS (`macos/aarch64`, `macos/x86_64`) collector and `kqueue` `EVFILT_PROC`
//! `NOTE_EXIT` continuity handle for `native-process-observation:v0`.

use std::ffi::{CStr, CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use libc::{c_char, c_int, c_void};
use serde_json::{json, Value};
use sysprims_evidence::{
    derive_arguments_digest, NATIVE_PROCESS_OBSERVATION_PROFILE_ID,
    NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
};

use super::{
    digest_observed, errno_to_digest_fact, fact_non_observed, fact_observed, is_lowercase_uuid,
    observer_architecture, principal_observed, start_identity_non_observed,
    start_identity_observed, stream_executable_digest_from_fd, CollectorRefusal,
    ObservationOptions, ARGV_DIGEST_DERIVATION_ID, MACOS_PRINCIPAL_DERIVATION_ID,
    MACOS_START_IDENTITY_PROFILE_ID, MAX_ARGUMENTS_RAW_BYTES,
};

const KINFO_PROC_SIZE_64: usize = 648;
const OFF_P_STARTTIME_SEC: usize = 0;
const OFF_P_STARTTIME_USEC: usize = 8;
const OFF_P_FLAG: usize = 32;
const OFF_P_STAT: usize = 36;
const OFF_P_PID: usize = 40;
const OFF_P_RUID: usize = 392;
const OFF_P_RGID: usize = 400;
const OFF_CR_UID: usize = 420;
const OFF_CR_NGROUPS: usize = 424;
const OFF_CR_GROUPS0: usize = 428;

const P_LP64: u32 = 0x0000_0004;

const SIDL: u8 = 1;
const SRUN: u8 = 2;
const SSLEEP: u8 = 3;
const SSTOP: u8 = 4;
const SZOMB: u8 = 5;

const MACOS_PROC_PIDPATH_MAX: usize = 4096;
const MACOS_UNSUPPORTED_PIDNS_REASON: &str =
    "Darwin kernel has a single boot-scoped process table without PID namespaces.";
const MACOS_UNSUPPORTED_TIMENS_REASON: &str =
    "Darwin kernel has a single boot clock domain without time namespaces.";
const MACOS_UNSUPPORTED_USERNS_REASON: &str =
    "Darwin kernel has a single credential numbering domain without user namespaces.";
const MACOS_UNSUPPORTED_NS_CTX_REASON: &str =
    "Darwin kernel does not implement Linux-style process namespaces.";
const MACOS_DEFERRED_SECCTX_REASON: &str =
    "macOS security-context derivation is deferred in native-process-observation:v0.";
const DEFAULT_NOT_PROBED_REASON: &str = "Omitted under default opt-in disclosure policy.";

extern "C" {
    fn proc_pidpath(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;
}

struct OwnedFd(c_int);

impl Drop for OwnedFd {
    fn drop(&mut self) {
        if self.0 >= 0 {
            unsafe {
                let _ = libc::close(self.0);
            }
        }
    }
}

impl OwnedFd {
    fn into_raw(mut self) -> c_int {
        let fd = self.0;
        self.0 = -1;
        fd
    }
}

pub(super) struct PlatformHandle {
    kq_fd: c_int,
    exited: AtomicBool,
}

impl std::fmt::Debug for PlatformHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformHandle")
            .field("backend", &"macos-kqueue")
            .field("exited", &self.exited.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Drop for PlatformHandle {
    fn drop(&mut self) {
        if self.kq_fd >= 0 {
            unsafe {
                let _ = libc::close(self.kq_fd);
            }
        }
    }
}

impl PlatformHandle {
    #[cfg(test)]
    pub(super) fn new_sentinel_for_debug_test(fd: c_int) -> Self {
        Self {
            kq_fd: fd,
            exited: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(super) fn disarm_for_debug_test(&mut self) {
        self.kq_fd = -1;
    }

    pub(super) fn poll_exited(&self) -> Result<bool, CollectorRefusal> {
        self.wait_exit(Duration::ZERO)
    }

    pub(super) fn wait_exit(&self, timeout: Duration) -> Result<bool, CollectorRefusal> {
        if self.exited.load(Ordering::Acquire) {
            return Ok(true);
        }
        let deadline = Instant::now().checked_add(timeout);
        loop {
            let ts = if timeout.is_zero() {
                libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                }
            } else {
                let now = Instant::now();
                let rem = match deadline {
                    Some(dl) if dl > now => dl.duration_since(now),
                    Some(_) => Duration::ZERO,
                    None => timeout,
                };
                let secs = libc::time_t::try_from(rem.as_secs()).unwrap_or(libc::time_t::MAX);
                libc::timespec {
                    tv_sec: secs,
                    tv_nsec: libc::c_long::from(rem.subsec_nanos() as i32),
                }
            };

            let mut ev: libc::kevent = unsafe { std::mem::zeroed() };
            let rc = unsafe { libc::kevent(self.kq_fd, ptr::null(), 0, &mut ev, 1, &ts) };
            if rc > 0 {
                let flags = ev.flags;
                let fflags = ev.fflags;
                let data = ev.data;
                if (flags & libc::EV_ERROR) != 0 {
                    return Err(CollectorRefusal::error(
                        "kqueue-wait-error",
                        format!("kevent reported EV_ERROR errno={data}"),
                    ));
                }
                if (fflags & libc::NOTE_EXIT) != 0 {
                    self.exited.store(true, Ordering::Release);
                    return Ok(true);
                }
                return Ok(false);
            }
            if rc == 0 {
                return Ok(false);
            }
            let errno = last_errno();
            if errno == libc::EINTR {
                if timeout.is_zero() {
                    return Ok(false);
                }
                if let Some(dl) = deadline {
                    if Instant::now() >= dl {
                        return Ok(false);
                    }
                }
                continue;
            }
            return Err(CollectorRefusal::error(
                "kqueue-wait-failed",
                format!("kevent wait failed errno={errno}"),
            ));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcState {
    Live,
    Zombie,
}

impl ProcState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Zombie => "zombie",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KinfoSnapshot {
    start_identity: String,
    state: ProcState,
    is_lp64: bool,
    principal: Result<String, &'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KinfoOutcome {
    Observed(KinfoSnapshot),
    NotPresent,
    Inaccessible(&'static str),
    Error(&'static str),
}

fn last_errno() -> i32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

fn read_boot_session_uuid() -> Result<String, CollectorRefusal> {
    let name = b"kern.bootsessionuuid\0";
    let mut buf = [0u8; 64];
    let mut size = buf.len();
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr() as *const c_char,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size == 0 || size > buf.len() {
        return Err(CollectorRefusal::error(
            "boot-id-unavailable",
            "sysctlbyname(kern.bootsessionuuid) failed",
        ));
    }
    let slice = &buf[..size];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let raw_str = std::str::from_utf8(&slice[..end]).map_err(|_| {
        CollectorRefusal::error("invalid-boot-id", "kern.bootsessionuuid is not UTF-8")
    })?;
    let normalized = raw_str.trim().to_ascii_lowercase();
    if !is_lowercase_uuid(&normalized) {
        return Err(CollectorRefusal::error(
            "invalid-boot-id",
            "kern.bootsessionuuid failed lowercase UUID validation",
        ));
    }
    Ok(normalized)
}

fn read_kinfo_proc(pid: u32) -> KinfoOutcome {
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
            libc::ESRCH => KinfoOutcome::NotPresent,
            libc::EPERM | libc::EACCES => KinfoOutcome::Inaccessible("sysctl-kern-proc-pid-denied"),
            _ => KinfoOutcome::Error("sysctl-kern-proc-pid-failed"),
        };
    }
    if size == 0 {
        return KinfoOutcome::NotPresent;
    }
    if size != KINFO_PROC_SIZE_64 {
        return KinfoOutcome::Error("unexpected-kinfo-proc-size");
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
    let p_flag = u32::from_ne_bytes(buf[OFF_P_FLAG..OFF_P_FLAG + 4].try_into().unwrap());
    let p_stat = buf[OFF_P_STAT];
    let p_pid = i32::from_ne_bytes(buf[OFF_P_PID..OFF_P_PID + 4].try_into().unwrap());
    let p_ruid = u32::from_ne_bytes(buf[OFF_P_RUID..OFF_P_RUID + 4].try_into().unwrap());
    let p_rgid = u32::from_ne_bytes(buf[OFF_P_RGID..OFF_P_RGID + 4].try_into().unwrap());
    let cr_uid = u32::from_ne_bytes(buf[OFF_CR_UID..OFF_CR_UID + 4].try_into().unwrap());
    let cr_ngroups =
        i16::from_ne_bytes(buf[OFF_CR_NGROUPS..OFF_CR_NGROUPS + 2].try_into().unwrap());
    let cr_gid0 = u32::from_ne_bytes(buf[OFF_CR_GROUPS0..OFF_CR_GROUPS0 + 4].try_into().unwrap());

    if p_pid != (pid as i32) {
        return KinfoOutcome::Error("kinfo-pid-mismatch");
    }
    if tv_sec < 0 || !(0..=999_999).contains(&tv_usec) {
        return KinfoOutcome::Error("invalid-kinfo-starttime");
    }

    let state = match p_stat {
        SZOMB => ProcState::Zombie,
        SRUN | SSLEEP | SSTOP => ProcState::Live,
        SIDL => return KinfoOutcome::Error("embryonic-process-sidl"),
        _ => return KinfoOutcome::Error("unexpected-proc-stat"),
    };

    let principal = if cr_ngroups < 1 {
        Err("empty-kinfo-groups")
    } else {
        Ok(format!(
            "ruid={p_ruid};euid={cr_uid};rgid={p_rgid};egid={cr_gid0}"
        ))
    };

    KinfoOutcome::Observed(KinfoSnapshot {
        start_identity: format!("{tv_sec}.{tv_usec:06}"),
        state,
        is_lp64: (p_flag & P_LP64) != 0,
        principal,
    })
}

fn read_procargs2_raw(pid: u32, max_bytes: usize) -> Result<Vec<u8>, (i32, &'static str)> {
    let mut mib: [c_int; 3] = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as c_int];
    let mut size: usize = 0;
    let rc1 = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            ptr::null_mut(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if rc1 != 0 {
        return Err((last_errno(), "kern-procargs2-size-failed"));
    }
    if size == 0 {
        return Err((0, "empty-kern-procargs2"));
    }
    if size > max_bytes {
        return Err((libc::E2BIG, "arguments-exceed-cap"));
    }
    let mut buf = vec![0u8; size];
    let rc2 = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if rc2 != 0 {
        let errno = last_errno();
        if errno == libc::ENOMEM {
            return Err((libc::E2BIG, "arguments-exceed-cap"));
        }
        return Err((errno, "kern-procargs2-read-failed"));
    }
    if size > max_bytes {
        return Err((libc::E2BIG, "arguments-exceed-cap"));
    }
    buf.truncate(size);
    Ok(buf)
}

fn probe_macos_arguments_digest(pid: u32, is_zombie: bool, is_lp64: bool) -> Value {
    if !matches!(observer_architecture(), "aarch64" | "x86_64") || !is_lp64 {
        return fact_non_observed("unsupported", "unqualified-kern-procargs2-layout");
    }
    match read_procargs2_raw(pid, MAX_ARGUMENTS_RAW_BYTES) {
        Ok(buf) => match super::parse_procargs2_argv_64(&buf, MAX_ARGUMENTS_RAW_BYTES) {
            Ok(argv) => digest_observed(ARGV_DIGEST_DERIVATION_ID, derive_arguments_digest(&argv)),
            Err(reason) => fact_non_observed("error", reason),
        },
        Err((errno, reason)) => match errno {
            libc::E2BIG => fact_non_observed("error", "arguments-exceed-cap"),
            libc::EPERM | libc::EACCES => {
                fact_non_observed("inaccessible", "kern-procargs2-denied")
            }
            libc::ESRCH => fact_non_observed("not-present", "target-pid-absent"),
            libc::EINVAL | 0 if is_zombie => {
                fact_non_observed("not-present", "zombie-process-image-released")
            }
            _ => fact_non_observed("error", reason),
        },
    }
}

enum ExecPathResolution {
    Resolved(CString),
    BackingOpenError(i32),
    PidResolutionError(i32, &'static str),
}

fn resolve_macos_exec_path(pid: u32) -> ExecPathResolution {
    let mut path_buf = [0u8; MACOS_PROC_PIDPATH_MAX];
    let rc = unsafe {
        proc_pidpath(
            pid as c_int,
            path_buf.as_mut_ptr() as *mut c_void,
            MACOS_PROC_PIDPATH_MAX as u32,
        )
    };
    if rc > 0 {
        if let Ok(cstr) = CStr::from_bytes_until_nul(&path_buf) {
            if !cstr.to_bytes().is_empty() && cstr.to_bytes()[0] == b'/' {
                return ExecPathResolution::Resolved(cstr.to_owned());
            }
        }
    }
    let pidpath_errno = last_errno();

    // Fallback to KERN_PROCARGS2 exec_path when proc_pidpath did not resolve,
    // canonicalizing symlinks before opening with O_NOFOLLOW | O_NONBLOCK.
    if let Ok(buf) = read_procargs2_raw(pid, 1_048_576) {
        if buf.len() > 4 {
            if let Some(nul_off) = buf[4..].iter().position(|&b| b == 0) {
                let exec_slice = &buf[4..4 + nul_off];
                if !exec_slice.is_empty() && exec_slice[0] == b'/' {
                    match std::fs::canonicalize(Path::new(OsStr::from_bytes(exec_slice))) {
                        Ok(canon) => {
                            if let Ok(cstr) = CString::new(canon.as_os_str().as_bytes()) {
                                return ExecPathResolution::Resolved(cstr);
                            }
                            return ExecPathResolution::BackingOpenError(libc::EINVAL);
                        }
                        Err(err) => {
                            return ExecPathResolution::BackingOpenError(
                                err.raw_os_error().unwrap_or(libc::EIO),
                            );
                        }
                    }
                }
            }
        }
    }

    ExecPathResolution::PidResolutionError(pidpath_errno, "proc-pidpath-failed")
}

fn probe_macos_executable_digest(pid: u32, is_zombie: bool) -> Value {
    let c_path = match resolve_macos_exec_path(pid) {
        ExecPathResolution::Resolved(p) => p,
        ExecPathResolution::BackingOpenError(errno) => {
            return errno_to_digest_fact(errno, "executable-open-failed");
        }
        ExecPathResolution::PidResolutionError(errno, reason) => {
            return match errno {
                libc::EPERM | libc::EACCES => {
                    fact_non_observed("inaccessible", "proc-pidpath-denied")
                }
                libc::ESRCH | libc::ENOENT => fact_non_observed("not-present", "target-pid-absent"),
                libc::EINVAL | 0 if is_zombie => {
                    fact_non_observed("not-present", "zombie-process-image-released")
                }
                _ => fact_non_observed("error", reason),
            };
        }
    };

    let fd = loop {
        let rc = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if rc >= 0 {
            break rc;
        }
        let errno = last_errno();
        if errno == libc::EINTR {
            continue;
        }
        return errno_to_digest_fact(errno, "executable-open-failed");
    };
    let guard = OwnedFd(fd);
    stream_executable_digest_from_fd(guard.0)
}

#[allow(clippy::too_many_arguments)]
fn assemble_macos_payload(
    pid: u32,
    host_scope_json: Value,
    observed_at: &str,
    boot_id: &str,
    process_state_fact: Value,
    start_identity_fact: Value,
    security_principal_fact: Value,
    executable_digest_fact: Value,
    arguments_digest_fact: Value,
    options: &ObservationOptions,
) -> Value {
    let ns_ctx_fact = if options.include_namespace_context_digest {
        fact_non_observed("unsupported", MACOS_UNSUPPORTED_NS_CTX_REASON)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };
    let secctx_fact = if options.include_security_context_digest {
        fact_non_observed("unsupported", MACOS_DEFERRED_SECCTX_REASON)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };

    json!({
        "profile": {
            "id": NATIVE_PROCESS_OBSERVATION_PROFILE_ID,
            "revision": NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
        },
        "observed_at": observed_at,
        "observer": {
            "platform_family": "macos",
            "architecture": observer_architecture(),
            "bracketed": true,
            "time_namespace": fact_non_observed("unsupported", MACOS_UNSUPPORTED_TIMENS_REASON),
            "user_namespace": fact_non_observed("unsupported", MACOS_UNSUPPORTED_USERNS_REASON),
        },
        "process": {
            "subject_ref": options.subject_ref,
            "host_scope": host_scope_json,
            "boot_id": fact_observed(boot_id),
            "pid_numbering_namespace": fact_non_observed("unsupported", MACOS_UNSUPPORTED_PIDNS_REASON),
            "target_pid_namespace": fact_non_observed("unsupported", MACOS_UNSUPPORTED_PIDNS_REASON),
            "pid": pid.to_string(),
            "process_state": process_state_fact,
            "start_identity": start_identity_fact,
            "execution_context": {
                "security_principal": security_principal_fact,
                "namespace_context_digest": ns_ctx_fact,
                "executable_digest": executable_digest_fact,
                "arguments_digest": arguments_digest_fact,
                "security_context_digest": secctx_fact,
            }
        }
    })
}

pub(super) fn observe_process_impl(
    pid: u32,
    host_scope_json: Value,
    observed_at: &str,
    options: &ObservationOptions,
) -> Result<Value, CollectorRefusal> {
    for attempt in 0..2 {
        let boot_0 = read_boot_session_uuid()?;
        let k0 = read_kinfo_proc(pid);

        match k0 {
            KinfoOutcome::NotPresent => {
                let boot_1 = read_boot_session_uuid()?;
                if boot_0 != boot_1 {
                    if attempt == 0 {
                        continue;
                    }
                    return Err(CollectorRefusal::error(
                        "unbracketed-incarnation",
                        "boot session UUID changed during absent-process observation",
                    ));
                }
                let sp = if options.probe_security_principal {
                    fact_non_observed("not-present", "target-pid-absent")
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };
                let exe = if options.include_executable_digest {
                    fact_non_observed("not-present", "target-pid-absent")
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };
                let args = if options.include_arguments_digest {
                    fact_non_observed("not-present", "target-pid-absent")
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };
                return Ok(assemble_macos_payload(
                    pid,
                    host_scope_json,
                    observed_at,
                    &boot_0,
                    fact_non_observed("not-present", "target-pid-absent"),
                    start_identity_non_observed(
                        MACOS_START_IDENTITY_PROFILE_ID,
                        "not-present",
                        "target-pid-absent",
                    ),
                    sp,
                    exe,
                    args,
                    options,
                ));
            }
            KinfoOutcome::Inaccessible(reason) => {
                let boot_1 = read_boot_session_uuid()?;
                if boot_0 != boot_1 {
                    if attempt == 0 {
                        continue;
                    }
                    return Err(CollectorRefusal::error(
                        "unbracketed-incarnation",
                        "boot session UUID changed during inaccessible observation",
                    ));
                }
                let sp = if options.probe_security_principal {
                    fact_non_observed("inaccessible", reason)
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };
                let exe = if options.include_executable_digest {
                    fact_non_observed("inaccessible", reason)
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };
                let args = if options.include_arguments_digest {
                    fact_non_observed("inaccessible", reason)
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };
                return Ok(assemble_macos_payload(
                    pid,
                    host_scope_json,
                    observed_at,
                    &boot_0,
                    fact_non_observed("inaccessible", reason),
                    start_identity_non_observed(
                        MACOS_START_IDENTITY_PROFILE_ID,
                        "inaccessible",
                        reason,
                    ),
                    sp,
                    exe,
                    args,
                    options,
                ));
            }
            KinfoOutcome::Error(reason) => {
                if attempt == 0 {
                    continue;
                }
                return Err(CollectorRefusal::error(
                    reason,
                    "macos kinfo_proc probe failed",
                ));
            }
            KinfoOutcome::Observed(snap0) => {
                let is_zombie = snap0.state == ProcState::Zombie;
                let exe_fact = if options.include_executable_digest {
                    probe_macos_executable_digest(pid, is_zombie)
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };
                let args_fact = if options.include_arguments_digest {
                    probe_macos_arguments_digest(pid, is_zombie, snap0.is_lp64)
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };

                let boot_1 = read_boot_session_uuid()?;
                let k1 = read_kinfo_proc(pid);
                let KinfoOutcome::Observed(snap1) = k1 else {
                    if attempt == 0 {
                        continue;
                    }
                    return Err(CollectorRefusal::error(
                        "unbracketed-incarnation",
                        "target process vanished or changed state during observation bracket",
                    ));
                };
                if boot_0 != boot_1 || snap0 != snap1 {
                    if attempt == 0 {
                        continue;
                    }
                    return Err(CollectorRefusal::error(
                        "unbracketed-incarnation",
                        "incarnation tuple changed between pre/post sysctl snapshots",
                    ));
                }

                let sp_fact = if options.probe_security_principal {
                    match snap0.principal {
                        Ok(ref pval) => principal_observed(
                            MACOS_PRINCIPAL_DERIVATION_ID,
                            "urn:3leaps:posix-credentials:macos-kinfo:v0",
                            pval.clone(),
                        ),
                        Err(reason) => fact_non_observed("error", reason),
                    }
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };

                return Ok(assemble_macos_payload(
                    pid,
                    host_scope_json,
                    observed_at,
                    &boot_0,
                    fact_observed(snap0.state.as_str()),
                    start_identity_observed(MACOS_START_IDENTITY_PROFILE_ID, snap0.start_identity),
                    sp_fact,
                    exe_fact,
                    args_fact,
                    options,
                ));
            }
        }
    }

    Err(CollectorRefusal::error(
        "unbracketed-incarnation",
        "failed to bracket macOS process observation",
    ))
}

pub(super) fn open_continuity_handle_impl(
    pid: u32,
    host_scope_json: Value,
    observed_at: &str,
    options: &ObservationOptions,
) -> Result<(Value, PlatformHandle), CollectorRefusal> {
    let boot_0 = read_boot_session_uuid()?;
    let snap0 = match read_kinfo_proc(pid) {
        KinfoOutcome::Observed(s) => s,
        KinfoOutcome::NotPresent => {
            return Err(CollectorRefusal::not_present(
                "target-not-present",
                "target PID does not exist",
            ))
        }
        KinfoOutcome::Inaccessible(r) => {
            return Err(CollectorRefusal::inaccessible(
                r,
                "target PID is inaccessible",
            ))
        }
        KinfoOutcome::Error(r) => {
            return Err(CollectorRefusal::error(
                r,
                "failed to read target kinfo_proc",
            ))
        }
    };
    if snap0.state != ProcState::Live {
        return Err(CollectorRefusal::error(
            "target-not-live",
            "continuity handle requires a live target process (not zombie)",
        ));
    }

    let raw_kq = unsafe { libc::kqueue() };
    if raw_kq < 0 {
        return Err(CollectorRefusal::error(
            "kqueue-create-failed",
            format!("kqueue() failed errno={}", last_errno()),
        ));
    }
    let kq_guard = OwnedFd(raw_kq);
    let cloexec_rc = unsafe { libc::fcntl(kq_guard.0, libc::F_SETFD, libc::FD_CLOEXEC) };
    if cloexec_rc < 0 {
        return Err(CollectorRefusal::error(
            "kqueue-cloexec-failed",
            format!("fcntl(FD_CLOEXEC) failed errno={}", last_errno()),
        ));
    }

    let kev = libc::kevent {
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
    let reg_rc = unsafe { libc::kevent(kq_guard.0, &kev, 1, &mut out_ev, 1, &zero_ts) };
    if reg_rc < 0 {
        let errno = last_errno();
        return Err(match errno {
            libc::ESRCH => CollectorRefusal::not_present(
                "target-not-present",
                "target PID exited before kqueue registration",
            ),
            libc::EPERM | libc::EACCES => CollectorRefusal::inaccessible(
                "kqueue-proc-denied",
                "kernel denied EVFILT_PROC NOTE_EXIT registration",
            ),
            _ => CollectorRefusal::error(
                "kqueue-registration-failed",
                format!("kevent registration failed errno={errno}"),
            ),
        });
    }
    if reg_rc > 0 {
        if (out_ev.flags & libc::EV_ERROR) != 0 {
            let errno = out_ev.data as i32;
            return Err(match errno {
                libc::ESRCH => CollectorRefusal::not_present(
                    "target-not-present",
                    "target PID exited before kqueue registration",
                ),
                libc::EPERM | libc::EACCES => CollectorRefusal::inaccessible(
                    "kqueue-proc-denied",
                    "kernel denied EVFILT_PROC NOTE_EXIT registration",
                ),
                _ => CollectorRefusal::error(
                    "kqueue-registration-failed",
                    format!("kevent EV_ERROR errno={errno}"),
                ),
            });
        }
        if (out_ev.fflags & libc::NOTE_EXIT) != 0 {
            return Err(CollectorRefusal::error(
                "target-not-live",
                "target process exited during kqueue registration",
            ));
        }
    }

    let exe_fact = if options.include_executable_digest {
        probe_macos_executable_digest(pid, false)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };
    let args_fact = if options.include_arguments_digest {
        probe_macos_arguments_digest(pid, false, snap0.is_lp64)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };

    let boot_1 = read_boot_session_uuid()?;
    let snap1 = match read_kinfo_proc(pid) {
        KinfoOutcome::Observed(s) => s,
        _ => {
            return Err(CollectorRefusal::error(
                "unbracketed-incarnation",
                "target process vanished during continuity handle registration",
            ))
        }
    };
    if boot_0 != boot_1 || snap0 != snap1 || snap1.state != ProcState::Live {
        return Err(CollectorRefusal::error(
            "unbracketed-incarnation",
            "incarnation changed during continuity handle registration",
        ));
    }

    let sp_fact = if options.probe_security_principal {
        match snap0.principal {
            Ok(ref pval) => principal_observed(
                MACOS_PRINCIPAL_DERIVATION_ID,
                "urn:3leaps:posix-credentials:macos-kinfo:v0",
                pval.clone(),
            ),
            Err(reason) => fact_non_observed("error", reason),
        }
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };

    let baseline = assemble_macos_payload(
        pid,
        host_scope_json,
        observed_at,
        &boot_0,
        fact_observed("live"),
        start_identity_observed(MACOS_START_IDENTITY_PROFILE_ID, snap0.start_identity),
        sp_fact,
        exe_fact,
        args_fact,
        options,
    );

    Ok((
        baseline,
        PlatformHandle {
            kq_fd: kq_guard.into_raw(),
            exited: AtomicBool::new(false),
        },
    ))
}
