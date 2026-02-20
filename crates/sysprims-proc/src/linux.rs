//! Linux implementation using /proc filesystem
//!
//! Reads process information from:
//! - `/proc/[pid]/stat` - process status and timing
//! - `/proc/[pid]/status` - detailed status including UID
//! - `/proc/[pid]/statm` - memory statistics
//! - `/proc/[pid]/cmdline` - command line arguments

use crate::{
    aggregate_error_warning, aggregate_permission_warning, make_port_snapshot, make_snapshot,
    FdInfo, FdKind, PortBinding, PortBindingsSnapshot, ProcessInfo, ProcessOptions,
    ProcessSnapshot, ProcessState, Protocol,
};
#[cfg(feature = "proc_ext")]
use crate::{MAX_ENV_ENTRIES, MAX_ENV_KEY_BYTES, MAX_ENV_TOTAL_BYTES, MAX_ENV_VALUE_BYTES};
#[cfg(feature = "proc_ext")]
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
use sysprims_core::{SysprimsError, SysprimsResult};

// ============================================================================
// Implementation
// ============================================================================

pub fn snapshot_impl(options: &ProcessOptions) -> SysprimsResult<ProcessSnapshot> {
    let mut processes = Vec::new();

    // Read /proc directory for numeric entries (PIDs)
    let proc_dir = match fs::read_dir("/proc") {
        Ok(dir) => dir,
        Err(e) => {
            return Err(SysprimsError::internal(format!(
                "Failed to read /proc: {}",
                e
            )));
        }
    };

    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip non-numeric entries
        if !name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let pid: u32 = match name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Skip PID 0 (kernel scheduler)
        if pid == 0 {
            continue;
        }

        // Silently skip processes we can't read
        if let Ok(info) = read_process_info(pid, options) {
            processes.push(info);
        }
    }

    Ok(make_snapshot(processes))
}

pub fn list_fds_impl(pid: u32) -> SysprimsResult<(Vec<FdInfo>, Vec<String>)> {
    let proc_fd_dir = Path::new("/proc").join(pid.to_string()).join("fd");
    let entries = match fs::read_dir(&proc_fd_dir) {
        Ok(d) => d,
        Err(e) => {
            return Err(match e.kind() {
                io::ErrorKind::NotFound => SysprimsError::not_found(pid),
                io::ErrorKind::PermissionDenied => {
                    SysprimsError::permission_denied(pid, "list fds")
                }
                _ => SysprimsError::internal(format!(
                    "Failed to read {}: {}",
                    proc_fd_dir.display(),
                    e
                )),
            })
        }
    };

    let mut fds = Vec::new();
    let mut read_errors = 0usize;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                read_errors += 1;
                continue;
            }
        };

        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let fd: u32 = match name_str.parse() {
            Ok(n) => n,
            Err(_) => continue,
        };

        let target = match fs::read_link(entry.path()) {
            Ok(t) => t.to_string_lossy().into_owned(),
            Err(_) => {
                read_errors += 1;
                continue;
            }
        };

        let kind = if target.starts_with("socket:[") {
            FdKind::Socket
        } else if target.starts_with("pipe:[") {
            FdKind::Pipe
        } else if target.starts_with("anon_inode:") {
            FdKind::Unknown
        } else {
            FdKind::File
        };

        fds.push(FdInfo {
            fd,
            kind,
            path: Some(target),
        });
    }

    fds.sort_by_key(|f| f.fd);

    let mut warnings = Vec::new();
    if let Some(w) = aggregate_error_warning(read_errors, "fd entries") {
        warnings.push(w);
    }

    Ok((fds, warnings))
}

pub fn get_process_impl(pid: u32, options: &ProcessOptions) -> SysprimsResult<ProcessInfo> {
    read_process_info(pid, options)
}

