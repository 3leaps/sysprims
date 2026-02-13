//! macOS implementation using libproc and sysctl
//!
//! Uses the following APIs:
//! - `proc_listpids()` - enumerate all PIDs
//! - `proc_pidinfo()` with `PROC_PIDTBSDINFO` - process info (name, ppid, state, user)
//! - `proc_pidinfo()` with `PROC_PIDTASKINFO` - resource info (CPU, memory)
//! - `proc_name()` - get process name
//! - `mach_timebase_info()` - convert Mach time units to nanoseconds
//! - `sysctl(CTL_KERN, KERN_PROCARGS2)` - read process command-line arguments

use crate::{
    aggregate_error_warning, aggregate_permission_warning, make_port_snapshot, make_snapshot,
    FdInfo, FdKind, PortBinding, PortBindingsSnapshot, ProcessInfo, ProcessSnapshot, ProcessState,
    Protocol,
};
use libc::{c_int, c_void, pid_t, uid_t};
use std::ffi::CStr;
use std::mem;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;
use std::thread;
use std::time::Instant;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysprims_core::{SysprimsError, SysprimsResult};

// ============================================================================
// libproc FFI Bindings
// ============================================================================

// Constants from <libproc.h>
const PROC_ALL_PIDS: u32 = 1;
const PROC_PIDTBSDINFO: c_int = 3;
const PROC_PIDTASKINFO: c_int = 4;
const MAXCOMLEN: usize = 16;
const MAXPATHLEN: usize = 1024;

const PROC_PIDLISTFDS: c_int = 1;
const PROC_PIDFDVNODEPATHINFO: c_int = 2;
const PROC_PIDFDSOCKETINFO: c_int = 3;
const PROX_FDTYPE_VNODE: u32 = 1;
const PROX_FDTYPE_SOCKET: u32 = 2;
const PROX_FDTYPE_PIPE: u32 = 6;

const SOCKINFO_IN: i32 = 1;
const SOCKINFO_TCP: i32 = 2;

const INI_IPV4: u8 = 0x1;
const INI_IPV6: u8 = 0x2;

const TSI_S_LISTEN: i32 = 1;

// Process status values from <sys/proc.h>
const SIDL: u32 = 1; // Process being created
const SRUN: u32 = 2; // Currently runnable
const SSLEEP: u32 = 3; // Sleeping on an address
const SSTOP: u32 = 4; // Process is stopped
const SZOMB: u32 = 5; // Process is a zombie

