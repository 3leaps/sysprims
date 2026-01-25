//! Windows implementation using Toolhelp32 and Win32 APIs
//!
//! Uses the following APIs:
//! - `CreateToolhelp32Snapshot` - enumerate all processes
//! - `Process32First/Next` - iterate process list
//! - `OpenProcess` / `GetProcessTimes` - CPU timing
//! - `GetProcessMemoryInfo` - memory usage
//! - `QueryFullProcessImageName` - process path

use crate::{
    aggregate_error_warning, make_port_snapshot, make_snapshot, PortBinding, PortBindingsSnapshot,
    ProcessInfo, ProcessSnapshot, ProcessState, Protocol,
};
use std::mem;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use sysprims_core::{SysprimsError, SysprimsResult};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER,
    INVALID_HANDLE_VALUE, NO_ERROR,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID,
    MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, MIB_TCP_STATE_LISTEN, MIB_UDP6ROW_OWNER_PID,
    MIB_UDP6TABLE_OWNER_PID, MIB_UDPROW_OWNER_PID, MIB_UDPTABLE_OWNER_PID,
    TCP_TABLE_OWNER_PID_LISTENER, UDP_TABLE_OWNER_PID,
};
use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;

use std::time::Duration;
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
    WaitForSingleObject, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_VM_READ,
};

// ============================================================================
// Implementation
// ============================================================================

pub fn snapshot_impl() -> SysprimsResult<ProcessSnapshot> {
    let mut processes = Vec::new();

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(SysprimsError::internal(format!(
                "CreateToolhelp32Snapshot failed: {}",
                GetLastError()
            )));
        }

        let mut entry: PROCESSENTRY32W = mem::zeroed();
        entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if let Ok(info) = process_entry_to_info(&entry) {
                    processes.push(info);
                }

                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snapshot);
    }

    Ok(make_snapshot(processes))
}

pub fn get_process_impl(pid: u32) -> SysprimsResult<ProcessInfo> {
    // Find process in snapshot
    let snap = snapshot_impl()?;
    snap.processes
        .into_iter()
        .find(|p| p.pid == pid)
        .ok_or_else(|| SysprimsError::not_found(pid))
}

pub fn wait_pid_impl(pid: u32, timeout: Duration) -> SysprimsResult<crate::WaitPidResult> {
    unsafe {
        let handle = OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 {
            let err = GetLastError();
            if err == ERROR_ACCESS_DENIED {
                return Err(SysprimsError::permission_denied(pid, "wait pid"));
            }
            return Err(SysprimsError::not_found(pid));
        }

        let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
        let wait = WaitForSingleObject(handle, timeout_ms);

        // WAIT_OBJECT_0 == 0, WAIT_TIMEOUT == 258
        if wait == 0 {
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut code) != 0;
            CloseHandle(handle);
            if !ok {
                return Ok(crate::make_wait_pid_result(
                    pid,
                    true,
                    false,
                    None,
                    vec!["GetExitCodeProcess failed".to_string()],
                ));
            }
            return Ok(crate::make_wait_pid_result(
                pid,
                true,
                false,
                Some(code as i32),
                vec![],
            ));
        }
        if wait == 258 {
            CloseHandle(handle);
            return Ok(crate::make_wait_pid_result(pid, false, true, None, vec![]));
        }

        CloseHandle(handle);
        Err(SysprimsError::system(
            "WaitForSingleObject failed",
            wait as i32,
        ))
    }
}

pub fn listening_ports_impl() -> SysprimsResult<PortBindingsSnapshot> {
    let mut warnings = Vec::new();
    let mut bindings = Vec::new();

    let (tcp_bindings, tcp_errors) = read_tcp_table(AF_INET)?;
    bindings.extend(tcp_bindings);
    if let Some(warning) = aggregate_error_warning(tcp_errors, "TCP entries") {
        warnings.push(warning);
    }

    let (tcp6_bindings, tcp6_errors) = read_tcp_table(AF_INET6)?;
    bindings.extend(tcp6_bindings);
    if let Some(warning) = aggregate_error_warning(tcp6_errors, "TCP6 entries") {
        warnings.push(warning);
    }

    let (udp_bindings, udp_errors) = read_udp_table(AF_INET)?;
    bindings.extend(udp_bindings);
    if let Some(warning) = aggregate_error_warning(udp_errors, "UDP entries") {
        warnings.push(warning);
    }

    let (udp6_bindings, udp6_errors) = read_udp_table(AF_INET6)?;
    bindings.extend(udp6_bindings);
    if let Some(warning) = aggregate_error_warning(udp6_errors, "UDP6 entries") {
        warnings.push(warning);
    }

    Ok(make_port_snapshot(bindings, warnings))
}