pub fn wait_pid_impl(pid: u32, timeout: Duration) -> SysprimsResult<crate::WaitPidResult> {
    let start = Instant::now();
    let mut first_check = true;

    loop {
        // SAFETY: kill(pid, 0) does not send a signal; it performs an existence/permission check.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            // Still running.
            // On Unix, an exited-but-unreaped child remains as a zombie and still
            // responds to kill(pid, 0). Treat zombies as exited for supervisor use.
            if let Ok(info) = read_process_info(pid, &ProcessOptions::default()) {
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

        // rc == -1
        let errno = unsafe { *libc::__errno_location() };
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
    let mut warnings = Vec::new();
    let mut bindings = collect_socket_bindings()?;

    if bindings.is_empty() {
        return Ok(make_port_snapshot(bindings, warnings));
    }

    let inode_to_pid = match map_inodes_to_pids(&bindings) {
        Ok((map, permission_denied, read_errors)) => {
            if let Some(warning) = aggregate_permission_warning(permission_denied, "pid entries") {
                warnings.push(warning);
            }
            if let Some(warning) = aggregate_error_warning(read_errors, "pid entries") {
                warnings.push(warning);
            }
            map
        }
        Err(err) => {
            warnings.push(format!("Failed to map socket inodes to PIDs: {}", err));
            HashMap::new()
        }
    };

    for binding in &mut bindings {
        if let Some(inode) = binding_inode(binding) {
            if let Some(pid) = inode_to_pid.get(&inode) {
                binding.pid = Some(*pid);
                if let Ok(process) = read_process_info(*pid, &ProcessOptions::default()) {
                    binding.process = Some(process);
                }
            }
        }
        binding.inode = None;
    }

    Ok(make_port_snapshot(bindings, warnings))
}

/// Read process information from /proc/[pid]/*.
fn read_process_info(pid: u32, options: &ProcessOptions) -> SysprimsResult<ProcessInfo> {
    let proc_path = Path::new("/proc").join(pid.to_string());

    // Check if process exists
    if !proc_path.exists() {
        return Err(SysprimsError::not_found(pid));
    }

    // Read /proc/[pid]/stat
    let stat_content = read_file(&proc_path.join("stat")).map_err(|e| map_io_error(e, pid))?;
    let stat = parse_stat(&stat_content)?;

    // Read /proc/[pid]/status for UID
    let status_content = read_file(&proc_path.join("status")).unwrap_or_default();
    let uid = parse_uid(&status_content);
    let user = uid.and_then(get_username);

    // Read /proc/[pid]/statm for memory
    let statm_content = read_file(&proc_path.join("statm")).unwrap_or_default();
    let memory_kb = parse_memory(&statm_content);

    // Read /proc/[pid]/cmdline (handles non-UTF-8 gracefully)
    let cmdline = read_cmdline(&proc_path.join("cmdline"));

    #[cfg(not(feature = "proc_ext"))]
    let _ = options;

    #[cfg(feature = "proc_ext")]
    let env = if options.include_env {
        read_env(&proc_path.join("environ"))
    } else {
        None
    };
    #[cfg(not(feature = "proc_ext"))]
    let env = None;

    #[cfg(feature = "proc_ext")]
    let thread_count = if options.include_threads {
        parse_thread_count(&status_content)
    } else {
        None
    };
    #[cfg(not(feature = "proc_ext"))]
    let thread_count = None;

    // Calculate elapsed time
    let boot_time = get_boot_time();
    let clock_ticks = get_clock_ticks();
    let start_time_secs = stat.starttime / clock_ticks + boot_time;
    let start_time_unix_ms = start_time_secs.saturating_mul(1000);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let elapsed_seconds = now.saturating_sub(start_time_secs);

    // Best-effort executable path (/proc/<pid>/exe)
    let exe_path = fs::read_link(proc_path.join("exe"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned());

    // Calculate CPU percentage (lifetime average)
    let total_cpu_ticks = stat.utime + stat.stime;
    let cpu_secs = total_cpu_ticks as f64 / clock_ticks as f64;
    let cpu_percent = if elapsed_seconds > 0 {
        (cpu_secs / elapsed_seconds as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    // Process state
    let state = match stat.state {
        'R' => ProcessState::Running,
        'S' | 'D' | 'I' => ProcessState::Sleeping,
        'T' | 't' => ProcessState::Stopped,
        'Z' | 'X' => ProcessState::Zombie,
        _ => ProcessState::Unknown,
    };

    // Use command name from cmdline if available, otherwise use comm
    let name = if cmdline.is_empty() || cmdline[0].is_empty() {
        stat.comm.clone()
    } else {
        // Extract just the executable name from path
        let cmd = &cmdline[0];
        cmd.rsplit('/').next().unwrap_or(cmd).to_string()
    };

    Ok(ProcessInfo {
        pid,
        ppid: stat.ppid,
        name,
        user,
        cpu_percent,
        memory_kb,
        elapsed_seconds,
        start_time_unix_ms: Some(start_time_unix_ms),
        exe_path,
        state,
        cmdline,
        env,
        thread_count,
    })
}

pub(crate) fn cpu_total_time_ns_impl(pid: u32) -> SysprimsResult<u64> {
    let proc_path = Path::new("/proc").join(pid.to_string());
    if !proc_path.exists() {
        return Err(SysprimsError::not_found(pid));
    }

    let stat_content = read_file(&proc_path.join("stat")).map_err(|e| map_io_error(e, pid))?;
    let stat = parse_stat(&stat_content)?;
    let clock_ticks = get_clock_ticks();
    let total_cpu_ticks = stat.utime + stat.stime;

    // Convert ticks -> nanoseconds.
    // Use u128 to avoid overflow.
    let ns = (total_cpu_ticks as u128)
        .saturating_mul(1_000_000_000u128)
        .checked_div(clock_ticks as u128)
        .unwrap_or(0);

    Ok(ns as u64)
}

fn collect_socket_bindings() -> SysprimsResult<Vec<PortBinding>> {
    let mut bindings = Vec::new();

    let tcp = parse_proc_net("/proc/net/tcp", Protocol::Tcp, &mut bindings)?;
    let tcp6 = parse_proc_net("/proc/net/tcp6", Protocol::Tcp, &mut bindings)?;
    let udp = parse_proc_net("/proc/net/udp", Protocol::Udp, &mut bindings)?;
    let udp6 = parse_proc_net("/proc/net/udp6", Protocol::Udp, &mut bindings)?;

    if !(tcp || tcp6 || udp || udp6) {
        return Err(SysprimsError::not_supported("port bindings", "linux"));
    }

    Ok(bindings)
}

fn parse_proc_net(
    path: &str,
    protocol: Protocol,
    bindings: &mut Vec<PortBinding>,
) -> SysprimsResult<bool> {
    let content = match fs::read_to_string(path) {
        Ok(data) => data,
        Err(err) => {
            if err.kind() == io::ErrorKind::NotFound {
                return Ok(false);
            }
            return Err(SysprimsError::internal(format!(
                "Failed to read {}: {}",
                path, err
            )));
        }
    };

    let found_file = true;

    for (line_idx, line) in content.lines().enumerate() {
        if line_idx == 0 {
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 10 {
            continue;
        }

        let local = parts[1];
        let state = parts[3];
        let inode = parts[9];

        let (local_addr, local_port) = parse_local_socket(local)?;
        if local_port == 0 {
            continue;
        }

        if protocol == Protocol::Tcp && state != "0A" {
            continue;
        }

        let state = if protocol == Protocol::Tcp {
            Some("listen".to_string())
        } else {
            None
        };

        let inode = inode.parse::<u64>().ok();

        bindings.push(PortBinding {
            protocol,
            local_addr,
            local_port,
            state,
            pid: None,
            process: None,
            inode,
        });
    }

    Ok(found_file)
}

fn parse_local_socket(local: &str) -> SysprimsResult<(Option<IpAddr>, u16)> {
    let mut parts = local.split(':');
    let addr_hex = parts
        .next()
        .ok_or_else(|| SysprimsError::internal("missing local address"))?;
    let port_hex = parts
        .next()
        .ok_or_else(|| SysprimsError::internal("missing local port"))?;

    let port_raw = u16::from_str_radix(port_hex, 16)
        .map_err(|_| SysprimsError::internal("invalid port hex"))?;
    let port = u16::from_be(port_raw);

    let addr = match addr_hex.len() {
        8 => Some(IpAddr::V4(parse_ipv4(addr_hex)?)),
        32 => Some(IpAddr::V6(parse_ipv6(addr_hex)?)),
        _ => None,
    };

    Ok((addr, port))
}

fn parse_ipv4(addr_hex: &str) -> SysprimsResult<Ipv4Addr> {
    let raw = u32::from_str_radix(addr_hex, 16)
        .map_err(|_| SysprimsError::internal("invalid IPv4 hex"))?;
    let bytes = raw.to_le_bytes();
    Ok(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]))
}

fn parse_ipv6(addr_hex: &str) -> SysprimsResult<Ipv6Addr> {
    if addr_hex.len() != 32 {
        return Err(SysprimsError::internal("invalid IPv6 hex length"));
    }
    let mut bytes = [0u8; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        let start = i * 2;
        let slice = &addr_hex[start..start + 2];
        *byte = u8::from_str_radix(slice, 16)
            .map_err(|_| SysprimsError::internal("invalid IPv6 hex"))?;
    }

    for chunk in bytes.chunks_exact_mut(4) {
        chunk.reverse();
    }

    Ok(Ipv6Addr::from(bytes))
}

fn binding_inode(binding: &PortBinding) -> Option<u64> {
    binding.inode
}

fn map_inodes_to_pids(
    bindings: &[PortBinding],
) -> SysprimsResult<(HashMap<u64, u32>, usize, usize)> {
    let mut candidate_inodes = HashMap::new();
    for binding in bindings {
        if let Some(inode) = binding_inode(binding) {
            candidate_inodes.insert(inode, ());
        }
    }

    if candidate_inodes.is_empty() {
        return Ok((HashMap::new(), 0, 0));
    }

    let mut inode_to_pid = HashMap::new();
    let mut permission_denied = 0usize;
    let mut read_errors = 0usize;

    let proc_dir = fs::read_dir("/proc")
        .map_err(|e| SysprimsError::internal(format!("Failed to read /proc: {}", e)))?;

    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let pid: u32 = match name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        if pid == 0 {
            continue;
        }

        let fd_dir = entry.path().join("fd");
        let entries = match fs::read_dir(&fd_dir) {
            Ok(entries) => entries,
            Err(err) => {
                if err.kind() == io::ErrorKind::PermissionDenied {
                    permission_denied += 1;
                } else {
                    read_errors += 1;
                }
                continue;
            }
        };

        for fd_entry in entries.flatten() {
            let target = match fs::read_link(fd_entry.path()) {
                Ok(target) => target,
                Err(_) => continue,
            };
            let target_str = target.to_string_lossy();
            if let Some(inode) = parse_socket_inode(&target_str) {
                if candidate_inodes.contains_key(&inode) {
                    inode_to_pid.entry(inode).or_insert(pid);
                }
            }
        }
    }

    Ok((inode_to_pid, permission_denied, read_errors))
}

fn parse_socket_inode(target: &str) -> Option<u64> {
    let prefix = "socket:[";
    if !target.starts_with(prefix) || !target.ends_with(']') {
        return None;
    }
    let inode_str = &target[prefix.len()..target.len() - 1];
    inode_str.parse::<u64>().ok()
}

/// Parsed /proc/[pid]/stat fields.
struct StatInfo {
    comm: String,
    state: char,
    ppid: u32,
    utime: u64,
    stime: u64,
    starttime: u64,
}

/// Parse /proc/[pid]/stat content.
///
/// Format: pid (comm) state ppid pgrp session tty_nr tpgid flags minflt cminflt
///         majflt cmajflt utime stime cutime cstime priority nice num_threads
///         itrealvalue starttime vsize rss ...
fn parse_stat(content: &str) -> SysprimsResult<StatInfo> {
    // comm can contain spaces and parens, so we need to parse carefully
    let start_paren = content
        .find('(')
        .ok_or_else(|| SysprimsError::internal("Invalid stat format: no '('"))?;
    let end_paren = content
        .rfind(')')
        .ok_or_else(|| SysprimsError::internal("Invalid stat format: no ')'"))?;

    let comm = content[start_paren + 1..end_paren].to_string();

    // Fields after the closing paren
    let rest = &content[end_paren + 2..]; // Skip ") "
    let fields: Vec<&str> = rest.split_whitespace().collect();

    if fields.len() < 20 {
        return Err(SysprimsError::internal(
            "Invalid stat format: too few fields",
        ));
    }

    let state = fields[0].chars().next().unwrap_or('?');
    let ppid: u32 = fields[1].parse().unwrap_or(0);
    let utime: u64 = fields[11].parse().unwrap_or(0);
    let stime: u64 = fields[12].parse().unwrap_or(0);
    let starttime: u64 = fields[19].parse().unwrap_or(0);

    Ok(StatInfo {
        comm,
        state,
        ppid,
        utime,
        stime,
        starttime,
    })
}

/// Parse UID from /proc/[pid]/status.
fn parse_uid(content: &str) -> Option<u32> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            // Format: "Uid:\treal\teffective\tsaved\tfsuid"
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if !fields.is_empty() {
                return fields[0].parse().ok();
            }
        }
    }
    None
}