/// BSD info structure returned by proc_pidinfo with PROC_PIDTBSDINFO
#[repr(C)]
#[derive(Debug, Default)]
struct ProcBsdInfo {
    pbi_flags: u32,
    pbi_status: u32,
    pbi_xstatus: u32,
    pbi_pid: u32,
    pbi_ppid: u32,
    pbi_uid: uid_t,
    pbi_gid: u32,
    pbi_ruid: uid_t,
    pbi_rgid: u32,
    pbi_svuid: uid_t,
    pbi_svgid: u32,
    _rfu_1: u32,
    pbi_comm: [u8; MAXCOMLEN],
    pbi_name: [u8; 2 * MAXCOMLEN],
    pbi_nfiles: u32,
    pbi_pgid: u32,
    pbi_pjobc: u32,
    e_tdev: u32,
    e_tpgid: u32,
    pbi_nice: i32,
    pbi_start_tvsec: u64,
    pbi_start_tvusec: u64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct ProcFdInfo {
    proc_fd: i32,
    proc_fdtype: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct In4In6Addr {
    i46a_pad32: [u32; 3],
    i46a_addr4: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InSockInfo {
    insi_fport: i32,
    insi_lport: i32,
    insi_gencnt: u64,
    insi_flags: u32,
    insi_flow: u32,
    insi_vflag: u8,
    insi_ip_ttl: u8,
    rfu_1: u32,
    insi_faddr: InSockAddr,
    insi_laddr: InSockAddr,
    insi_v4: InSockV4,
    insi_v6: InSockV6,
}

#[repr(C)]
#[derive(Clone, Copy)]
union InSockAddr {
    ina_46: In4In6Addr,
    ina_6: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InSockV4 {
    in4_tos: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InSockV6 {
    in6_hlim: u8,
    in6_cksum: i32,
    in6_ifindex: u16,
    in6_hops: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TcpSockInfo {
    tcpsi_ini: InSockInfo,
    tcpsi_state: i32,
    tcpsi_timer: [i32; 4],
    tcpsi_mss: i32,
    tcpsi_flags: u32,
    rfu_1: u32,
    tcpsi_tp: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SockbufInfo {
    sbi_cc: u32,
    sbi_hiwat: u32,
    sbi_mbcnt: u32,
    sbi_mbmax: u32,
    sbi_lowat: u32,
    sbi_flags: i16,
    sbi_timeo: i16,
}

// NOTE: We intentionally do not model socket_fdinfo/socket_info as Rust structs
// for parsing. See read_socket_binding() for offset-based parsing.

const PROC_FILEINFO_SIZE: usize = 24;

fn compute_socket_info_offsets(vinfo_stat_size: usize) -> (usize, usize, usize) {
    // Layout based on sys/proc_info.h:
    // struct socket_fdinfo { struct proc_fileinfo pfi; struct socket_info psi; };
    // struct socket_info { struct vinfo_stat soi_stat; ... int soi_kind; uint32_t rfu_1; union soi_proto; }

    let base = PROC_FILEINFO_SIZE;
    let mut off = base + vinfo_stat_size;

    off += 8; // soi_so
    off += 8; // soi_pcb

    off += 4; // soi_type
    let soi_protocol_off = off;
    off += 4; // soi_protocol

    off += 4; // soi_family

    off += 2; // soi_options
    off += 2; // soi_linger
    off += 2; // soi_state
    off += 2; // soi_qlen
    off += 2; // soi_incqlen
    off += 2; // soi_qlimit
    off += 2; // soi_timeo
    off += 2; // soi_error

    off += 4; // soi_oobmark
    off += mem::size_of::<SockbufInfo>(); // soi_rcv
    off += mem::size_of::<SockbufInfo>(); // soi_snd

    let soi_kind_off = off;
    off += 4; // soi_kind
    off += 4; // rfu_1
    let soi_proto_off = off;

    (soi_protocol_off, soi_kind_off, soi_proto_off)
}

fn read_i32_at(buf: &[u8], offset: usize) -> Option<i32> {
    if buf.len() < offset + 4 {
        return None;
    }
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&buf[offset..offset + 4]);
    Some(i32::from_ne_bytes(bytes))
}

fn select_socket_info_layout(buf: &[u8]) -> Option<(usize, usize, usize)> {
    // vinfo_stat size varies across SDKs. Try common candidates.
    // Validate by checking that derived soi_kind and soi_protocol look plausible.
    let candidates = [136usize, 144usize];

    for stat_size in candidates {
        let (proto_off, kind_off, proto_union_off) = compute_socket_info_offsets(stat_size);
        let kind = read_i32_at(buf, kind_off)?;
        if !(0..=7).contains(&kind) {
            continue;
        }

        let proto = read_i32_at(buf, proto_off).unwrap_or(0);
        let proto_ok = proto == libc::IPPROTO_TCP || proto == libc::IPPROTO_UDP || proto == 0;
        let kind_ok = kind == SOCKINFO_TCP || kind == SOCKINFO_IN || kind == 0;
        if proto_ok && kind_ok {
            return Some((proto_off, kind_off, proto_union_off));
        }
    }

    None
}

/// Task info structure returned by proc_pidinfo with PROC_PIDTASKINFO
#[repr(C)]
#[derive(Debug, Default)]
struct ProcTaskInfo {
    pti_virtual_size: u64,
    pti_resident_size: u64,
    pti_total_user: u64,
    pti_total_system: u64,
    pti_threads_user: u64,
    pti_threads_system: u64,
    pti_policy: i32,
    pti_faults: i32,
    pti_pageins: i32,
    pti_cow_faults: i32,
    pti_messages_sent: i32,
    pti_messages_received: i32,
    pti_syscalls_mach: i32,
    pti_syscalls_unix: i32,
    pti_csw: i32,
    pti_threadnum: i32,
    pti_numrunning: i32,
    pti_priority: i32,
}

extern "C" {
    fn proc_listpids(type_: u32, typeinfo: u32, buffer: *mut c_void, buffersize: c_int) -> c_int;

    fn proc_pidinfo(
        pid: c_int,
        flavor: c_int,
        arg: u64,
        buffer: *mut c_void,
        buffersize: c_int,
    ) -> c_int;

    fn proc_pidfdinfo(
        pid: c_int,
        fd: c_int,
        flavor: c_int,
        buffer: *mut c_void,
        buffersize: c_int,
    ) -> c_int;

    fn proc_name(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;

    fn proc_pidpath(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;

    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> c_int;
}

/// Mach timebase info for converting Mach time units to nanoseconds.
/// On Apple Silicon, numer/denom is typically 125/3 (~41.67x).
/// On Intel Macs, it's often 1/1.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

/// Cached Mach timebase conversion factor (numer/denom).
/// Computed once on first use.
static MACH_TIMEBASE_FACTOR: OnceLock<f64> = OnceLock::new();

/// Get the Mach timebase conversion factor for converting Mach time to nanoseconds.
fn mach_to_ns_factor() -> f64 {
    *MACH_TIMEBASE_FACTOR.get_or_init(|| {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        let ret = unsafe { mach_timebase_info(&mut info) };
        if ret != 0 || info.denom == 0 {
            // Fallback to 1:1 if we can't get timebase info
            1.0
        } else {
            info.numer as f64 / info.denom as f64
        }
    })
}

/// Convert Mach time units to nanoseconds.
#[inline]
fn mach_time_to_ns(mach_time: u64) -> u64 {
    (mach_time as f64 * mach_to_ns_factor()) as u64
}

// ============================================================================
// Implementation
// ============================================================================

pub fn snapshot_impl() -> SysprimsResult<ProcessSnapshot> {
    let pids = list_all_pids()?;
    let mut processes = Vec::with_capacity(pids.len());

    for pid in pids {
        if pid <= 0 {
            continue;
        }
        // Silently skip processes we can't read
        if let Ok(info) = read_process_info(pid as u32) {
            processes.push(info);
        }
    }

    Ok(make_snapshot(processes))
}

pub fn get_process_impl(pid: u32) -> SysprimsResult<ProcessInfo> {
    read_process_info(pid)
}

pub fn wait_pid_impl(pid: u32, timeout: Duration) -> SysprimsResult<crate::WaitPidResult> {
    let start = Instant::now();
    let mut first_check = true;

    loop {
        // SAFETY: kill(pid, 0) does not send a signal; it performs an existence/permission check.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            // Treat zombies as exited (kill(pid, 0) still succeeds for zombies).
            if let Ok(info) = read_process_info(pid) {
                if info.state == crate::ProcessState::Zombie {
                    return Ok(crate::make_wait_pid_result(pid, true, false, None, vec![]));
                }
            }
            if start.elapsed() >= timeout {
                return Ok(crate::make_wait_pid_result(pid, false, true, None, vec![]));
            }
            thread::sleep(Duration::from_millis(25));
            first_check = false;
            continue;
        }

        let errno = unsafe { *libc::__error() };
        if errno == libc::ESRCH {
            if first_check {
                return Err(SysprimsError::not_found(pid));
            }
            return Ok(crate::make_wait_pid_result(pid, true, false, None, vec![]));
        }
        if errno == libc::EPERM {
            return Err(SysprimsError::permission_denied(pid, "wait pid"));
        }

        return Err(SysprimsError::system("kill(pid, 0) failed", errno));
    }
}

pub fn listening_ports_impl() -> SysprimsResult<PortBindingsSnapshot> {
    let pids = list_all_pids()?;
    let mut bindings = Vec::new();
    let mut permission_denied = 0usize;
    let mut read_errors = 0usize;
    let mut skipped_other_user = 0usize;
    let mut socket_permission_denied = 0usize;
    let mut socket_read_errors = 0usize;
    let mut socket_unsupported = 0usize;
    let mut socket_missing_port = 0usize;

    let current_uid = unsafe { libc::geteuid() };

    for pid in pids {
        if pid <= 0 {
            continue;
        }

        // Filter to current UID to reduce SIP/TCC noise and improve performance.
        // This still includes "self" and is the most useful view for typical tooling.
        match get_bsd_info(pid as u32) {
            Ok(bsd) => {
                if bsd.pbi_uid != current_uid {
                    skipped_other_user += 1;
                    continue;
                }
            }
            Err(SysprimsError::PermissionDenied { .. }) => {
                permission_denied += 1;
                continue;
            }
            Err(_) => {
                read_errors += 1;
                continue;
            }
        }

        match list_socket_fds(pid) {
            Ok(fds) => {
                for fd in fds {
                    match read_socket_binding(pid, fd) {
                        Ok(binding) => bindings.push(binding),
                        Err(SysprimsError::PermissionDenied { .. }) => {
                            socket_permission_denied += 1
                        }
                        Err(SysprimsError::Internal { message }) => {
                            if message.contains("unsupported socket") {
                                socket_unsupported += 1;
                            } else if message.contains("no local port") {
                                socket_missing_port += 1;
                            } else {
                                socket_read_errors += 1;
                            }
                        }
                        Err(_) => socket_read_errors += 1,
                    }
                }
            }
            Err(err) => match err {
                SysprimsError::PermissionDenied { .. } => permission_denied += 1,
                _ => read_errors += 1,
            },
        }
    }

    let mut warnings = Vec::new();

    if skipped_other_user > 0 {
        warnings.push(format!(
            "macos port bindings are best-effort; scanning current user processes only (uid={})",
            current_uid
        ));
        warnings.push(format!(
            "Skipped {} pid entries owned by other users",
            skipped_other_user
        ));
    }

    if let Some(warning) = aggregate_permission_warning(permission_denied, "pid entries") {
        warnings.push(warning);
    }
    if let Some(warning) = aggregate_error_warning(read_errors, "pid entries") {
        warnings.push(warning);
    }

    if let Some(warning) = aggregate_permission_warning(socket_permission_denied, "socket entries")
    {
        warnings.push(warning);
    }
    if let Some(warning) = aggregate_error_warning(socket_read_errors, "socket entries") {
        warnings.push(warning);
    }
    if socket_unsupported > 0 {
        warnings.push(format!(
            "Skipped {} socket entries due to unsupported socket kinds",
            socket_unsupported
        ));
    }
    if socket_missing_port > 0 {
        warnings.push(format!(
            "Skipped {} socket entries with no local port",
            socket_missing_port
        ));
    }

    Ok(make_port_snapshot(bindings, warnings))
}

/// Get list of all PIDs on the system.
fn list_all_pids() -> SysprimsResult<Vec<pid_t>> {
    // First call to get required buffer size
    let buffer_size = unsafe { proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };

    if buffer_size <= 0 {
        return Err(SysprimsError::internal("proc_listpids failed to get size"));
    }

    let count = buffer_size as usize / mem::size_of::<pid_t>();
    let mut pids: Vec<pid_t> = vec![0; count];

    let actual = unsafe {
        proc_listpids(
            PROC_ALL_PIDS,
            0,
            pids.as_mut_ptr() as *mut c_void,
            buffer_size,
        )
    };

    if actual <= 0 {
        return Err(SysprimsError::internal("proc_listpids failed"));
    }

    // Trim to actual count
    let actual_count = actual as usize / mem::size_of::<pid_t>();
    pids.truncate(actual_count);

    Ok(pids)
}

fn list_socket_fds(pid: pid_t) -> SysprimsResult<Vec<i32>> {
    // libproc does not provide a reliable "size query" mode for PROC_PIDLISTFDS.
    // Instead, allocate a buffer and retry with growth if the result indicates truncation.
    let mut buffer_size: usize = 4096;
    let max_buffer_size: usize = 1024 * 1024;

    loop {
        let count = buffer_size / mem::size_of::<ProcFdInfo>();
        let mut fdinfo = vec![ProcFdInfo::default(); count];

        let actual = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDLISTFDS,
                0,
                fdinfo.as_mut_ptr() as *mut c_void,
                buffer_size as c_int,
            )
        };

        if actual <= 0 {
            let errno = unsafe { *libc::__error() };
            if errno == libc::EPERM || errno == libc::EACCES {
                return Err(SysprimsError::permission_denied(
                    pid as u32,
                    "list socket fds",
                ));
            }
            return Err(SysprimsError::internal("proc_pidinfo list fds failed"));
        }

        // proc_pidinfo returns the number of bytes written.
        let actual_bytes = actual as usize;
        if actual_bytes < buffer_size || buffer_size >= max_buffer_size {
            let actual_count = actual_bytes / mem::size_of::<ProcFdInfo>();
            fdinfo.truncate(actual_count);
            return Ok(fdinfo
                .into_iter()
                .filter(|info| info.proc_fdtype == PROX_FDTYPE_SOCKET)
                .map(|info| info.proc_fd)
                .collect());
        }

        buffer_size = (buffer_size * 2).min(max_buffer_size);
    }
}

fn list_all_fds(pid: pid_t) -> SysprimsResult<Vec<ProcFdInfo>> {
    // libproc does not provide a reliable "size query" mode for PROC_PIDLISTFDS.
    // Instead, allocate a buffer and retry with growth if the result indicates truncation.
    let mut buffer_size: usize = 4096;
    let max_buffer_size: usize = 1024 * 1024;

    loop {
        let count = buffer_size / mem::size_of::<ProcFdInfo>();
        let mut fdinfo = vec![ProcFdInfo::default(); count];

        let actual = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDLISTFDS,
                0,
                fdinfo.as_mut_ptr() as *mut c_void,
                buffer_size as c_int,
            )
        };

        if actual <= 0 {
            let errno = unsafe { *libc::__error() };
            if errno == libc::EPERM || errno == libc::EACCES {
                return Err(SysprimsError::permission_denied(pid as u32, "list fds"));
            }
            if errno == libc::ESRCH {
                return Err(SysprimsError::not_found(pid as u32));
            }
            return Err(SysprimsError::internal("proc_pidinfo list fds failed"));
        }

        // proc_pidinfo returns the number of bytes written.
        let actual_bytes = actual as usize;
        if actual_bytes < buffer_size || buffer_size >= max_buffer_size {
            let actual_count = actual_bytes / mem::size_of::<ProcFdInfo>();
            fdinfo.truncate(actual_count);
            return Ok(fdinfo);
        }

        buffer_size = (buffer_size * 2).min(max_buffer_size);
    }
}

fn read_vnode_fd_path(pid: pid_t, fd: i32) -> Option<String> {
    // Use PROC_PIDFDVNODEPATHINFO and extract the trailing MAXPATHLEN bytes.
    // proc_pidfdinfo returns the number of bytes written; the path is at the
    // end of the vnode_fdinfowithpath structure.
    let mut buffer_size: usize = 2048;
    let max_buffer_size: usize = 64 * 1024;

    loop {
        let mut buf = vec![0u8; buffer_size];
        let result = unsafe {
            proc_pidfdinfo(
                pid,
                fd,
                PROC_PIDFDVNODEPATHINFO,
                buf.as_mut_ptr() as *mut c_void,
                buffer_size as c_int,
            )
        };

        if result <= 0 {
            return None;
        }

        let written = result as usize;
        if written < buffer_size || buffer_size >= max_buffer_size {
            if written < MAXPATHLEN {
                return None;
            }

            let tail = &buf[written.saturating_sub(MAXPATHLEN)..written];
            let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
            return Some(String::from_utf8_lossy(&tail[..end]).into_owned());
        }

        buffer_size = (buffer_size * 2).min(max_buffer_size);
    }
}

pub fn list_fds_impl(pid: u32) -> SysprimsResult<(Vec<FdInfo>, Vec<String>)> {
    let pid = pid as pid_t;
    let infos = list_all_fds(pid)?;

    let mut fds = Vec::new();
    let mut path_missing = 0usize;

    for info in infos {
        if info.proc_fd < 0 {
            continue;
        }

        let fd_num = info.proc_fd;
        let fd = fd_num as u32;
        let kind = match info.proc_fdtype {
            PROX_FDTYPE_VNODE => FdKind::File,
            PROX_FDTYPE_SOCKET => FdKind::Socket,
            PROX_FDTYPE_PIPE => FdKind::Pipe,
            _ => FdKind::Unknown,
        };

        let path = if kind == FdKind::File {
            let p = read_vnode_fd_path(pid, fd_num);
            if p.is_none() {
                path_missing += 1;
            }
            p
        } else {
            None
        };

        fds.push(FdInfo { fd, kind, path });
    }

    fds.sort_by_key(|f| f.fd);

    let mut warnings = Vec::new();
    if path_missing > 0 {
        warnings.push(format!(
            "Failed to resolve paths for {} file fds",
            path_missing
        ));
    }

    Ok((fds, warnings))
}

fn read_socket_binding(pid: pid_t, fd: i32) -> SysprimsResult<PortBinding> {
    // Don't model the full socket_fdinfo union layout directly; it contains large
    // members (e.g. unix domain socket addresses) and an undersized model can
    // cause proc_pidfdinfo() to fail with EINVAL.
    //
    // Instead, request a generously sized buffer and parse the fixed prefix.
    let mut buf = [0u8; 2048];
    let size = buf.len() as c_int;
    let result = unsafe {
        proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDSOCKETINFO,
            buf.as_mut_ptr() as *mut c_void,
            size,
        )
    };

    if result <= 0 {
        let errno = unsafe { *libc::__error() };
        if errno == libc::EPERM || errno == libc::EACCES {
            return Err(SysprimsError::permission_denied(
                pid as u32,
                "read socket info",
            ));
        }
        return Err(SysprimsError::internal("proc_pidfdinfo socket failed"));
    }

    let written = result as usize;
    let (soi_protocol_off, soi_kind_off, soi_proto_off) =
        select_socket_info_layout(&buf[..written])
            .ok_or_else(|| SysprimsError::internal("unsupported socket_info layout"))?;

    let kind = read_i32_at(&buf[..written], soi_kind_off)
        .ok_or_else(|| SysprimsError::internal("socket kind missing"))?;

    // Determine protocol using socket_info.soi_protocol where possible.
    // This is more robust than relying solely on soi_kind.
    let soi_protocol = read_i32_at(&buf[..written], soi_protocol_off).unwrap_or(0);
    let protocol = match soi_protocol {
        x if x == libc::IPPROTO_TCP => Protocol::Tcp,
        x if x == libc::IPPROTO_UDP => Protocol::Udp,
        _ => match kind {
            SOCKINFO_TCP => Protocol::Tcp,
            SOCKINFO_IN => Protocol::Udp,
            _ => {
                return Err(SysprimsError::internal(
                    "unsupported socket kind for port bindings",
                ))
            }
        },
    };

    if written < soi_proto_off + mem::size_of::<InSockInfo>() {
        return Err(SysprimsError::internal("socket proto truncated"));
    }
    let proto_ptr = unsafe { buf.as_ptr().add(soi_proto_off) };

    let (local_addr, local_port) = unsafe {
        match kind {
            SOCKINFO_TCP => {
                if written < soi_proto_off + mem::size_of::<TcpSockInfo>() {
                    return Err(SysprimsError::internal("tcp sockinfo truncated"));
                }
                let tcp: TcpSockInfo = std::ptr::read_unaligned(proto_ptr as *const TcpSockInfo);
                read_tcp_binding(&tcp)
            }
            SOCKINFO_IN => {
                let inet: InSockInfo = std::ptr::read_unaligned(proto_ptr as *const InSockInfo);
                read_in_binding(&inet)
            }
            _ => Err(SysprimsError::internal(
                "unsupported socket kind for port bindings",
            )),
        }?
    };

    if local_port == 0 {
        return Err(SysprimsError::internal("socket has no local port"));
    }

    let state = if protocol == Protocol::Tcp {
        if kind != SOCKINFO_TCP {
            // We can't reliably read TCP state from non-TCP socket kinds.
            return Err(SysprimsError::internal(
                "tcp socket kind unsupported for listener detection",
            ));
        }

        let tcp: TcpSockInfo = unsafe { std::ptr::read_unaligned(proto_ptr as *const TcpSockInfo) };
        if tcp.tcpsi_state == TSI_S_LISTEN {
            Some("listen".to_string())
        } else {
            // Keep semantics strict: only return listening TCP sockets.
            return Err(SysprimsError::internal("tcp socket not listening"));
        }
    } else {
        None
    };

    let process = read_process_info(pid as u32).ok();

    Ok(PortBinding {
        protocol,
        local_addr,
        local_port,
        state,
        pid: Some(pid as u32),
        process,
        inode: None,
    })
}

fn read_tcp_binding(info: &TcpSockInfo) -> SysprimsResult<(Option<IpAddr>, u16)> {
    let port = u16::from_be(info.tcpsi_ini.insi_lport as u16);
    let addr = read_in_addr(&info.tcpsi_ini)?;
    Ok((addr, port))
}

fn read_in_binding(info: &InSockInfo) -> SysprimsResult<(Option<IpAddr>, u16)> {
    let port = u16::from_be(info.insi_lport as u16);
    let addr = read_in_addr(info)?;
    Ok((addr, port))
}

fn read_in_addr(info: &InSockInfo) -> SysprimsResult<Option<IpAddr>> {
    if info.insi_vflag & INI_IPV4 == INI_IPV4 {
        let addr = unsafe { info.insi_laddr.ina_46.i46a_addr4 };
        return Ok(Some(IpAddr::V4(Ipv4Addr::new(
            addr[0], addr[1], addr[2], addr[3],
        ))));
    }

    if info.insi_vflag & INI_IPV6 == INI_IPV6 {
        let addr = unsafe { info.insi_laddr.ina_6 };
        return Ok(Some(IpAddr::V6(Ipv6Addr::from(addr))));
    }

    Ok(None)
}

/// Read process information for a single PID.
fn read_process_info(pid: u32) -> SysprimsResult<ProcessInfo> {
    let bsd_info = get_bsd_info(pid)?;
    let task_info = get_task_info(pid).ok();
    let name = get_process_name(pid).unwrap_or_else(|| extract_name(&bsd_info));
    let user = get_username(bsd_info.pbi_uid);

    // Calculate elapsed time
    let start_time = Duration::new(
        bsd_info.pbi_start_tvsec,
        bsd_info.pbi_start_tvusec as u32 * 1000,
    );
    let start_time_unix_ms = bsd_info
        .pbi_start_tvsec
        .saturating_mul(1000)
        .saturating_add((bsd_info.pbi_start_tvusec as u64) / 1000);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let elapsed_seconds = now.as_secs().saturating_sub(start_time.as_secs());

    // Best-effort executable path
    let exe_path = {
        let mut buffer = [0u8; MAXPATHLEN];
        let result = unsafe {
            proc_pidpath(
                pid as c_int,
                buffer.as_mut_ptr() as *mut c_void,
                MAXPATHLEN as u32,
            )
        };

        if result > 0 {
            Some(
                unsafe { CStr::from_ptr(buffer.as_ptr() as *const i8) }
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            None
        }
    };

    // Calculate CPU percentage
    let cpu_percent = task_info
        .as_ref()
        .map(|t| calculate_cpu_percent(t, elapsed_seconds))
        .unwrap_or(0.0);

    // Memory in KB
    let memory_kb = task_info
        .as_ref()
        .map(|t| t.pti_resident_size / 1024)
        .unwrap_or(0);

    // Process state
    let state = match bsd_info.pbi_status {
        SRUN => ProcessState::Running,
        SSLEEP | SIDL => ProcessState::Sleeping,
        SSTOP => ProcessState::Stopped,
        SZOMB => ProcessState::Zombie,
        _ => ProcessState::Unknown,
    };

    let cmdline = read_cmdline(pid);

    Ok(ProcessInfo {
        pid,
        ppid: bsd_info.pbi_ppid,
        name,
        user,
        cpu_percent,
        memory_kb,
        elapsed_seconds,
        start_time_unix_ms: Some(start_time_unix_ms),
        exe_path,
        state,
        cmdline,
    })
}

/// Read command-line arguments for a process via `sysctl(CTL_KERN, KERN_PROCARGS2)`.
///
/// Returns the full argv vector (e.g. `["bun", "run", "scripts/dev.ts", "--root", "/path"]`).
/// Returns an empty vector if the process doesn't exist, we lack permissions, or parsing fails.
fn read_cmdline(pid: u32) -> Vec<String> {
    // Defensive: avoid pid_t overflow / negative semantics via cast.
    if pid == 0 || pid > i32::MAX as u32 {
        return Vec::new();
    }

    let mut mib: [c_int; 3] = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as c_int];

    // First call: query buffer size
    let mut size: usize = 0;
    let ret = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret != 0 || size == 0 {
        return Vec::new();
    }

    // Second call: read the data
    let mut buf: Vec<u8> = vec![0u8; size];
    let ret = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret != 0 {
        return Vec::new();
    }
    buf.truncate(size);

    // Parse KERN_PROCARGS2 format:
    //   [argc: i32] [exec_path\0] [padding \0s] [argv[0]\0] [argv[1]\0] ...
    if buf.len() < mem::size_of::<c_int>() {
        return Vec::new();
    }

    // argc is untrusted data from the kernel buffer; cap it to avoid pathological allocations.
    const MAX_ARGC: i32 = 4096;

    let argc = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if argc <= 0 || argc > MAX_ARGC {
        return Vec::new();
    }

    // Skip past exec_path (null-terminated string after argc)
    let mut pos = mem::size_of::<c_int>();
    // Scan past the exec_path
    while pos < buf.len() && buf[pos] != 0 {
        pos += 1;
    }
    // Skip the null terminator and any padding null bytes
    while pos < buf.len() && buf[pos] == 0 {
        pos += 1;
    }

    // Read argc null-terminated argument strings
    let mut args = Vec::with_capacity(argc as usize);
    for _ in 0..argc {
        if pos >= buf.len() {
            break;
        }
        let start = pos;
        while pos < buf.len() && buf[pos] != 0 {
            pos += 1;
        }
        if start != pos {
            args.push(String::from_utf8_lossy(&buf[start..pos]).into_owned());
        }
        pos += 1; // skip null terminator
    }

    args
}

/// Get BSD info for a process.
fn get_bsd_info(pid: u32) -> SysprimsResult<ProcBsdInfo> {
    let mut info: ProcBsdInfo = unsafe { mem::zeroed() };
    let size = mem::size_of::<ProcBsdInfo>() as c_int;

    let result = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut c_void,
            size,
        )
    };

