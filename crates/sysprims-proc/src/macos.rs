//! macOS implementation using libproc
//!
//! Uses the following APIs:
//! - `proc_listpids()` - enumerate all PIDs
//! - `proc_pidinfo()` with `PROC_PIDTBSDINFO` - process info (name, ppid, state, user)
//! - `proc_pidinfo()` with `PROC_PIDTASKINFO` - resource info (CPU, memory)
//! - `proc_name()` - get process name

use crate::{
    aggregate_error_warning, aggregate_permission_warning, make_port_snapshot, make_snapshot,
    PortBinding, PortBindingsSnapshot, ProcessInfo, ProcessSnapshot, ProcessState, Protocol,
};
use libc::{c_int, c_void, pid_t, uid_t};
use std::ffi::CStr;
use std::mem;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
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
const PROC_PIDFDSOCKETINFO: c_int = 3;
const PROX_FDTYPE_SOCKET: u32 = 2;

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
#[derive(Debug, Default, Clone, Copy)]
struct ProcFileInfo {
    fi_openflags: u32,
    fi_status: u32,
    fi_offset: i64,
    fi_type: i32,
    fi_guardflags: u32,
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
struct SocketInfo {
    soi_stat: [u8; 144],
    soi_so: u64,
    soi_pcb: u64,
    soi_type: i32,
    soi_protocol: i32,
    soi_family: i32,
    soi_options: i16,
    soi_linger: i16,
    soi_state: i16,
    soi_qlen: i16,
    soi_incqlen: i16,
    soi_qlimit: i16,
    soi_timeo: i16,
    soi_error: u16,
    soi_oobmark: u32,
    soi_rcv: [u8; 24],
    soi_snd: [u8; 24],
    soi_kind: i32,
    rfu_1: u32,
    soi_proto: SocketProto,
}

#[repr(C)]
union SocketProto {
    pri_in: InSockInfo,
    pri_tcp: TcpSockInfo,
    pri_un: [u8; 106],
    pri_ndrv: [u8; 16],
    pri_kern_event: [u8; 12],
    pri_kern_ctl: [u8; 68],
    pri_vsock: [u8; 40],
}

#[repr(C)]
struct SocketFdInfo {
    pfi: ProcFileInfo,
    psi: SocketInfo,
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

pub fn listening_ports_impl() -> SysprimsResult<PortBindingsSnapshot> {
    let pids = list_all_pids()?;
    let mut bindings = Vec::new();
    let mut permission_denied = 0usize;
    let mut read_errors = 0usize;

    for pid in pids {
        if pid <= 0 {
            continue;
        }

        match list_socket_fds(pid) {
            Ok(fds) => {
                for fd in fds {
                    if let Ok(binding) = read_socket_binding(pid, fd) {
                        bindings.push(binding);
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
    if let Some(warning) = aggregate_permission_warning(permission_denied, "pid entries") {
        warnings.push(warning);
    }
    if let Some(warning) = aggregate_error_warning(read_errors, "pid entries") {
        warnings.push(warning);
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

fn read_socket_binding(pid: pid_t, fd: i32) -> SysprimsResult<PortBinding> {
    let mut info: SocketFdInfo = unsafe { mem::zeroed() };
    let size = mem::size_of::<SocketFdInfo>() as c_int;
    let result = unsafe {
        proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDSOCKETINFO,
            &mut info as *mut _ as *mut c_void,
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

    let protocol = match info.psi.soi_kind {
        SOCKINFO_TCP => Protocol::Tcp,
        SOCKINFO_IN => Protocol::Udp,
        _ => {
            return Err(SysprimsError::internal(
                "unsupported socket kind for port bindings",
            ))
        }
    };

    let (local_addr, local_port) = unsafe {
        match protocol {
            Protocol::Tcp => read_tcp_binding(&info.psi.soi_proto.pri_tcp)?,
            Protocol::Udp => read_in_binding(&info.psi.soi_proto.pri_in)?,
        }
    };

    if local_port == 0 {
        return Err(SysprimsError::internal("socket has no local port"));
    }

    let state = if protocol == Protocol::Tcp
        && unsafe { info.psi.soi_proto.pri_tcp.tcpsi_state } == TSI_S_LISTEN
    {
        Some("listen".to_string())
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
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let elapsed_seconds = now.as_secs().saturating_sub(start_time.as_secs());

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

    // Get command line (not easily available on macOS, use name)
    let cmdline = if name.is_empty() {
        vec![]
    } else {
        vec![name.clone()]
    };

    Ok(ProcessInfo {
        pid,
        ppid: bsd_info.pbi_ppid,
        name,
        user,
        cpu_percent,
        memory_kb,
        elapsed_seconds,
        state,
        cmdline,
    })
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

    // Total CPU time in nanoseconds
    let total_cpu_ns = task_info.pti_total_user + task_info.pti_total_system;

    // Convert to seconds
    let cpu_secs = total_cpu_ns as f64 / 1_000_000_000.0;

    // Calculate percentage (normalized across all cores)
    // This gives lifetime average, not instantaneous
    let percent = (cpu_secs / elapsed_secs as f64) * 100.0;

    // Clamp to valid range
    percent.clamp(0.0, 100.0)
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
