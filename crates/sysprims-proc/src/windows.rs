//! Windows implementation using Toolhelp32 and Win32 APIs
//!
//! Uses the following APIs:
//! - `CreateToolhelp32Snapshot` - enumerate all processes
//! - `Process32First/Next` - iterate process list
//! - `OpenProcess` / `GetProcessTimes` - CPU timing
//! - `GetProcessMemoryInfo` - memory usage
//! - `QueryFullProcessImageName` - process path

use crate::{make_snapshot, ProcessInfo, ProcessSnapshot, ProcessState};
use std::mem;
use sysprims_core::{SysprimsError, SysprimsResult};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
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
    let (cpu_percent, memory_kb, elapsed_seconds) = get_process_stats(pid).unwrap_or((0.0, 0, 0));

    Ok(ProcessInfo {
        pid,
        ppid,
        name: name.clone(),
        user: None, // Would require more complex token queries
        cpu_percent,
        memory_kb,
        elapsed_seconds,
        state: ProcessState::Unknown, // Windows doesn't expose this simply
        cmdline: vec![name],
    })
}

/// Get CPU and memory stats for a process.
unsafe fn get_process_stats(pid: u32) -> Option<(f64, u64, u64)> {
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
    let (cpu_percent, elapsed_seconds) = if times_ok {
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

        (cpu, elapsed)
    } else {
        (0.0, 0)
    };

    Some((cpu_percent, memory_kb, elapsed_seconds))
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