    if result <= 0 {
        // Check if process doesn't exist vs permission denied
        let errno = unsafe { *libc::__error() };
        if errno == libc::ESRCH {
            return Err(SysprimsError::not_found(pid));
        } else if errno == libc::EPERM || errno == libc::EACCES {
            return Err(SysprimsError::permission_denied(pid, "read process info"));
        }
        return Err(SysprimsError::not_found(pid));
    }

    Ok(info)
}

/// Get task info for a process (CPU, memory).
fn get_task_info(pid: u32) -> SysprimsResult<ProcTaskInfo> {
    let mut info: ProcTaskInfo = unsafe { mem::zeroed() };
    let size = mem::size_of::<ProcTaskInfo>() as c_int;

    let result = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDTASKINFO,
            0,
            &mut info as *mut _ as *mut c_void,
            size,
        )
    };

    if result <= 0 {
        return Err(SysprimsError::internal("Failed to get task info"));
    }

    Ok(info)
}

/// Get process name using proc_name.
fn get_process_name(pid: u32) -> Option<String> {
    let mut buffer = [0u8; MAXPATHLEN];

    let result = unsafe {
        proc_name(
            pid as c_int,
            buffer.as_mut_ptr() as *mut c_void,
            MAXPATHLEN as u32,
        )
    };

    if result > 0 {
        let name = unsafe {
            CStr::from_ptr(buffer.as_ptr() as *const i8)
                .to_string_lossy()
                .into_owned()
        };
        if !name.is_empty() {
            return Some(name);
        }
    }

    None
}