/// Convert PROCESSENTRY32W to ProcessInfo.
unsafe fn process_entry_to_info(entry: &PROCESSENTRY32W) -> SysprimsResult<ProcessInfo> {
    let pid = entry.th32ProcessID;
    let ppid = entry.th32ParentProcessID;

    // Extract process name from szExeFile (null-terminated wide string)
    let name = {
        let end = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        String::from_utf16_lossy(&entry.szExeFile[..end])
    };

    // Try to get additional info by opening the process
    let (cpu_percent, memory_kb, elapsed_seconds, start_time_unix_ms) =
        get_process_stats(pid).unwrap_or((0.0, 0, 0, None));

    let exe_path = get_process_exe_path(pid);

    Ok(ProcessInfo {
        pid,
        ppid,
        name: name.clone(),
        user: None, // Would require more complex token queries
        cpu_percent,
        memory_kb,
        elapsed_seconds,
        start_time_unix_ms,
        exe_path,
        state: ProcessState::Unknown, // Windows doesn't expose this simply
        cmdline: vec![name],
    })
}

fn read_tcp_table(af: u16) -> SysprimsResult<(Vec<PortBinding>, usize)> {
    let mut buffer_size: u32 = 0;
    let mut result = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut buffer_size,
            0,
            af as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };

    if result != ERROR_INSUFFICIENT_BUFFER {
        if result == ERROR_ACCESS_DENIED {
            return Err(SysprimsError::permission_denied(0, "list tcp table"));
        }
        if result != NO_ERROR {
            return Err(SysprimsError::system(
                "GetExtendedTcpTable failed",
                result as i32,
            ));
        }
    }

    let mut buffer = vec![0u8; buffer_size as usize];
    result = unsafe {
        GetExtendedTcpTable(
            buffer.as_mut_ptr() as *mut _,
            &mut buffer_size,
            0,
            af as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };

    if result != NO_ERROR {
        if result == ERROR_ACCESS_DENIED {
            return Err(SysprimsError::permission_denied(0, "list tcp table"));
        }
        return Err(SysprimsError::system(
            "GetExtendedTcpTable failed",
            result as i32,
        ));
    }

    match af {
        AF_INET6 => read_tcp_table_v6(&buffer),
        _ => read_tcp_table_v4(&buffer),
    }
}

fn read_tcp_table_v4(buffer: &[u8]) -> SysprimsResult<(Vec<PortBinding>, usize)> {
    let table = unsafe { &*(buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID) };
    let entries =
        unsafe { std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize) };

    let mut bindings = Vec::new();
    let mut skipped = 0usize;

    for row in entries {
        if row.dwState != MIB_TCP_STATE_LISTEN as u32 {
            continue;
        }
        if let Some(binding) = tcp_row_to_binding(row, AF_INET) {
            bindings.push(binding);
        } else {
            skipped += 1;
        }
    }

    Ok((bindings, skipped))
}

fn read_tcp_table_v6(buffer: &[u8]) -> SysprimsResult<(Vec<PortBinding>, usize)> {
    let table = unsafe { &*(buffer.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID) };
    let entries =
        unsafe { std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize) };

    let mut bindings = Vec::new();
    let mut skipped = 0usize;

    for row in entries {
        if row.dwState != MIB_TCP_STATE_LISTEN as u32 {
            continue;
        }
        if let Some(binding) = tcp6_row_to_binding(row) {
            bindings.push(binding);
        } else {
            skipped += 1;
        }
    }

    Ok((bindings, skipped))
}