fn parse_thread_count(content: &str) -> Option<u32> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("Threads:") {
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if let Some(first) = fields.first() {
                return first.parse().ok();
            }
        }
    }
    None
}

#[cfg(feature = "proc_ext")]
fn read_env(path: &Path) -> Option<BTreeMap<String, String>> {
    let bytes = fs::read(path).ok()?;

    let mut env = BTreeMap::new();
    let mut total_bytes = 0usize;

    for entry in bytes.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        if env.len() >= MAX_ENV_ENTRIES {
            break;
        }

        let pair = String::from_utf8_lossy(entry);
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        if key.len() > MAX_ENV_KEY_BYTES {
            continue;
        }

        if value.len() > MAX_ENV_VALUE_BYTES {
            continue;
        }

        let entry_bytes = key.len().saturating_add(value.len());
        total_bytes = total_bytes.saturating_add(entry_bytes);
        if total_bytes > MAX_ENV_TOTAL_BYTES {
            return None;
        }

        env.insert(key.to_string(), value.to_string());
    }

    Some(env)
}

/// Parse memory from /proc/[pid]/statm.
///
/// Format: size resident shared text lib data dt (all in pages)
fn parse_memory(content: &str) -> u64 {
    let fields: Vec<&str> = content.split_whitespace().collect();
    if fields.len() >= 2 {
        let pages: u64 = fields[1].parse().unwrap_or(0);
        let page_size = get_page_size();
        return pages * page_size / 1024; // Convert to KB
    }
    0
}