/// Extract name from BSD info comm field.
fn extract_name(info: &ProcBsdInfo) -> String {
    // Try pbi_name first (longer), then pbi_comm
    let name_bytes = if info.pbi_name[0] != 0 {
        &info.pbi_name[..]
    } else {
        &info.pbi_comm[..]
    };

    let end = name_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name_bytes.len());
    String::from_utf8_lossy(&name_bytes[..end]).into_owned()
}

/// Get username from UID (thread-safe).
///
/// Uses getpwuid_r which is reentrant and safe for concurrent calls.
fn get_username(uid: uid_t) -> Option<String> {
    // Initial buffer size - will grow if needed
    let mut buf_size = 1024usize;
    let max_buf_size = 65536usize;

    loop {
        let mut buf: Vec<u8> = vec![0; buf_size];
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();

        let ret = unsafe {
            libc::getpwuid_r(
                uid,
                &mut pwd,
                buf.as_mut_ptr() as *mut libc::c_char,
                buf_size,
                &mut result,
            )
        };

        if ret == libc::ERANGE && buf_size < max_buf_size {
            // Buffer too small, try larger
            buf_size *= 2;
            continue;
        }

        if ret != 0 || result.is_null() {
            return None;
        }

        // Extract username from the result
        let name_ptr = pwd.pw_name;
        if name_ptr.is_null() {
            return None;
        }

        let name = unsafe { CStr::from_ptr(name_ptr).to_string_lossy().into_owned() };
        return Some(name);
    }
}

