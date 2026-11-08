// SPDX-License-Identifier: MIT OR Apache-2.0

//! Linux (`linux/x86_64`, `linux/aarch64`) descriptor-anchored collector and
//! `pidfd_open` continuity handle for `native-process-observation:v0`.
//!
//! Enforces the three explicit descriptor-resolution classes:
//! - Class A: `readlinkat(proc_fd, "self")` == canonical decimal `getpid()`
//!   (never passes `"self"` to `openat2` or `openat`).
//! - Class B: `openat2` with `RESOLVE_BENEATH | RESOLVE_NO_XDEV |
//!   RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS` for all `procfs` directories
//!   and regular files; `ENOSYS` refuses with `openat2-unavailable`.
//! - Class C: single-component `openat(fd, entry, O_RDONLY | O_CLOEXEC)` for
//!   kernel magic links (`ns/*` verified `NSFS_MAGIC`; `exe` verified `S_ISREG`).

use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use libc::{c_char, c_int, c_uint, c_void};
use serde_json::{json, Value};
use sysprims_evidence::{
    derive_arguments_digest, derive_linux_namespace_context_digest,
    derive_linux_security_context_digest, linux_id_is_unambiguously_mapped, LINUX_NAMESPACE_KINDS,
    NATIVE_PROCESS_OBSERVATION_PROFILE_ID, NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
};

use super::{
    digest_observed, errno_to_digest_fact, fact_non_observed, fact_observed, is_lowercase_uuid,
    observer_architecture, principal_observed, start_identity_non_observed,
    start_identity_observed, stream_executable_digest_from_fd, CollectorRefusal,
    ObservationOptions, ARGV_DIGEST_DERIVATION_ID, LINUX_NS_DIGEST_DERIVATION_ID,
    LINUX_PRINCIPAL_DERIVATION_ID, LINUX_SECCTX_DIGEST_DERIVATION_ID,
    LINUX_START_IDENTITY_PROFILE_ID, MAX_ARGUMENTS_RAW_BYTES,
};

const PROC_SUPER_MAGIC_U64: u64 = 0x9fa0;
const NSFS_MAGIC_U64: u64 = 0x6e73_6673;

const RESOLVE_NO_XDEV: u64 = 0x01;
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;
const CLASS_B_RESOLVE: u64 =
    RESOLVE_BENEATH | RESOLVE_NO_XDEV | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS;

const MAX_SMALL_PROC_FILE_BYTES: usize = 65_536;
const MAX_MOUNTINFO_BYTES: usize = 524_288;
const DEFAULT_NOT_PROBED_REASON: &str = "Omitted under default opt-in disclosure policy.";

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
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
    pidfd: c_int,
    exited: AtomicBool,
}