fn read_udp_table(af: u16) -> SysprimsResult<(Vec<PortBinding>, usize)> {
    let mut buffer_size: u32 = 0;
    let mut result = unsafe {
        GetExtendedUdpTable(
            std::ptr::null_mut(),
            &mut buffer_size,
            0,
            af as u32,
            UDP_TABLE_OWNER_PID,
            0,
        )
    };

    if result != ERROR_INSUFFICIENT_BUFFER {
        if result == ERROR_ACCESS_DENIED {
            return Err(SysprimsError::permission_denied(0, "list udp table"));
        }
        if result != NO_ERROR {
            return Err(SysprimsError::system(
                "GetExtendedUdpTable failed",
                result as i32,
            ));
        }
    }

    let mut buffer = vec![0u8; buffer_size as usize];
    result = unsafe {
        GetExtendedUdpTable(
            buffer.as_mut_ptr() as *mut _,
            &mut buffer_size,
            0,
            af as u32,
            UDP_TABLE_OWNER_PID,
            0,
        )
    };

    if result != NO_ERROR {
        if result == ERROR_ACCESS_DENIED {
            return Err(SysprimsError::permission_denied(0, "list udp table"));
        }
        return Err(SysprimsError::system(
            "GetExtendedUdpTable failed",
            result as i32,
        ));
    }

    match af {
        AF_INET6 => read_udp_table_v6(&buffer),
        _ => read_udp_table_v4(&buffer),
    }
}

fn read_udp_table_v4(buffer: &[u8]) -> SysprimsResult<(Vec<PortBinding>, usize)> {
    let table = unsafe { &*(buffer.as_ptr() as *const MIB_UDPTABLE_OWNER_PID) };
    let entries =
        unsafe { std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize) };

    let mut bindings = Vec::new();
    let mut skipped = 0usize;

    for row in entries {
        if let Some(binding) = udp_row_to_binding(row, AF_INET) {
            bindings.push(binding);
        } else {
            skipped += 1;
        }
    }

    Ok((bindings, skipped))
}

fn read_udp_table_v6(buffer: &[u8]) -> SysprimsResult<(Vec<PortBinding>, usize)> {
    let table = unsafe { &*(buffer.as_ptr() as *const MIB_UDP6TABLE_OWNER_PID) };
    let entries =
        unsafe { std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize) };

    let mut bindings = Vec::new();
    let mut skipped = 0usize;

    for row in entries {
        if let Some(binding) = udp6_row_to_binding(row) {
            bindings.push(binding);
        } else {
            skipped += 1;
        }
    }

    Ok((bindings, skipped))
}

fn tcp_row_to_binding(row: &MIB_TCPROW_OWNER_PID, af: u16) -> Option<PortBinding> {
    let local_port = u16::from_be(row.dwLocalPort as u16);
    if local_port == 0 {
        return None;
    }

    let local_addr = match af {
        AF_INET => Some(IpAddr::V4(Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()))),
        _ => None,
    };

    Some(PortBinding {
        protocol: Protocol::Tcp,
        local_addr,
        local_port,
        state: Some("listen".to_string()),
        pid: Some(row.dwOwningPid),
        process: None,
        inode: None,
    })
}

fn tcp6_row_to_binding(row: &MIB_TCP6ROW_OWNER_PID) -> Option<PortBinding> {
    let local_port = u16::from_be(row.dwLocalPort as u16);
    if local_port == 0 {
        return None;
    }

    let local_addr = Some(IpAddr::V6(Ipv6Addr::from(row.ucLocalAddr)));

    Some(PortBinding {
        protocol: Protocol::Tcp,
        local_addr,
        local_port,
        state: Some("listen".to_string()),
        pid: Some(row.dwOwningPid),
        process: None,
        inode: None,
    })
}

fn udp_row_to_binding(row: &MIB_UDPROW_OWNER_PID, af: u16) -> Option<PortBinding> {
    let local_port = u16::from_be(row.dwLocalPort as u16);
    if local_port == 0 {
        return None;
    }

    let local_addr = match af {
        AF_INET => Some(IpAddr::V4(Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()))),
        _ => None,
    };

    Some(PortBinding {
        protocol: Protocol::Udp,
        local_addr,
        local_port,
        state: None,
        pid: Some(row.dwOwningPid),
        process: None,
        inode: None,
    })
}

