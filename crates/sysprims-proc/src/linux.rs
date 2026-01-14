//! Linux implementation using /proc filesystem
//!
//! Reads process information from:
//! - `/proc/[pid]/stat` - process status and timing
//! - `/proc/[pid]/status` - detailed status including UID
//! - `/proc/[pid]/statm` - memory statistics
//! - `/proc/[pid]/cmdline` - command line arguments

use crate::{make_snapshot, ProcessInfo, ProcessSnapshot, ProcessState};
use std::ffi::CStr;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use sysprims_core::{SysprimsError, SysprimsResult};

// ============================================================================
// Implementation
// ============================================================================

pub fn snapshot_impl() -> SysprimsResult<ProcessSnapshot> {
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
        if let Ok(info) = read_process_info(pid) {
            processes.push(info);
        }
    }

    Ok(make_snapshot(processes))
}

pub fn get_process_impl(pid: u32) -> SysprimsResult<ProcessInfo> {
    read_process_info(pid)
}

/// Read process information from /proc/[pid]/*.
fn read_process_info(pid: u32) -> SysprimsResult<ProcessInfo> {
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

    // Calculate elapsed time
    let boot_time = get_boot_time();
    let clock_ticks = get_clock_ticks();
    let start_time_secs = stat.starttime / clock_ticks + boot_time;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let elapsed_seconds = now.saturating_sub(start_time_secs);

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
        state,
        cmdline,
    })
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