impl std::fmt::Debug for PlatformHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformHandle")
            .field("backend", &"linux-pidfd")
            .field("exited", &self.exited.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Drop for PlatformHandle {
    fn drop(&mut self) {
        if self.pidfd >= 0 {
            unsafe {
                let _ = libc::close(self.pidfd);
            }
        }
    }
}

impl PlatformHandle {
    #[cfg(test)]
    pub(super) fn new_sentinel_for_debug_test(fd: c_int) -> Self {
        Self {
            pidfd: fd,
            exited: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(super) fn disarm_for_debug_test(&mut self) {
        self.pidfd = -1;
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
            let timeout_ms: c_int = if timeout.is_zero() {
                0
            } else {
                let now = Instant::now();
                let rem = match deadline {
                    Some(dl) if dl > now => dl.duration_since(now),
                    Some(_) => Duration::ZERO,
                    None => timeout,
                };
                let ms = rem.as_millis();
                if ms == 0 && !rem.is_zero() {
                    1
                } else {
                    c_int::try_from(ms).unwrap_or(c_int::MAX)
                }
            };

            let mut pfd = libc::pollfd {
                fd: self.pidfd,
                events: libc::POLLIN,
                revents: 0,
            };
            let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
            if rc > 0 {
                if (pfd.revents & libc::POLLNVAL) != 0 {
                    return Err(CollectorRefusal::error(
                        "pidfd-invalid",
                        "poll reported POLLNVAL on pidfd",
                    ));
                }
                if (pfd.revents & (libc::POLLIN | libc::POLLHUP)) != 0 {
                    self.exited.store(true, Ordering::Release);
                    return Ok(true);
                }
                if (pfd.revents & libc::POLLERR) != 0 {
                    return Err(CollectorRefusal::error(
                        "pidfd-poll-error",
                        "poll reported POLLERR on pidfd without exit readiness",
                    ));
                }
                return Ok(false);
            }
            if rc == 0 {
                if timeout.is_zero() {
                    return Ok(false);
                }
                if let Some(dl) = deadline {
                    if Instant::now() >= dl {
                        return Ok(false);
                    }
                    continue;
                }
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
                "pidfd-poll-failed",
                format!("poll(pidfd) failed errno={errno}"),
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
enum NsProbe {
    Observed(String),
    NotPresent(&'static str),
    Inaccessible(&'static str),
    Error(&'static str),
}

impl NsProbe {
    fn to_fact(&self) -> Value {
        match self {
            Self::Observed(v) => fact_observed(v.clone()),
            Self::NotPresent(r) => fact_non_observed("not-present", r),
            Self::Inaccessible(r) => fact_non_observed("inaccessible", r),
            Self::Error(r) => fact_non_observed("error", r),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UserNsIdMapping {
    uid_extents: Vec<(u32, u32)>,
    gid_extents: Vec<(u32, u32)>,
    overflow_uid: u32,
    overflow_gid: u32,
}

type FactProbeOutcome<T> = Result<T, (String, &'static str)>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObserverAnchor {
    boot_id: String,
    pid_numbering_ns: String,
    time_ns: NsProbe,
    user_ns: NsProbe,
    id_mapping: Option<FactProbeOutcome<UserNsIdMapping>>,
    hidepid_conceals: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxIncarnation {
    anchor: ObserverAnchor,
    target_pid_ns: NsProbe,
    tgid: u32,
    start_ticks: String,
    state: ProcState,
    uids: Option<[u32; 4]>,
    gids: Option<[u32; 4]>,
}

enum TargetSnapshot {
    Observed {
        incarnation: LinuxIncarnation,
        pid_dir: OwnedFd,
        status_text: String,
    },
    NotPresent {
        anchor: ObserverAnchor,
        reason: &'static str,
    },
    Unknown {
        anchor: ObserverAnchor,
        reason: &'static str,
    },
    Inaccessible {
        anchor: ObserverAnchor,
        reason: &'static str,
    },
}

fn last_errno() -> i32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

fn open_verified_proc_root() -> Result<(OwnedFd, u32, u32), CollectorRefusal> {
    let path = b"/proc\0";
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    let fd = loop {
        let rc = unsafe { libc::open(path.as_ptr() as *const c_char, flags) };
        if rc >= 0 {
            break rc;
        }
        let errno = last_errno();
        if errno == libc::EINTR {
            continue;
        }
        return Err(CollectorRefusal::error(
            "procfs-root-unverified",
            format!("open(/proc) failed errno={errno}"),
        ));
    };
    let guard = OwnedFd(fd);

    let mut stfs: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatfs(guard.0, &mut stfs) } != 0 {
        return Err(CollectorRefusal::error(
            "procfs-root-unverified",
            format!("fstatfs(/proc) failed errno={}", last_errno()),
        ));
    }
    if (stfs.f_type as u64) != PROC_SUPER_MAGIC_U64 {
        return Err(CollectorRefusal::error(
            "procfs-root-unverified",
            format!(
                "unexpected /proc filesystem magic 0x{:x}",
                stfs.f_type as u64
            ),
        ));
    }

    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(guard.0, &mut st) } != 0 {
        return Err(CollectorRefusal::error(
            "procfs-root-unverified",
            format!("fstat(/proc) failed errno={}", last_errno()),
        ));
    }
    let maj = libc::major(st.st_dev);
    let min = libc::minor(st.st_dev);
    Ok((guard, maj, min))
}

/// Class A: `readlinkat(proc_fd, "self")` must equal canonical decimal `getpid()`.
fn verify_self_symlink(proc_fd: c_int) -> Result<String, CollectorRefusal> {
    let self_pid = unsafe { libc::getpid() };
    if self_pid <= 0 {
        return Err(CollectorRefusal::error(
            "self-pid-mismatch",
            "getpid() returned non-positive value",
        ));
    }
    let expected = self_pid.to_string();
    let name = b"self\0";
    let mut buf = [0u8; 64];
    let n = unsafe {
        libc::readlinkat(
            proc_fd,
            name.as_ptr() as *const c_char,
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        )
    };
    if n <= 0 || (n as usize) >= buf.len() {
        return Err(CollectorRefusal::error(
            "self-pid-mismatch",
            format!("readlinkat(proc_fd, self) failed errno={}", last_errno()),
        ));
    }
    let link_text = std::str::from_utf8(&buf[..n as usize]).map_err(|_| {
        CollectorRefusal::error("self-pid-mismatch", "/proc/self link is not UTF-8")
    })?;
    if link_text != expected {
        return Err(CollectorRefusal::error(
            "self-pid-mismatch",
            format!("/proc/self resolved to {link_text:?}, expected {expected:?}"),
        ));
    }
    Ok(expected)
}

/// Class B: `openat2` with strict `RESOLVE_BENEATH | RESOLVE_NO_XDEV |
/// RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`.
fn openat2_class_b(dir_fd: c_int, rel_path: &str, directory: bool) -> Result<OwnedFd, i32> {
    if rel_path.is_empty()
        || rel_path.starts_with('/')
        || rel_path == "self"
        || rel_path.starts_with("self/")
    {
        return Err(libc::EINVAL);
    }
    let c_rel = CString::new(rel_path).map_err(|_| libc::EINVAL)?;
    let mut flags = (libc::O_RDONLY | libc::O_CLOEXEC) as u64;
    if directory {
        flags |= libc::O_DIRECTORY as u64;
    }
    let how = OpenHow {
        flags,
        mode: 0,
        resolve: CLASS_B_RESOLVE,
    };
    loop {
        let rc = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                dir_fd,
                c_rel.as_ptr(),
                &how as *const OpenHow,
                std::mem::size_of::<OpenHow>(),
            )
        };
        if rc >= 0 {
            return Ok(OwnedFd(rc as c_int));
        }
        let errno = last_errno();
        if errno == libc::EINTR {
            continue;
        }
        return Err(errno);
    }
}

fn map_class_b_observer_err(errno: i32, context: &str) -> CollectorRefusal {
    match errno {
        libc::ENOSYS => CollectorRefusal::unsupported(
            "openat2-unavailable",
            "Linux kernel does not support openat2(2); unconfined procfs fallback is forbidden",
        ),
        libc::EXDEV | libc::ELOOP => CollectorRefusal::error(
            "procfs-confinement-violation",
            format!("openat2 confinement rejected {context} (errno={errno})"),
        ),
        libc::EPERM | libc::EACCES => CollectorRefusal::inaccessible(
            "procfs-observer-denied",
            format!("permission denied opening {context}"),
        ),
        _ => CollectorRefusal::error(
            "procfs-observer-error",
            format!("openat2 failed on {context} (errno={errno})"),
        ),
    }
}

/// Class C: single-component `openat(parent_fd, name, O_RDONLY | O_CLOEXEC)` for
/// kernel magic links (`ns/*` and `exe`).
fn openat_class_c_single(parent_fd: c_int, entry: &str) -> Result<OwnedFd, i32> {
    if entry.is_empty() || entry.contains('/') || entry == "." || entry == ".." || entry == "self" {
        return Err(libc::EINVAL);
    }
    let c_entry = CString::new(entry).map_err(|_| libc::EINVAL)?;
    loop {
        let rc = unsafe {
            libc::openat(
                parent_fd,
                c_entry.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC,
            )
        };
        if rc >= 0 {
            return Ok(OwnedFd(rc));
        }
        let errno = last_errno();
        if errno == libc::EINTR {
            continue;
        }
        return Err(errno);
    }
}

fn read_ns_magic_id(ns_dir_fd: c_int, kind: &str, prefix: &str) -> NsProbe {
    let fd = match openat_class_c_single(ns_dir_fd, kind) {
        Ok(f) => f,
        Err(errno) => {
            return match errno {
                libc::ENOENT => NsProbe::NotPresent("namespace-entry-absent"),
                libc::EPERM | libc::EACCES => NsProbe::Inaccessible("ptrace-may-access-denied"),
                _ => NsProbe::Error("namespace-open-failed"),
            };
        }
    };

    let mut stfs: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatfs(fd.0, &mut stfs) } != 0 {
        return NsProbe::Error("namespace-fstatfs-failed");
    }
    if (stfs.f_type as u64) != NSFS_MAGIC_U64 {
        return NsProbe::Error("namespace-not-nsfs-magic");
    }

    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd.0, &mut st) } != 0 {
        return NsProbe::Error("namespace-fstat-failed");
    }
    let maj = libc::major(st.st_dev);
    let min = libc::minor(st.st_dev);
    let ino = st.st_ino as u64;
    NsProbe::Observed(format!("{prefix}:{maj}:{min}:{ino}"))
}

fn read_fd_bytes_capped(fd: c_int, max_bytes: usize) -> Result<Vec<u8>, (i32, &'static str)> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if n == 0 {
            break;
        }
        if n < 0 {
            let errno = last_errno();
            if errno == libc::EINTR {
                continue;
            }
            return Err((errno, "procfs-read-failed"));
        }
        let chunk = &buf[..n as usize];
        if out.len().saturating_add(chunk.len()) > max_bytes {
            return Err((libc::E2BIG, "procfs-file-exceeds-cap"));
        }
        out.extend_from_slice(chunk);
    }
    Ok(out)
}

fn read_class_b_text(
    dir_fd: c_int,
    rel_path: &str,
    max_bytes: usize,
) -> Result<String, (i32, &'static str)> {
    let fd = openat2_class_b(dir_fd, rel_path, false).map_err(|e| (e, "openat2-failed"))?;
    let bytes = read_fd_bytes_capped(fd.0, max_bytes)?;
    String::from_utf8(bytes).map_err(|_| (libc::EILSEQ, "procfs-invalid-utf8"))
}

fn parse_canonical_u32_str(s: &str) -> Option<u32> {
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    s.parse::<u32>().ok()
}

fn parse_canonical_u64_str(s: &str) -> Option<u64> {
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    s.parse::<u64>().ok()
}

fn parse_id_map_extents(text: &str) -> Option<Vec<(u32, u32)>> {
    let mut extents = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let toks: Vec<&str> = trimmed.split_whitespace().collect();
        if toks.len() != 3 {
            return None;
        }
        let first = toks[0].parse::<u32>().ok()?;
        let _lower = toks[1].parse::<u32>().ok()?;
        let count = toks[2].parse::<u32>().ok()?;
        extents.push((first, count));
    }
    Some(extents)
}

fn mountinfo_conceals_pids(mountinfo_text: &str, proc_maj: u32, proc_min: u32) -> bool {
    let expected_dev = format!("{proc_maj}:{proc_min}");
    let mut matched_any = false;
    for line in mountinfo_text.lines() {
        let Some((lhs, rhs)) = line.split_once(" - ") else {
            continue;
        };
        let lfields: Vec<&str> = lhs.split_whitespace().collect();
        let rfields: Vec<&str> = rhs.split_whitespace().collect();
        if lfields.len() < 5 || rfields.len() < 3 {
            continue;
        }
        let dev_id = lfields[2];
        let mount_point = lfields[4];
        let fstype = rfields[0];
        if fstype != "proc" || (mount_point != "/proc" && dev_id != expected_dev) {
            continue;
        }
        matched_any = true;
        for opt in lfields
            .get(5)
            .copied()
            .unwrap_or("")
            .split(',')
            .chain(rfields[2].split(','))
        {
            if opt == "subset=pid" {
                return true;
            }
            if let Some(val) = opt.strip_prefix("hidepid=") {
                if !matches!(val, "0" | "off" | "1" | "noaccess") {
                    return true;
                }
            }
        }
    }
    !matched_any
}

fn parse_proc_stat(stat_text: &str) -> Result<(ProcState, String), &'static str> {
    let close_idx = stat_text
        .rfind(')')
        .ok_or("malformed-procfs-stat-missing-comm")?;
    let after = stat_text
        .get(close_idx + 1..)
        .ok_or("malformed-procfs-stat-tail")?;
    let rest = after
        .strip_prefix(' ')
        .ok_or("malformed-procfs-stat-delimiter")?;
    let fields: Vec<&str> = rest.trim_end_matches('\n').split(' ').collect();
    if fields.len() <= 19 {
        return Err("malformed-procfs-stat-field-count");
    }
    let state_tok = fields[0];
    if state_tok.len() != 1 {
        return Err("malformed-procfs-stat-state");
    }
    let state = match state_tok.as_bytes()[0] {
        b'Z' => ProcState::Zombie,
        b'R' | b'S' | b'D' | b'T' | b't' | b'W' | b'I' | b'P' => ProcState::Live,
        _ => return Err("transient-or-dead-procfs-state"),
    };
    let start_ticks_tok = fields[19];
    if parse_canonical_u64_str(start_ticks_tok).is_none() {
        return Err("invalid-procfs-starttime-ticks");
    }
    Ok((state, start_ticks_tok.to_string()))
}

fn status_line_map(status_text: &str) -> BTreeMap<&str, &str> {
    let mut map = BTreeMap::new();
    for line in status_text.lines() {
        if let Some((k, v)) = line.split_once(':') {
            map.entry(k.trim()).or_insert_with(|| v.trim());
        }
    }
    map
}

fn parse_quad_u32(val: &str) -> Option<[u32; 4]> {
    let toks: Vec<&str> = val.split_whitespace().collect();
    if toks.len() != 4 {
        return None;
    }
    Some([
        toks[0].parse::<u32>().ok()?,
        toks[1].parse::<u32>().ok()?,
        toks[2].parse::<u32>().ok()?,
        toks[3].parse::<u32>().ok()?,
    ])
}

fn collect_observer_anchor(
    proc_fd: c_int,
    proc_maj: u32,
    proc_min: u32,
    need_id_maps: bool,
) -> Result<ObserverAnchor, CollectorRefusal> {
    let self_pid_str = verify_self_symlink(proc_fd)?;
    let self_dir = openat2_class_b(proc_fd, &self_pid_str, true)
        .map_err(|e| map_class_b_observer_err(e, "self_dir"))?;

    // Step 3: Descriptor-anchored NSpid check on self_dir/status.
    let self_status = read_class_b_text(self_dir.0, "status", MAX_SMALL_PROC_FILE_BYTES)
        .map_err(|(e, _)| map_class_b_observer_err(e, "self_dir/status"))?;
    let self_map = status_line_map(&self_status);
    let Some(nspid_raw) = self_map.get("NSpid") else {
        return Err(CollectorRefusal::error(
            "missing-self-nspid",
            "self/status lacks NSpid line",
        ));
    };
    let nspid_toks: Vec<&str> = nspid_raw.split_whitespace().collect();
    if nspid_toks.len() != 1 || nspid_toks[0] != self_pid_str {
        return Err(CollectorRefusal::error(
            "nested-or-mismatched-nspid",
            format!("self/status NSpid={nspid_raw:?} does not match single-level getpid()={self_pid_str}"),
        ));
    }

    let hidepid_conceals = match read_class_b_text(self_dir.0, "mountinfo", MAX_MOUNTINFO_BYTES) {
        Ok(mi) => mountinfo_conceals_pids(&mi, proc_maj, proc_min),
        Err((libc::ENOSYS, _)) => {
            return Err(map_class_b_observer_err(libc::ENOSYS, "self_dir/mountinfo"))
        }
        Err((e, _)) if e == libc::EXDEV || e == libc::ELOOP => {
            return Err(map_class_b_observer_err(e, "self_dir/mountinfo"))
        }
        Err(_) => true,
    };

    // Step 4: Numbering, observer time, and observer user namespaces from self_ns_fd.
    let self_ns = openat2_class_b(self_dir.0, "ns", true)
        .map_err(|e| map_class_b_observer_err(e, "self_dir/ns"))?;
    let pid_numbering_ns = match read_ns_magic_id(self_ns.0, "pid", "pidns") {
        NsProbe::Observed(v) => v,
        NsProbe::Inaccessible(_) => {
            return Err(CollectorRefusal::inaccessible(
                "pid-numbering-namespace-denied",
                "unable to read observer self/ns/pid",
            ))
        }
        _ => {
            return Err(CollectorRefusal::error(
                "pid-numbering-namespace-unavailable",
                "unable to verify observer self/ns/pid",
            ))
        }
    };
    let time_ns = match read_ns_magic_id(self_ns.0, "time", "timens") {
        NsProbe::NotPresent(_) => NsProbe::NotPresent("kernel-without-timens"),
        other => other,
    };
    let user_ns = match read_ns_magic_id(self_ns.0, "user", "userns") {
        NsProbe::NotPresent(_) => NsProbe::NotPresent("kernel-without-userns"),
        other => other,
    };

    let id_mapping = if need_id_maps && matches!(user_ns, NsProbe::Observed(_)) {
        Some(load_userns_id_mapping(proc_fd, self_dir.0)?)
    } else {
        None
    };

    // Step 5: Boot ID newline stripping via Class B openat2.
    let boot_raw = read_class_b_text(proc_fd, "sys/kernel/random/boot_id", 128)
        .map_err(|(e, _)| map_class_b_observer_err(e, "sys/kernel/random/boot_id"))?;
    let boot_id = boot_raw.trim_end_matches('\n').to_string();
    if !is_lowercase_uuid(&boot_id) {
        return Err(CollectorRefusal::error(
            "invalid-boot-id",
            "Linux boot_id failed lowercase UUID validation",
        ));
    }

    Ok(ObserverAnchor {
        boot_id,
        pid_numbering_ns,
        time_ns,
        user_ns,
        id_mapping,
        hidepid_conceals,
    })
}

fn load_userns_id_mapping(
    proc_fd: c_int,
    self_dir_fd: c_int,
) -> Result<FactProbeOutcome<UserNsIdMapping>, CollectorRefusal> {
    let read_map =
        |fd: c_int, rel: &str| -> Result<FactProbeOutcome<Vec<(u32, u32)>>, CollectorRefusal> {
            match read_class_b_text(fd, rel, MAX_SMALL_PROC_FILE_BYTES) {
                Ok(txt) => match parse_id_map_extents(&txt) {
                    Some(ext) => Ok(Ok(ext)),
                    None => Ok(Err(("error".into(), "malformed-userns-id-map"))),
                },
                Err((libc::ENOSYS, _)) => Err(map_class_b_observer_err(libc::ENOSYS, rel)),
                Err((e, _)) if e == libc::EXDEV || e == libc::ELOOP => {
                    Err(map_class_b_observer_err(e, rel))
                }
                Err((e, _)) if e == libc::EPERM || e == libc::EACCES => {
                    Ok(Err(("inaccessible".into(), "userns-id-map-denied")))
                }
                Err(_) => Ok(Err(("error".into(), "userns-id-map-read-failed"))),
            }
        };
    let read_overflow = |rel: &str| -> Result<FactProbeOutcome<u32>, CollectorRefusal> {
        match read_class_b_text(proc_fd, rel, 64) {
            Ok(txt) => match txt.trim().parse::<u32>() {
                Ok(v) => Ok(Ok(v)),
                Err(_) => Ok(Err(("error".into(), "malformed-overflow-id"))),
            },
            Err((libc::ENOSYS, _)) => Err(map_class_b_observer_err(libc::ENOSYS, rel)),
            Err((e, _)) if e == libc::EXDEV || e == libc::ELOOP => {
                Err(map_class_b_observer_err(e, rel))
            }
            Err((e, _)) if e == libc::EPERM || e == libc::EACCES => {
                Ok(Err(("inaccessible".into(), "overflow-id-denied")))
            }
            Err(_) => Ok(Err(("error".into(), "overflow-id-read-failed"))),
        }
    };

    let uid_extents = match read_map(self_dir_fd, "uid_map")? {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };
    let gid_extents = match read_map(self_dir_fd, "gid_map")? {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };
    let overflow_uid = match read_overflow("sys/kernel/overflowuid")? {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };
    let overflow_gid = match read_overflow("sys/kernel/overflowgid")? {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };

    Ok(Ok(UserNsIdMapping {
        uid_extents,
        gid_extents,
        overflow_uid,
        overflow_gid,
    }))
}

fn capture_target_snapshot(
    proc_fd: c_int,
    proc_maj: u32,
    proc_min: u32,
    pid: u32,
    need_id_maps: bool,
) -> Result<TargetSnapshot, CollectorRefusal> {
    let anchor = collect_observer_anchor(proc_fd, proc_maj, proc_min, need_id_maps)?;
    let pid_str = pid.to_string();

    let pid_dir = match openat2_class_b(proc_fd, &pid_str, true) {
        Ok(fd) => fd,
        Err(libc::ENOSYS) => return Err(map_class_b_observer_err(libc::ENOSYS, "<pid>")),
        Err(e) if e == libc::EXDEV || e == libc::ELOOP => {
            return Err(map_class_b_observer_err(e, "<pid>"))
        }
        Err(libc::ENOENT) => {
            if anchor.hidepid_conceals {
                return Ok(TargetSnapshot::Unknown {
                    anchor,
                    reason: "procfs-hidepid-concealment-possible",
                });
            }
            return Ok(TargetSnapshot::NotPresent {
                anchor,
                reason: "target-pid-absent",
            });
        }
        Err(e) if e == libc::EPERM || e == libc::EACCES => {
            return Ok(TargetSnapshot::Inaccessible {
                anchor,
                reason: "procfs-pid-dir-denied",
            });
        }
        Err(e) => {
            return Err(CollectorRefusal::error(
                "procfs-pid-open-failed",
                format!("openat2(<pid>) failed errno={e}"),
            ));
        }
    };

    let status_text = match read_class_b_text(pid_dir.0, "status", MAX_SMALL_PROC_FILE_BYTES) {
        Ok(t) => t,
        Err((libc::ENOSYS, _)) => {
            return Err(map_class_b_observer_err(libc::ENOSYS, "<pid>/status"))
        }
        Err((e, _)) if e == libc::EXDEV || e == libc::ELOOP => {
            return Err(map_class_b_observer_err(e, "<pid>/status"))
        }
        Err((e, _)) if e == libc::ENOENT || e == libc::ESRCH => {
            if anchor.hidepid_conceals {
                return Ok(TargetSnapshot::Unknown {
                    anchor,
                    reason: "procfs-hidepid-concealment-possible",
                });
            }
            return Ok(TargetSnapshot::NotPresent {
                anchor,
                reason: "target-pid-absent",
            });
        }
        Err((e, _)) if e == libc::EPERM || e == libc::EACCES => {
            return Ok(TargetSnapshot::Inaccessible {
                anchor,
                reason: "procfs-status-denied",
            });
        }
        Err((_, r)) => {
            return Err(CollectorRefusal::error(r, "failed reading <pid>/status"));
        }
    };

    let smap = status_line_map(&status_text);
    let Some(tgid_str) = smap.get("Tgid") else {
        return Err(CollectorRefusal::error(
            "missing-procfs-tgid",
            "<pid>/status missing Tgid line",
        ));
    };
    let Some(tgid) = parse_canonical_u32_str(tgid_str) else {
        return Err(CollectorRefusal::error(
            "invalid-procfs-tgid",
            "<pid>/status Tgid is not a canonical u32",
        ));
    };
    // Step 6: Thread-group leader guard (Tgid == pid).
    if tgid != pid {
        return Ok(TargetSnapshot::NotPresent {
            anchor,
            reason: "tid-not-thread-group-leader",
        });
    }

    // Step 7: Safe <pid_dir>/stat parsing and target PID namespace.
    let stat_text = match read_class_b_text(pid_dir.0, "stat", MAX_SMALL_PROC_FILE_BYTES) {
        Ok(t) => t,
        Err((libc::ENOSYS, _)) => return Err(map_class_b_observer_err(libc::ENOSYS, "<pid>/stat")),
        Err((e, _)) if e == libc::EXDEV || e == libc::ELOOP => {
            return Err(map_class_b_observer_err(e, "<pid>/stat"))
        }
        Err((e, _)) if e == libc::ENOENT || e == libc::ESRCH => {
            if anchor.hidepid_conceals {
                return Ok(TargetSnapshot::Unknown {
                    anchor,
                    reason: "procfs-hidepid-concealment-possible",
                });
            }
            return Ok(TargetSnapshot::NotPresent {
                anchor,
                reason: "target-pid-absent",
            });
        }
        Err((e, _)) if e == libc::EPERM || e == libc::EACCES => {
            return Ok(TargetSnapshot::Inaccessible {
                anchor,
                reason: "procfs-stat-denied",
            });
        }
        Err((_, r)) => return Err(CollectorRefusal::error(r, "failed reading <pid>/stat")),
    };

    let (state, start_ticks) = parse_proc_stat(&stat_text)
        .map_err(|r| CollectorRefusal::error(r, "invalid <pid>/stat"))?;

    let target_pid_ns = match openat2_class_b(pid_dir.0, "ns", true) {
        Ok(pid_ns_dir) => read_ns_magic_id(pid_ns_dir.0, "pid", "pidns"),
        Err(libc::ENOSYS) => return Err(map_class_b_observer_err(libc::ENOSYS, "<pid>/ns")),
        Err(e) if e == libc::EXDEV || e == libc::ELOOP => {
            return Err(map_class_b_observer_err(e, "<pid>/ns"))
        }
        Err(e) if e == libc::EPERM || e == libc::EACCES => {
            NsProbe::Inaccessible("ptrace-may-access-denied")
        }
        Err(libc::ENOENT) => NsProbe::NotPresent("namespace-dir-absent"),
        Err(_) => NsProbe::Error("namespace-dir-open-failed"),
    };

    let uids = smap.get("Uid").and_then(|v| parse_quad_u32(v));
    let gids = smap.get("Gid").and_then(|v| parse_quad_u32(v));

    Ok(TargetSnapshot::Observed {
        incarnation: LinuxIncarnation {
            anchor,
            target_pid_ns,
            tgid,
            start_ticks,
            state,
            uids,
            gids,
        },
        pid_dir,
        status_text,
    })
}

fn evaluate_linux_principal(inc: &LinuxIncarnation) -> Value {
    let (Some(uids), Some(gids)) = (inc.uids, inc.gids) else {
        return fact_non_observed("error", "malformed-procfs-status-uidgid");
    };
    let userns_scope: &str = match &inc.anchor.user_ns {
        NsProbe::Observed(v) => {
            let Some(ref map_res) = inc.anchor.id_mapping else {
                return fact_non_observed("error", "missing-userns-id-map");
            };
            let mapping = match map_res {
                Ok(m) => m,
                Err((status, reason)) => return fact_non_observed(status, reason),
            };
            if !uids.iter().all(|&u| {
                linux_id_is_unambiguously_mapped(u, &mapping.uid_extents, mapping.overflow_uid)
            }) || !gids.iter().all(|&g| {
                linux_id_is_unambiguously_mapped(g, &mapping.gid_extents, mapping.overflow_gid)
            }) {
                return fact_non_observed("unknown", "unmapped-or-overflow-id");
            }
            v.as_str()
        }
        NsProbe::NotPresent(_) => {
            if uids.contains(&u32::MAX) || gids.contains(&u32::MAX) {
                return fact_non_observed("unknown", "unmapped-or-overflow-id");
            }
            "userns:initial"
        }
        NsProbe::Inaccessible(r) => return fact_non_observed("inaccessible", r),
        NsProbe::Error(r) => return fact_non_observed("error", r),
    };

    let ns = format!("urn:3leaps:posix-credentials:linux-procfs-status:v0:{userns_scope}");
    let val = format!(
        "ruid={};euid={};suid={};fsuid={};rgid={};egid={};sgid={};fsgid={}",
        uids[0], uids[1], uids[2], uids[3], gids[0], gids[1], gids[2], gids[3]
    );
    principal_observed(LINUX_PRINCIPAL_DERIVATION_ID, ns, val)
}

fn probe_linux_arguments_digest(pid_dir_fd: c_int) -> Result<Value, CollectorRefusal> {
    let fd = match openat2_class_b(pid_dir_fd, "cmdline", false) {
        Ok(f) => f,
        Err(libc::ENOSYS) => return Err(map_class_b_observer_err(libc::ENOSYS, "<pid>/cmdline")),
        Err(e) if e == libc::EXDEV || e == libc::ELOOP => {
            return Err(map_class_b_observer_err(e, "<pid>/cmdline"))
        }
        Err(e) if e == libc::EPERM || e == libc::EACCES => {
            return Ok(fact_non_observed("inaccessible", "procfs-cmdline-denied"));
        }
        Err(e) if e == libc::ENOENT || e == libc::ESRCH => {
            return Ok(fact_non_observed("not-present", "empty-cmdline"));
        }
        Err(_) => return Ok(fact_non_observed("error", "procfs-cmdline-open-failed")),
    };

    match read_fd_bytes_capped(fd.0, MAX_ARGUMENTS_RAW_BYTES) {
        Ok(raw) if raw.is_empty() => Ok(fact_non_observed("not-present", "empty-cmdline")),
        Ok(raw) => {
            let argv = super::parse_linux_cmdline_argv(&raw);
            Ok(digest_observed(
                ARGV_DIGEST_DERIVATION_ID,
                derive_arguments_digest(&argv),
            ))
        }
        Err((libc::E2BIG, _)) => Ok(fact_non_observed("error", "arguments-exceed-cap")),
        Err((e, _)) if e == libc::EPERM || e == libc::EACCES => {
            Ok(fact_non_observed("inaccessible", "procfs-cmdline-denied"))
        }
        Err((libc::ESRCH, _)) => Ok(fact_non_observed("not-present", "empty-cmdline")),
        Err((_, r)) => Ok(fact_non_observed("error", r)),
    }
}

fn probe_linux_executable_digest(pid_dir_fd: c_int) -> Value {
    match openat_class_c_single(pid_dir_fd, "exe") {
        Ok(exe_fd) => stream_executable_digest_from_fd(exe_fd.0),
        Err(errno) => errno_to_digest_fact(errno, "procfs-exe-open-failed"),
    }
}

fn probe_linux_namespace_context_digest(pid_dir_fd: c_int) -> Result<Value, CollectorRefusal> {
    let pid_ns_dir = match openat2_class_b(pid_dir_fd, "ns", true) {
        Ok(fd) => fd,
        Err(libc::ENOSYS) => return Err(map_class_b_observer_err(libc::ENOSYS, "<pid>/ns")),
        Err(e) if e == libc::EXDEV || e == libc::ELOOP => {
            return Err(map_class_b_observer_err(e, "<pid>/ns"))
        }
        Err(e) if e == libc::EPERM || e == libc::EACCES => {
            return Ok(fact_non_observed(
                "inaccessible",
                "ptrace-may-access-denied",
            ))
        }
        Err(libc::ENOENT) => return Ok(fact_non_observed("unsupported", "namespace-dir-absent")),
        Err(_) => return Ok(fact_non_observed("error", "namespace-dir-open-failed")),
    };

    let mut ns_map = BTreeMap::new();
    for &kind in LINUX_NAMESPACE_KINDS {
        match read_ns_magic_id(pid_ns_dir.0, kind, "ns") {
            NsProbe::Observed(v) => {
                ns_map.insert(kind.to_string(), v);
            }
            NsProbe::Inaccessible(r) => return Ok(fact_non_observed("inaccessible", r)),
            NsProbe::NotPresent(_) => {
                return Ok(fact_non_observed(
                    "unsupported",
                    "kernel-missing-namespace-member",
                ))
            }
            NsProbe::Error(r) => return Ok(fact_non_observed("error", r)),
        }
    }

    match derive_linux_namespace_context_digest(&ns_map) {
        Ok(digest) => Ok(digest_observed(LINUX_NS_DIGEST_DERIVATION_ID, digest)),
        Err(_) => Ok(fact_non_observed("error", "invalid-linux-ns-digest-input")),
    }
}

fn probe_linux_security_context_digest(inc: &LinuxIncarnation, status_text: &str) -> Value {
    let userns_scope: &str = match &inc.anchor.user_ns {
        NsProbe::Observed(v) => v.as_str(),
        NsProbe::NotPresent(_) => "userns:initial",
        NsProbe::Inaccessible(r) => return fact_non_observed("inaccessible", r),
        NsProbe::Error(r) => return fact_non_observed("error", r),
    };

    let smap = status_line_map(status_text);
    let required_keys = [
        "Groups",
        "NoNewPrivs",
        "Seccomp",
        "CapInh",
        "CapPrm",
        "CapEff",
        "CapBnd",
        "CapAmb",
    ];
    for k in required_keys {
        if !smap.contains_key(k) {
            return fact_non_observed("unsupported", "missing-procfs-status-secctx-field");
        }
    }

    let mut gids: Vec<u32> = Vec::new();
    let groups_raw = smap["Groups"];
    if !groups_raw.is_empty() {
        for tok in groups_raw.split_whitespace() {
            let Some(gid) = tok.parse::<u32>().ok() else {
                return fact_non_observed("error", "malformed-procfs-groups");
            };
            if matches!(inc.anchor.user_ns, NsProbe::Observed(_)) {
                let Some(ref map_res) = inc.anchor.id_mapping else {
                    return fact_non_observed("error", "missing-userns-id-map");
                };
                let mapping = match map_res {
                    Ok(m) => m,
                    Err((st, r)) => return fact_non_observed(st, r),
                };
                if !linux_id_is_unambiguously_mapped(
                    gid,
                    &mapping.gid_extents,
                    mapping.overflow_gid,
                ) {
                    return fact_non_observed("unknown", "unmapped-or-overflow-gid");
                }
            } else if gid == u32::MAX {
                return fact_non_observed("unknown", "unmapped-or-overflow-gid");
            }
            gids.push(gid);
        }
    }
    gids.sort_unstable();
    gids.dedup();
    let groups_csv = gids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");

    let nnp = smap["NoNewPrivs"];
    if nnp != "0" && nnp != "1" {
        return fact_non_observed("error", "malformed-procfs-nonewprivs");
    }
    let Some(seccomp_u32) = smap["Seccomp"].parse::<u32>().ok() else {
        return fact_non_observed("error", "malformed-procfs-seccomp");
    };
    let seccomp = seccomp_u32.to_string();

    let canonical = format!(
        "observer_user_ns={userns_scope};groups={groups_csv};no_new_privs={nnp};seccomp={seccomp};cap_inh={};cap_prm={};cap_eff={};cap_bnd={};cap_amb={}",
        smap["CapInh"], smap["CapPrm"], smap["CapEff"], smap["CapBnd"], smap["CapAmb"]
    );
    match derive_linux_security_context_digest(canonical.as_bytes()) {
        Ok(digest) => digest_observed(LINUX_SECCTX_DIGEST_DERIVATION_ID, digest),
        Err(_) => fact_non_observed("error", "malformed-procfs-capabilities"),
    }
}

#[allow(clippy::too_many_arguments)]
fn assemble_linux_payload(
    pid: u32,
    host_scope_json: Value,
    observed_at: &str,
    anchor: &ObserverAnchor,
    target_pid_ns_fact: Value,
    process_state_fact: Value,
    start_identity_fact: Value,
    security_principal_fact: Value,
    ns_ctx_fact: Value,
    exe_fact: Value,
    args_fact: Value,
    secctx_fact: Value,
    options: &ObservationOptions,
) -> Value {
    json!({
        "profile": {
            "id": NATIVE_PROCESS_OBSERVATION_PROFILE_ID,
            "revision": NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
        },
        "observed_at": observed_at,
        "observer": {
            "platform_family": "linux",
            "architecture": observer_architecture(),
            "bracketed": true,
            "time_namespace": anchor.time_ns.to_fact(),
            "user_namespace": anchor.user_ns.to_fact(),
        },
        "process": {
            "subject_ref": options.subject_ref,
            "host_scope": host_scope_json,
            "boot_id": fact_observed(anchor.boot_id.clone()),
            "pid_numbering_namespace": fact_observed(anchor.pid_numbering_ns.clone()),
            "target_pid_namespace": target_pid_ns_fact,
            "pid": pid.to_string(),
            "process_state": process_state_fact,
            "start_identity": start_identity_fact,
            "execution_context": {
                "security_principal": security_principal_fact,
                "namespace_context_digest": ns_ctx_fact,
                "executable_digest": exe_fact,
                "arguments_digest": args_fact,
                "security_context_digest": secctx_fact,
            }
        }
    })
}

fn non_observed_context_facts(
    status: &str,
    reason: &str,
    options: &ObservationOptions,
) -> (Value, Value, Value, Value, Value) {
    let sp = if options.probe_security_principal {
        fact_non_observed(status, reason)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };
    let ns = if options.include_namespace_context_digest {
        fact_non_observed(status, reason)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };
    let exe = if options.include_executable_digest {
        fact_non_observed(status, reason)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };
    let args = if options.include_arguments_digest {
        fact_non_observed(status, reason)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };
    let sec = if options.include_security_context_digest {
        fact_non_observed(status, reason)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };
    (sp, ns, exe, args, sec)
}

pub(super) fn observe_process_impl(
    pid: u32,
    host_scope_json: Value,
    observed_at: &str,
    options: &ObservationOptions,
) -> Result<Value, CollectorRefusal> {
    let (proc_fd, proc_maj, proc_min) = open_verified_proc_root()?;
    let need_id_maps = options.probe_security_principal || options.include_security_context_digest;

    for attempt in 0..2 {
        let s0 = capture_target_snapshot(proc_fd.0, proc_maj, proc_min, pid, need_id_maps)?;
        match s0 {
            TargetSnapshot::NotPresent { anchor, reason } => {
                let anchor_1 = collect_observer_anchor(proc_fd.0, proc_maj, proc_min, false)?;
                if anchor.boot_id != anchor_1.boot_id
                    || anchor.pid_numbering_ns != anchor_1.pid_numbering_ns
                    || anchor.time_ns != anchor_1.time_ns
                    || anchor.user_ns != anchor_1.user_ns
                {
                    if attempt == 0 {
                        continue;
                    }
                    return Err(CollectorRefusal::error(
                        "unbracketed-incarnation",
                        "observer anchor changed during absent-process observation",
                    ));
                }
                let (sp, ns, exe, args, sec) =
                    non_observed_context_facts("not-present", reason, options);
                return Ok(assemble_linux_payload(
                    pid,
                    host_scope_json,
                    observed_at,
                    &anchor,
                    fact_non_observed("not-present", reason),
                    fact_non_observed("not-present", reason),
                    start_identity_non_observed(
                        LINUX_START_IDENTITY_PROFILE_ID,
                        "not-present",
                        reason,
                    ),
                    sp,
                    ns,
                    exe,
                    args,
                    sec,
                    options,
                ));
            }
            TargetSnapshot::Unknown { anchor, reason } => {
                let (sp, ns, exe, args, sec) =
                    non_observed_context_facts("unknown", reason, options);
                return Ok(assemble_linux_payload(
                    pid,
                    host_scope_json,
                    observed_at,
                    &anchor,
                    fact_non_observed("unknown", reason),
                    fact_non_observed("unknown", reason),
                    start_identity_non_observed(LINUX_START_IDENTITY_PROFILE_ID, "unknown", reason),
                    sp,
                    ns,
                    exe,
                    args,
                    sec,
                    options,
                ));
            }
            TargetSnapshot::Inaccessible { anchor, reason } => {
                let (sp, ns, exe, args, sec) =
                    non_observed_context_facts("inaccessible", reason, options);
                return Ok(assemble_linux_payload(
                    pid,
                    host_scope_json,
                    observed_at,
                    &anchor,
                    fact_non_observed("inaccessible", reason),
                    fact_non_observed("inaccessible", reason),
                    start_identity_non_observed(
                        LINUX_START_IDENTITY_PROFILE_ID,
                        "inaccessible",
                        reason,
                    ),
                    sp,
                    ns,
                    exe,
                    args,
                    sec,
                    options,
                ));
            }
            TargetSnapshot::Observed {
                incarnation: inc0,
                pid_dir: pid_dir0,
                status_text: status0,
            } => {
                let ns_ctx_fact = if options.include_namespace_context_digest {
                    probe_linux_namespace_context_digest(pid_dir0.0)?
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };
                let exe_fact = if options.include_executable_digest {
                    probe_linux_executable_digest(pid_dir0.0)
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };
                let args_fact = if options.include_arguments_digest {
                    probe_linux_arguments_digest(pid_dir0.0)?
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };
                let secctx_0 = if options.include_security_context_digest {
                    Some(probe_linux_security_context_digest(&inc0, &status0))
                } else {
                    None
                };

                drop(pid_dir0);

                let s1 = capture_target_snapshot(proc_fd.0, proc_maj, proc_min, pid, need_id_maps)?;
                let TargetSnapshot::Observed {
                    incarnation: inc1,
                    status_text: status1,
                    ..
                } = s1
                else {
                    if attempt == 0 {
                        continue;
                    }
                    return Err(CollectorRefusal::error(
                        "unbracketed-incarnation",
                        "target process vanished during observation bracket",
                    ));
                };

                if inc0 != inc1 {
                    if attempt == 0 {
                        continue;
                    }
                    return Err(CollectorRefusal::error(
                        "unbracketed-incarnation",
                        "Linux incarnation or credentials changed between pre/post snapshots",
                    ));
                }

                let secctx_fact = match secctx_0 {
                    Some(f0) => {
                        let f1 = probe_linux_security_context_digest(&inc1, &status1);
                        if f0 != f1 {
                            if attempt == 0 {
                                continue;
                            }
                            return Err(CollectorRefusal::error(
                                "unbracketed-incarnation",
                                "security_context_digest changed between pre/post snapshots",
                            ));
                        }
                        f0
                    }
                    None => fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON),
                };

                let sp_fact = if options.probe_security_principal {
                    evaluate_linux_principal(&inc0)
                } else {
                    fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
                };

                return Ok(assemble_linux_payload(
                    pid,
                    host_scope_json,
                    observed_at,
                    &inc0.anchor,
                    inc0.target_pid_ns.to_fact(),
                    fact_observed(inc0.state.as_str()),
                    start_identity_observed(LINUX_START_IDENTITY_PROFILE_ID, inc0.start_ticks),
                    sp_fact,
                    ns_ctx_fact,
                    exe_fact,
                    args_fact,
                    secctx_fact,
                    options,
                ));
            }
        }
    }

    Err(CollectorRefusal::error(
        "unbracketed-incarnation",
        "failed to bracket Linux process observation",
    ))
}

pub(super) fn open_continuity_handle_impl(
    pid: u32,
    host_scope_json: Value,
    observed_at: &str,
    options: &ObservationOptions,
) -> Result<(Value, PlatformHandle), CollectorRefusal> {
    let (proc_fd, proc_maj, proc_min) = open_verified_proc_root()?;
    let need_id_maps = options.probe_security_principal || options.include_security_context_digest;

    let s0 = capture_target_snapshot(proc_fd.0, proc_maj, proc_min, pid, need_id_maps)?;
    let (inc0, pid_dir0, status0) = match s0 {
        TargetSnapshot::Observed {
            incarnation,
            pid_dir,
            status_text,
        } => (incarnation, pid_dir, status_text),
        TargetSnapshot::NotPresent { reason, .. } => {
            return Err(CollectorRefusal::not_present("target-not-present", reason))
        }
        TargetSnapshot::Unknown { reason, .. } => {
            return Err(CollectorRefusal::new(
                "unknown",
                "target-visibility-unknown",
                reason,
            ))
        }
        TargetSnapshot::Inaccessible { reason, .. } => {
            return Err(CollectorRefusal::inaccessible(
                reason,
                "target PID is inaccessible",
            ))
        }
    };

    if inc0.state != ProcState::Live {
        return Err(CollectorRefusal::error(
            "target-not-live",
            "continuity handle requires a live target process (not zombie)",
        ));
    }

    let raw_pidfd = loop {
        let rc = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as c_int, 0 as c_uint) };
        if rc >= 0 {
            break rc as c_int;
        }
        let errno = last_errno();
        if errno == libc::EINTR {
            continue;
        }
        return Err(match errno {
            libc::ENOSYS => CollectorRefusal::unsupported(
                "pidfd-unavailable",
                "Linux kernel does not support pidfd_open(2)",
            ),
            libc::ESRCH => CollectorRefusal::not_present(
                "target-not-present",
                "target PID exited before pidfd_open",
            ),
            libc::EPERM | libc::EACCES => {
                CollectorRefusal::inaccessible("pidfd-open-denied", "kernel denied pidfd_open")
            }
            _ => CollectorRefusal::error(
                "pidfd-open-failed",
                format!("pidfd_open failed errno={errno}"),
            ),
        });
    };
    let pidfd_guard = OwnedFd(raw_pidfd);
    let cloexec_rc = unsafe { libc::fcntl(pidfd_guard.0, libc::F_SETFD, libc::FD_CLOEXEC) };
    if cloexec_rc < 0 {
        return Err(CollectorRefusal::error(
            "pidfd-cloexec-failed",
            format!("fcntl(FD_CLOEXEC) failed errno={}", last_errno()),
        ));
    }

    let ns_ctx_fact = if options.include_namespace_context_digest {
        probe_linux_namespace_context_digest(pid_dir0.0)?
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };
    let exe_fact = if options.include_executable_digest {
        probe_linux_executable_digest(pid_dir0.0)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };
    let args_fact = if options.include_arguments_digest {
        probe_linux_arguments_digest(pid_dir0.0)?
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };
    let secctx_0 = if options.include_security_context_digest {
        Some(probe_linux_security_context_digest(&inc0, &status0))
    } else {
        None
    };

    drop(pid_dir0);

    let s1 = capture_target_snapshot(proc_fd.0, proc_maj, proc_min, pid, need_id_maps)?;
    let TargetSnapshot::Observed {
        incarnation: inc1,
        status_text: status1,
        ..
    } = s1
    else {
        return Err(CollectorRefusal::error(
            "unbracketed-incarnation",
            "target process vanished during pidfd registration",
        ));
    };

    if inc0 != inc1 || inc1.state != ProcState::Live {
        return Err(CollectorRefusal::error(
            "unbracketed-incarnation",
            "incarnation changed during pidfd registration",
        ));
    }

    let secctx_fact = match secctx_0 {
        Some(f0) => {
            let f1 = probe_linux_security_context_digest(&inc1, &status1);
            if f0 != f1 {
                return Err(CollectorRefusal::error(
                    "unbracketed-incarnation",
                    "security_context_digest changed during pidfd registration",
                ));
            }
            f0
        }
        None => fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON),
    };

    let sp_fact = if options.probe_security_principal {
        evaluate_linux_principal(&inc0)
    } else {
        fact_non_observed("not-probed", DEFAULT_NOT_PROBED_REASON)
    };

    let baseline = assemble_linux_payload(
        pid,
        host_scope_json,
        observed_at,
        &inc0.anchor,
        inc0.target_pid_ns.to_fact(),
        fact_observed("live"),
        start_identity_observed(LINUX_START_IDENTITY_PROFILE_ID, inc0.start_ticks),
        sp_fact,
        ns_ctx_fact,
        exe_fact,
        args_fact,
        secctx_fact,
        options,
    );

    Ok((
        baseline,
        PlatformHandle {
            pidfd: pidfd_guard.into_raw(),
            exited: AtomicBool::new(false),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proc_stat_handles_spaces_parens_and_newlines_in_comm() {
        let stat = "4242 (weird (comm)\nname ) S 1 4242 4242 0 -1 4194304 10 0 0 0 1 2 0 0 20 0 1 0 987654321 12345 10 18446744073709551615\n";
        let (st, ticks) = parse_proc_stat(stat).expect("valid stat");
        assert_eq!(st, ProcState::Live);
        assert_eq!(ticks, "987654321");

        let zstat = "4242 (z) Z 1 4242 4242 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0\n";
        let (zst, zticks) = parse_proc_stat(zstat).expect("valid zombie stat");
        assert_eq!(zst, ProcState::Zombie);
        assert_eq!(zticks, "0");
    }

    #[test]
    fn mountinfo_detects_hidepid_invisibility_and_subset_pid() {
        let normal =
            "22 28 0:21 / /proc rw,nosuid,nodev,noexec,relatime shared:12 - proc proc rw\n";
        assert!(!mountinfo_conceals_pids(normal, 0, 21));

        let noaccess = "22 28 0:21 / /proc rw,relatime - proc proc rw,hidepid=noaccess\n";
        assert!(!mountinfo_conceals_pids(noaccess, 0, 21));

        let invisible = "22 28 0:21 / /proc rw,relatime - proc proc rw,hidepid=invisible\n";
        assert!(mountinfo_conceals_pids(invisible, 0, 21));

        let hidepid2 = "22 28 0:21 / /proc rw,relatime - proc proc rw,hidepid=2\n";
        assert!(mountinfo_conceals_pids(hidepid2, 0, 21));

        let subset = "22 28 0:21 / /proc rw,relatime - proc proc rw,subset=pid\n";
        assert!(mountinfo_conceals_pids(subset, 0, 21));
    }
}