/// Read command line from /proc/[pid]/cmdline.
///
/// Arguments are separated by null bytes. Uses lossy UTF-8 conversion
/// to handle non-UTF-8 command line arguments gracefully.
fn read_cmdline(path: &Path) -> Vec<String> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };

    bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Read file content as string.
fn read_file(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
}

/// Map IO error to SysprimsError.
fn map_io_error(e: io::Error, pid: u32) -> SysprimsError {
    match e.kind() {
        io::ErrorKind::NotFound => SysprimsError::not_found(pid),
        io::ErrorKind::PermissionDenied => {
            SysprimsError::permission_denied(pid, "read process info")
        }
        _ => SysprimsError::not_found(pid),
    }
}

/// Get username from UID (thread-safe).
///
/// Uses getpwuid_r which is reentrant and safe for concurrent calls.
fn get_username(uid: u32) -> Option<String> {
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

/// Get system boot time from /proc/stat.
fn get_boot_time() -> u64 {
    if let Ok(content) = fs::read_to_string("/proc/stat") {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("btime ") {
                return rest.trim().parse().unwrap_or(0);
            }
        }
    }
    0
}

/// Get clock ticks per second (usually 100 on Linux).
///
/// Returns 100 as fallback if sysconf fails (returns -1).
fn get_clock_ticks() -> u64 {
    let result = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if result <= 0 {
        100 // Standard fallback for Linux
    } else {
        result as u64
    }
}