/// Calculate CPU percentage from task info.
///
/// This is a rough estimate based on total CPU time divided by elapsed time.
/// For accurate instantaneous CPU usage, we'd need to sample twice.
fn calculate_cpu_percent(task_info: &ProcTaskInfo, elapsed_secs: u64) -> f64 {
    if elapsed_secs == 0 {
        return 0.0;
    }

    // Total CPU time in Mach time units - convert to nanoseconds
    let total_mach_time = task_info.pti_total_user + task_info.pti_total_system;
    let total_cpu_ns = mach_time_to_ns(total_mach_time);

    // Convert to seconds
    let cpu_secs = total_cpu_ns as f64 / 1_000_000_000.0;

    // Calculate percentage (normalized across all cores)
    // This gives lifetime average, not instantaneous
    let percent = (cpu_secs / elapsed_secs as f64) * 100.0;

    // Clamp to valid range
    percent.clamp(0.0, 100.0)
}

pub(crate) fn cpu_total_time_ns_impl(pid: u32) -> SysprimsResult<u64> {
    let task_info = get_task_info(pid)?;
    // Convert Mach time units to nanoseconds
    let total_mach_time = task_info
        .pti_total_user
        .saturating_add(task_info.pti_total_system);
    Ok(mach_time_to_ns(total_mach_time))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_pids() {
        let pids = list_all_pids().unwrap();
        assert!(!pids.is_empty());
        // Should contain at least PID 1 (launchd)
        assert!(pids.contains(&1));
    }

    #[test]
    fn test_read_self() {
        let pid = std::process::id();
        let info = read_process_info(pid).unwrap();
        assert_eq!(info.pid, pid);
    }

    #[test]
    fn test_read_pid_1_or_permission_denied() {
        // On macOS with SIP, launchd (PID 1) may not be readable
        // This is expected behavior, so we accept either success or permission denied
        match read_process_info(1) {
            Ok(info) => {
                assert_eq!(info.pid, 1);
                assert_eq!(info.ppid, 0);
                assert!(!info.name.is_empty());
            }
            Err(SysprimsError::PermissionDenied { pid, .. }) => {
                assert_eq!(pid, 1);
                // This is acceptable on modern macOS with SIP
            }
            Err(e) => panic!("Unexpected error reading PID 1: {:?}", e),
        }
    }

    #[test]
    fn test_nonexistent_pid() {
        let result = read_process_info(99999999);
        assert!(matches!(result, Err(SysprimsError::NotFound { .. })));
    }

    #[test]
    fn test_username_lookup() {
        // Current user should be resolvable
        let uid = unsafe { libc::geteuid() };
        let name = get_username(uid);
        assert!(name.is_some());
    }
}