fn udp6_row_to_binding(row: &MIB_UDP6ROW_OWNER_PID) -> Option<PortBinding> {
    let local_port = u16::from_be(row.dwLocalPort as u16);
    if local_port == 0 {
        return None;
    }

    let local_addr = Some(IpAddr::V6(Ipv6Addr::from(row.ucLocalAddr)));

    Some(PortBinding {
        protocol: Protocol::Udp,
        local_addr,
        local_port,
        state: None,
        pid: Some(row.dwOwningPid),
        process: None,
        inode: None,
    })
}

/// Get CPU and memory stats for a process.
unsafe fn get_process_stats(pid: u32) -> Option<(f64, u64, u64, Option<u64>)> {
    let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
    if handle == 0 {
        return None;
    }

    // Get process times
    let mut creation_time = mem::zeroed();
    let mut exit_time = mem::zeroed();
    let mut kernel_time = mem::zeroed();
    let mut user_time = mem::zeroed();

    let times_ok = GetProcessTimes(
        handle,
        &mut creation_time,
        &mut exit_time,
        &mut kernel_time,
        &mut user_time,
    ) != 0;

    // Get memory info
    let mut mem_counters: PROCESS_MEMORY_COUNTERS = mem::zeroed();
    mem_counters.cb = mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;

    let mem_ok = GetProcessMemoryInfo(
        handle,
        &mut mem_counters,
        mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
    ) != 0;

    CloseHandle(handle);

    let memory_kb = if mem_ok {
        mem_counters.WorkingSetSize as u64 / 1024
    } else {
        0
    };

    // Calculate elapsed time and CPU percent
    let (cpu_percent, elapsed_seconds, start_time_unix_ms) = if times_ok {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();

        // FILETIME is 100-nanosecond intervals since Jan 1, 1601
        // Convert to seconds since Unix epoch
        let creation_100ns =
            (creation_time.dwHighDateTime as u64) << 32 | creation_time.dwLowDateTime as u64;
        // Windows epoch to Unix epoch offset in 100ns intervals
        const WINDOWS_EPOCH_OFFSET: u64 = 116444736000000000;
        let creation_unix_100ns = creation_100ns.saturating_sub(WINDOWS_EPOCH_OFFSET);
        let creation_secs = creation_unix_100ns / 10_000_000;

        let elapsed = now.as_secs().saturating_sub(creation_secs);

        // Best-effort process creation time in Unix ms
        let creation_ms = creation_unix_100ns / 10_000;

        // CPU time in 100ns intervals
        let kernel_100ns =
            (kernel_time.dwHighDateTime as u64) << 32 | kernel_time.dwLowDateTime as u64;
        let user_100ns = (user_time.dwHighDateTime as u64) << 32 | user_time.dwLowDateTime as u64;
        let total_cpu_secs = (kernel_100ns + user_100ns) as f64 / 10_000_000.0;

        let cpu = if elapsed > 0 {
            (total_cpu_secs / elapsed as f64 * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        (cpu, elapsed, Some(creation_ms))
    } else {
        (0.0, 0, None)
    };

    Some((cpu_percent, memory_kb, elapsed_seconds, start_time_unix_ms))
}

unsafe fn get_process_exe_path(pid: u32) -> Option<String> {
    let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
    if handle == 0 {
        return None;
    }

    // Start with a reasonable buffer; retry if it is too small.
    let mut buf_len: u32 = 260;
    let mut buf: Vec<u16> = vec![0u16; buf_len as usize];

    let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut buf_len) != 0;
    if !ok {
        CloseHandle(handle);
        return None;
    }
    CloseHandle(handle);

    // buf_len is number of wide chars written (excluding null).
    buf.truncate(buf_len as usize);
    Some(String::from_utf16_lossy(&buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_not_empty() {
        let snap = snapshot_impl().unwrap();
        assert!(!snap.processes.is_empty());
    }

    #[test]
    fn test_get_self() {
        let pid = std::process::id();
        let info = get_process_impl(pid).unwrap();
        assert_eq!(info.pid, pid);
    }
}