/// Get page size in bytes.
///
/// Returns 4096 as fallback if sysconf fails (returns -1).
fn get_page_size() -> u64 {
    let result = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if result <= 0 {
        4096 // Standard fallback page size
    } else {
        result as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_stat() {
        let content = "1234 (test process) S 1 1234 1234 0 -1 4194304 1000 0 0 0 100 50 0 0 20 0 1 0 12345 67890 123 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0";
        let stat = parse_stat(content).unwrap();
        assert_eq!(stat.comm, "test process");
        assert_eq!(stat.state, 'S');
        assert_eq!(stat.ppid, 1);
        assert_eq!(stat.utime, 100);
        assert_eq!(stat.stime, 50);
        assert_eq!(stat.starttime, 12345);
    }

    #[test]
    fn test_parse_uid() {
        let content = "Name:\ttest\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\n";
        let uid = parse_uid(content);
        assert_eq!(uid, Some(1000));
    }

    #[test]
    fn test_clock_ticks() {
        let ticks = get_clock_ticks();
        assert!(ticks > 0, "Clock ticks should be positive");
        // Common values are 100 or 1000
        assert!((100..=10000).contains(&ticks));
    }

    #[test]
    fn test_page_size() {
        let size = get_page_size();
        assert!(size > 0, "Page size should be positive");
        // Common value is 4096
        assert!(size >= 1024);
    }
}
