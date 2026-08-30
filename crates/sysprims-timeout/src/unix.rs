//! Unix implementation of timeout with process groups.
//!
//! Uses a pre-fork `setsid()` hook and sealed receipt to acquire an owned
//! session before reporting guaranteed group-signaling eligibility.

use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use libc::SIGKILL;
use sysprims_core::{SysprimsError, SysprimsResult};
use sysprims_session::UnixSessionReceipt;

use crate::{
    capture_containment_identity, capture_receipt_bound_containment_identity, completion_from_pids,
    unknown_completion, verify_containment_identity, ContainmentAdoptionError,
    ContainmentBoundaryStrength, ContainmentChild, ContainmentCompletionEvidence, ContainmentGuard,
    ContainmentIdentityValidation, ContainmentObservation, ContainmentOutcome,
    ContainmentSpawnError, GroupingMode, TerminateTreeConfig, TimeoutConfig, TimeoutOutcome,
    TreeKillReliability, MAX_COMPLETION_OBSERVATION_RETRIES, MAX_COMPLETION_OBSERVED_PIDS,
};
use crate::{SpawnInGroupConfig, SpawnInGroupResult};
use sysprims_core::get_platform;
use sysprims_core::schema::SPAWN_IN_GROUP_RESULT_V1;

/// Polling interval for checking if child has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub fn spawn_contained_impl(
    mut command: Command,
) -> Result<ContainmentGuard<Child>, ContainmentSpawnError> {
    let program = command.get_program().to_string_lossy().into_owned();
    let (hook, pending_receipt) =
        sysprims_session::prepare_session_acquisition().map_err(ContainmentSpawnError::Spawn)?;
    // SAFETY: the hook is prepared before fork and is the command's only
    // session/group acquirer. Its child-side implementation is async-signal-safe.
    unsafe {
        command.pre_exec(move || hook.acquire());
    }

    let child = command.spawn().map_err(|error| {
        ContainmentSpawnError::Spawn(if error.kind() == std::io::ErrorKind::NotFound {
            SysprimsError::not_found_command(&program)
        } else if error.kind() == std::io::ErrorKind::PermissionDenied {
            SysprimsError::permission_denied_command(&program)
        } else {
            SysprimsError::spawn_failed(&program, error.to_string())
        })
    })?;
    let receipt = match pending_receipt.into_receipt(child.id()) {
        Ok(receipt) => receipt,
        Err(error) => {
            cleanup_failed_spawn_child(child);
            return Err(ContainmentSpawnError::Adoption(error));
        }
    };
    match contain_acquired_session_impl(child, receipt) {
        Ok(guard) => Ok(guard),
        Err(adoption) => {
            cleanup_failed_spawn_child(adoption.child);
            Err(ContainmentSpawnError::Adoption(adoption.error))
        }
    }
}

fn cleanup_failed_spawn_child(mut child: Child) {
    match child.try_wait() {
        Ok(Some(_)) => {}
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
        }
        Err(_) => {
            // Ownership could not be confirmed through the child adapter.
            // Fail closed without signaling a raw PID.
        }
    }
}

pub fn adopt_contained_impl<C: ContainmentChild>(
    child: C,
) -> Result<ContainmentGuard<C>, ContainmentAdoptionError<C>> {
    let evidence = match acquire_unix_evidence(&child) {
        Ok(evidence) => evidence,
        Err(error) => return Err(ContainmentAdoptionError { error, child }),
    };

    Ok(ContainmentGuard {
        child: Some(child),
        identity: evidence.0,
        reliability: TreeKillReliability::Unproven,
        boundary_strength: ContainmentBoundaryStrength::CooperativeGroup,
        finalized: false,
        pgid: evidence.1,
        session_id: evidence.2,
        session_receipt: None,
    })
}

pub fn contain_acquired_session_impl<C: ContainmentChild>(
    child: C,
    receipt: UnixSessionReceipt,
) -> Result<ContainmentGuard<C>, ContainmentAdoptionError<C>> {
    let evidence = match acquire_receipt_evidence(&child, &receipt) {
        Ok(evidence) => evidence,
        Err(error) => return Err(ContainmentAdoptionError { error, child }),
    };

    Ok(ContainmentGuard {
        child: Some(child),
        identity: evidence,
        reliability: TreeKillReliability::Guaranteed,
        boundary_strength: ContainmentBoundaryStrength::CooperativeGroup,
        finalized: false,
        pgid: receipt.process_group_id(),
        session_id: receipt.session_id(),
        session_receipt: Some(receipt),
    })
}

fn acquire_receipt_evidence<C: ContainmentChild>(
    child: &C,
    receipt: &UnixSessionReceipt,
) -> SysprimsResult<crate::ContainmentIdentity> {
    let pid = child
        .process_id()
        .ok_or_else(|| SysprimsError::invalid_argument("child process id is unavailable"))?;
    if receipt.child_pid() != pid
        || receipt.process_group_id() != pid
        || receipt.session_id() != pid
        || receipt.session_kind() != "new_session"
        || receipt.identifier_provenance() != "setsid_structural_child_pid"
    {
        return Err(SysprimsError::invalid_argument(
            "session acquisition receipt does not match the owned child",
        ));
    }

    let child_was_exited = verify_owned_child_slot(pid)?;
    let identity = match capture_receipt_bound_containment_identity(pid) {
        Ok(identity) => identity,
        Err(_) if child_was_exited || owned_child_is_exited_unreaped(pid)? => {
            crate::ContainmentIdentity {
                pid,
                start_time_unix_ms: 0,
                exe_path: String::new(),
            }
        }
        Err(error) => return Err(error),
    };
    let pid_i32 = pid as i32;
    // SAFETY: `pid_i32` was range-checked through the receipt and owned child.
    let pgid = unsafe { libc::getpgid(pid_i32) };
    // SAFETY: same validated positive PID as above.
    let session_id = unsafe { libc::getsid(pid_i32) };
    if pgid != pid_i32 || session_id != pid_i32 {
        if child_was_exited || owned_child_is_exited_unreaped(pid)? {
            return Ok(identity);
        }
        return Err(SysprimsError::invalid_argument(
            "spawn-time session acquisition no longer matches the owned child",
        ));
    }
    // SAFETY: PID zero is used only with getpgid to query the caller; no signal
    // is sent.
    if unsafe { libc::getpgid(0) } == pgid {
        return Err(SysprimsError::invalid_argument(
            "acquired child shares the caller's process group",
        ));
    }
    Ok(identity)
}

fn validate_containment_before_signal<C: ContainmentChild>(
    guard: &ContainmentGuard<C>,
) -> SysprimsResult<bool> {
    let child = guard
        .child
        .as_ref()
        .expect("active containment guard retains its child");
    if child.process_id() != Some(guard.identity.pid) {
        return Err(SysprimsError::invalid_argument(
            "owned child identity changed; refusing containment operation",
        ));
    }

    let identity_validation = if let Some(receipt) = guard.session_receipt.as_ref() {
        if receipt.child_pid() != guard.identity.pid
            || receipt.process_group_id() != guard.pgid
            || receipt.session_id() != guard.session_id
            || receipt.session_kind() != "new_session"
            || receipt.identifier_provenance() != "setsid_structural_child_pid"
        {
            return Err(SysprimsError::invalid_argument(
                "sealed session receipt no longer matches the containment guard",
            ));
        }
        ContainmentIdentityValidation::ReceiptBoundExecVolatile
    } else {
        ContainmentIdentityValidation::Strict
    };

    let owned_child_exited = if guard.session_receipt.is_some() {
        Some(verify_owned_child_slot(guard.identity.pid)?)
    } else {
        None
    };
    let receipt_only_fast_exit = guard.session_receipt.is_some()
        && guard.identity.start_time_unix_ms == 0
        && guard.identity.exe_path.is_empty();
    let mut identity_metadata_verified = if receipt_only_fast_exit {
        false
    } else {
        verify_containment_identity(&guard.identity, identity_validation)?
    };
    let exited_ownership_verified = owned_child_exited.unwrap_or(false)
        || (guard.session_receipt.is_some()
            && !identity_metadata_verified
            && owned_child_is_exited_unreaped(guard.identity.pid)?);
    if guard.session_receipt.is_some() && !identity_metadata_verified && !exited_ownership_verified
    {
        return Err(SysprimsError::invalid_argument(
            "guaranteed child identity and unreaped ownership are unavailable; refusing group signal",
        ));
    }

    if identity_metadata_verified {
        let pid_i32 = guard.identity.pid as i32;
        let current_pgid = unsafe { libc::getpgid(pid_i32) };
        let current_session_id = unsafe { libc::getsid(pid_i32) };
        identity_metadata_verified = reconcile_group_identity_after_live_check(
            guard.session_receipt.is_some(),
            guard.pgid,
            guard.session_id,
            current_pgid,
            current_session_id,
            || owned_child_is_exited_unreaped(guard.identity.pid),
        )?;
    }

    if unsafe { libc::getpgid(0) } == guard.pgid as i32 {
        return Err(SysprimsError::invalid_argument(
            "contained group matches the caller's group; refusing group signal",
        ));
    }

    Ok(identity_metadata_verified)
}

fn reconcile_group_identity_after_live_check<F>(
    receipt_bound: bool,
    expected_pgid: u32,
    expected_session_id: u32,
    current_pgid: i32,
    current_session_id: i32,
    verify_exited_unreaped: F,
) -> SysprimsResult<bool>
where
    F: FnOnce() -> SysprimsResult<bool>,
{
    if current_pgid == expected_pgid as i32 && current_session_id == expected_session_id as i32 {
        return Ok(true);
    }
    if receipt_bound && verify_exited_unreaped()? {
        return Ok(false);
    }
    Err(SysprimsError::invalid_argument(
        "process group identity changed; refusing containment operation",
    ))
}

fn owned_child_is_exited_unreaped(pid: u32) -> SysprimsResult<bool> {
    for attempt in 0..=5 {
        if verify_owned_child_slot(pid)? {
            return Ok(true);
        }
        if attempt < 5 {
            std::thread::sleep(POLL_INTERVAL);
        }
    }
    Ok(false)
}

/// Verify that this process still owns the unreaped child slot.
///
/// A successful zero-status `waitid` proves a live child; a returned matching
/// PID proves an exited child while `WNOWAIT` preserves the reap capability.
fn verify_owned_child_slot(pid: u32) -> SysprimsResult<bool> {
    let mut information = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: `information` points to writable storage, the PID is range
    // checked by the receipt, and WNOWAIT preserves exclusive reap ownership.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            information.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        return Err(if error.raw_os_error() == Some(libc::ECHILD) {
            SysprimsError::invalid_argument(
                "owned child reap capability is unavailable; refusing guaranteed containment",
            )
        } else {
            SysprimsError::system(
                format!("cannot verify unreaped child ownership: {error}"),
                error.raw_os_error().unwrap_or(0),
            )
        });
    }

    // SAFETY: successful waitid initialized `information`.
    let information = unsafe { information.assume_init() };
    // SAFETY: waitid populated the child-status variant of siginfo_t.
    let observed_pid = unsafe { information.si_pid() };
    if observed_pid == 0 {
        return Ok(false);
    }
    if observed_pid == pid as libc::pid_t {
        return Ok(true);
    }
    Err(SysprimsError::invalid_argument(
        "waitid returned a different child; refusing guaranteed containment",
    ))
}

fn acquire_unix_evidence<C: ContainmentChild>(
    child: &C,
) -> SysprimsResult<(crate::ContainmentIdentity, u32, u32)> {
    let pid = child
        .process_id()
        .ok_or_else(|| SysprimsError::invalid_argument("child process id is unavailable"))?;
    let identity = capture_containment_identity(pid)?;
    let pid_i32 = pid as i32;
    let pgid = unsafe { libc::getpgid(pid_i32) };
    let session_id = unsafe { libc::getsid(pid_i32) };
    if pgid <= 0 || session_id <= 0 {
        return Err(SysprimsError::system(
            "process group or session unavailable during containment acquisition",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        ));
    }
    if pgid != pid_i32 {
        return Err(SysprimsError::invalid_argument(
            "child is not a process-group leader; refusing group adoption",
        ));
    }

    let caller_pgid = unsafe { libc::getpgid(0) };
    if caller_pgid == pgid {
        return Err(SysprimsError::invalid_argument(
            "child shares the caller's process group; refusing group adoption",
        ));
    }

    Ok((identity, pgid as u32, session_id as u32))
}

fn reconcile_group_signal_failure<F>(
    error: SysprimsError,
    observe_completion: F,
    empty_warning: &'static str,
) -> SysprimsResult<&'static str>
where
    F: FnOnce() -> ContainmentCompletionEvidence,
{
    let reconcilable = matches!(&error, SysprimsError::NotFound { .. })
        || (cfg!(target_os = "macos") && matches!(&error, SysprimsError::PermissionDenied { .. }));
    if !reconcilable {
        return Err(error);
    }

    if !matches!(
        observe_completion(),
        ContainmentCompletionEvidence::Empty { .. }
    ) {
        return Err(error);
    }

    Ok(empty_warning)
}

pub fn terminate_contained_impl<C: ContainmentChild>(
    guard: &mut ContainmentGuard<C>,
    config: TerminateTreeConfig,
) -> SysprimsResult<ContainmentOutcome> {
    validate_signal(config.signal, "signal")?;
    validate_signal(config.kill_signal, "kill_signal")?;

    let identity_metadata_verified = validate_containment_before_signal(guard)?;

    let mut warnings = Vec::new();
    if !identity_metadata_verified {
        if guard.session_receipt.is_some() {
            warnings.push(
                "Leader exited unreaped; same-spawn receipt and waitid ownership preserve group identity"
                    .to_string(),
            );
        } else {
            warnings.push(
                "Live identity metadata unavailable; proceeding with owned child and bound group"
                    .to_string(),
            );
        }
    }

    let graceful_sent = match sysprims_signal::killpg(guard.pgid, config.signal) {
        Ok(()) => true,
        Err(error) => {
            let warning = reconcile_group_signal_failure(
                error,
                || observe_containment_completion(guard.pgid, guard.session_id),
                "Process group became empty before cleanup signal; proceeding to reap",
            )?;
            warnings.push(warning.to_string());
            false
        }
    };

    if graceful_sent {
        std::thread::sleep(Duration::from_millis(config.grace_timeout_ms));
    }

    let escalated = if graceful_sent {
        validate_containment_before_signal(guard)?;
        match sysprims_signal::killpg(guard.pgid, config.kill_signal) {
            Ok(()) => true,
            Err(error) => {
                let warning = reconcile_group_signal_failure(
                    error,
                    || observe_containment_completion(guard.pgid, guard.session_id),
                    "Process group became empty before escalation; proceeding to reap",
                )?;
                warnings.push(warning.to_string());
                false
            }
        }
    } else {
        false
    };

    let _ = wait_for_contained_child_exit(
        guard.identity.pid,
        Duration::from_millis(config.kill_timeout_ms),
    );
    let completion = observe_containment_completion(guard.pgid, guard.session_id);
    let child = guard
        .child
        .as_mut()
        .expect("active containment guard retains its child");
    let exited = reap_contained_child(child)?;
    if !exited {
        warnings.push("Timed out waiting to reap contained child".to_string());
    } else {
        guard.finalized = true;
    }

    Ok(ContainmentOutcome {
        identity: guard.identity.clone(),
        pgid: Some(guard.pgid),
        signal_sent: graceful_sent.then_some(config.signal),
        kill_signal: escalated.then_some(config.kill_signal),
        escalated,
        exited,
        timed_out: !exited,
        tree_kill_reliability: guard.reliability,
        boundary_strength: guard.boundary_strength,
        completion,
        warnings,
    })
}

fn stabilize_completion<F>(
    observation: ContainmentObservation,
    mut snapshot: F,
) -> ContainmentCompletionEvidence
where
    F: FnMut() -> Result<Vec<u32>, ()>,
{
    let mut previous = None;
    for _ in 0..=MAX_COMPLETION_OBSERVATION_RETRIES {
        let current = match snapshot() {
            Ok(mut pids) => {
                pids.sort_unstable();
                pids
            }
            Err(()) => return unknown_completion(observation),
        };
        if previous.as_ref() == Some(&current) {
            return completion_from_pids(observation, current);
        }
        previous = Some(current);
        std::thread::sleep(POLL_INTERVAL);
    }
    unknown_completion(observation)
}

#[cfg(target_os = "linux")]
fn observe_containment_completion(pgid: u32, session_id: u32) -> ContainmentCompletionEvidence {
    use std::os::fd::AsRawFd;

    let proc_directory = match std::fs::File::open("/proc") {
        Ok(directory) => directory,
        Err(_) => return unknown_completion(ContainmentObservation::LinuxProcfsProcessGroup),
    };
    let descriptor = proc_directory.as_raw_fd();
    let fdinfo = match std::fs::read(format!("/proc/self/fdinfo/{descriptor}")) {
        Ok(fdinfo) => fdinfo,
        Err(_) => return unknown_completion(ContainmentObservation::LinuxProcfsProcessGroup),
    };
    let mountinfo = match std::fs::read("/proc/self/mountinfo") {
        Ok(mountinfo) => mountinfo,
        Err(_) => return unknown_completion(ContainmentObservation::LinuxProcfsProcessGroup),
    };
    let Some(mount_id) = linux_procfs_mount_id(&fdinfo) else {
        return unknown_completion(ContainmentObservation::LinuxProcfsProcessGroup);
    };
    let proc_path = format!("/proc/self/fd/{descriptor}");
    let proc_path = std::path::Path::new(&proc_path);

    observe_linux_completion_with(
        linux_procfs_visibility_is_complete(&mountinfo, mount_id)
            && linux_procfs_matches_caller_namespace(proc_path),
        || linux_group_snapshot(proc_path, pgid, session_id),
    )
}

#[cfg(target_os = "linux")]
fn observe_linux_completion_with<F>(
    visibility_is_complete: bool,
    snapshot: F,
) -> ContainmentCompletionEvidence
where
    F: FnMut() -> Result<Vec<u32>, ()>,
{
    if !visibility_is_complete {
        return unknown_completion(ContainmentObservation::LinuxProcfsProcessGroup);
    }
    stabilize_completion(ContainmentObservation::LinuxProcfsProcessGroup, snapshot)
}

#[cfg(target_os = "linux")]
fn linux_procfs_mount_id(fdinfo: &[u8]) -> Option<u64> {
    fdinfo.split(|byte| *byte == b'\n').find_map(|line| {
        let value = line.strip_prefix(b"mnt_id:")?;
        std::str::from_utf8(value).ok()?.trim().parse().ok()
    })
}

#[cfg(target_os = "linux")]
fn linux_procfs_visibility_is_complete(mountinfo: &[u8], effective_mount_id: u64) -> bool {
    let mut effective_mount_is_complete = false;
    for line in mountinfo.split(|byte| *byte == b'\n') {
        let Some(separator) = line.windows(3).position(|window| window == b" - ") else {
            continue;
        };
        let mut mount_fields = line[..separator].split(|byte| byte.is_ascii_whitespace());
        let mount_id = mount_fields
            .next()
            .and_then(|value| std::str::from_utf8(value).ok())
            .and_then(|value| value.parse::<u64>().ok());
        let _parent_id = mount_fields.next();
        let _device = mount_fields.next();
        let root = mount_fields.next();
        let mount_point = mount_fields.next();
        let mount_options = mount_fields.next();
        let mut filesystem_fields = line[separator + 3..].split(|byte| byte.is_ascii_whitespace());
        let filesystem_type = filesystem_fields.next();
        let _mount_source = filesystem_fields.next();
        let super_options = filesystem_fields.next();

        if mount_point.is_some_and(linux_procfs_mount_masks_pid) {
            return false;
        }
        if mount_id == Some(effective_mount_id) {
            effective_mount_is_complete = root == Some(b"/".as_slice())
                && mount_point == Some(b"/proc".as_slice())
                && filesystem_type == Some(b"proc".as_slice())
                && mount_options.is_some_and(linux_procfs_options_are_complete)
                && super_options.is_some_and(linux_procfs_options_are_complete);
        }
    }
    effective_mount_is_complete
}

#[cfg(target_os = "linux")]
fn linux_procfs_mount_masks_pid(mount_point: &[u8]) -> bool {
    let Some(relative) = mount_point.strip_prefix(b"/proc/") else {
        return false;
    };
    let first_component = relative
        .split(|byte| *byte == b'/')
        .next()
        .unwrap_or_default();
    !first_component.is_empty() && first_component.iter().all(u8::is_ascii_digit)
}

#[cfg(target_os = "linux")]
fn linux_procfs_matches_caller_namespace(proc_path: &std::path::Path) -> bool {
    let stat = match std::fs::read(proc_path.join("self/stat")) {
        Ok(stat) => stat,
        Err(_) => return false,
    };
    let caller_pgid = unsafe { libc::getpgid(0) };
    let caller_session = unsafe { libc::getsid(0) };
    if caller_pgid <= 0 || caller_session <= 0 {
        return false;
    }
    linux_procfs_identity_matches(
        &stat,
        std::process::id(),
        caller_pgid as u32,
        caller_session as u32,
    )
}

#[cfg(target_os = "linux")]
fn linux_procfs_identity_matches(
    stat: &[u8],
    caller_pid: u32,
    caller_pgid: u32,
    caller_session: u32,
) -> bool {
    let Some(pid_end) = stat.iter().position(|byte| byte.is_ascii_whitespace()) else {
        return false;
    };
    let Ok(procfs_pid) = std::str::from_utf8(&stat[..pid_end]) else {
        return false;
    };
    let Ok(procfs_pid) = procfs_pid.parse::<u32>() else {
        return false;
    };
    matches!(
        parse_linux_stat_membership(stat),
        Ok((_state, process_group, session))
            if procfs_pid == caller_pid
                && process_group == caller_pgid
                && session == caller_session
    )
}

#[cfg(target_os = "linux")]
fn linux_procfs_options_are_complete(options: &[u8]) -> bool {
    options.split(|byte| *byte == b',').all(|option| {
        !option.starts_with(b"hidepid") || matches!(option, b"hidepid=0" | b"hidepid=off")
    })
}

#[cfg(target_os = "linux")]
fn linux_group_snapshot(
    proc_path: &std::path::Path,
    pgid: u32,
    session_id: u32,
) -> Result<Vec<u32>, ()> {
    let entries = std::fs::read_dir(proc_path).map_err(|_| ())?;
    let mut members = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| ())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == 0 || pid > sysprims_signal::MAX_SAFE_PID {
            return Err(());
        }
        let stat = match std::fs::read(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(()),
        };
        let (state, process_group, session) = parse_linux_stat_membership(&stat)?;
        if process_group == pgid && session == session_id && !matches!(state, 'Z' | 'X' | 'x') {
            if members.len() == MAX_COMPLETION_OBSERVED_PIDS {
                return Err(());
            }
            members.push(pid);
        }
    }
    Ok(members)
}

#[cfg(target_os = "linux")]
fn parse_linux_stat_membership(stat: &[u8]) -> Result<(char, u32, u32), ()> {
    let end_paren = stat.iter().rposition(|byte| *byte == b')').ok_or(())?;
    let suffix = std::str::from_utf8(&stat[end_paren + 1..]).map_err(|_| ())?;
    let mut fields = suffix.split_whitespace();
    let state = fields
        .next()
        .and_then(|value| value.chars().next())
        .ok_or(())?;
    let _parent_pid = fields.next().ok_or(())?;
    let process_group = fields.next().ok_or(())?.parse().map_err(|_| ())?;
    let session = fields.next().ok_or(())?.parse().map_err(|_| ())?;
    Ok((state, process_group, session))
}

#[cfg(target_os = "macos")]
fn observe_containment_completion(pgid: u32, _session_id: u32) -> ContainmentCompletionEvidence {
    stabilize_completion(ContainmentObservation::MacosLibprocProcessGroup, || {
        macos_group_snapshot(pgid)
    })
}

#[cfg(target_os = "macos")]
fn macos_group_snapshot(pgid: u32) -> Result<Vec<u32>, ()> {
    use std::ffi::c_void;

    const PROC_PGRP_ONLY: u32 = 2;

    unsafe extern "C" {
        fn proc_listpids(
            type_: u32,
            typeinfo: u32,
            buffer: *mut c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
    }

    macos_group_snapshot_with(
        |buffer| {
            let written = match buffer {
                None => unsafe { proc_listpids(PROC_PGRP_ONLY, pgid, std::ptr::null_mut(), 0) },
                Some(buffer) => unsafe {
                    proc_listpids(
                        PROC_PGRP_ONLY,
                        pgid,
                        buffer.as_mut_ptr().cast(),
                        std::mem::size_of_val(buffer) as libc::c_int,
                    )
                },
            };
            if written < 0 || !(written as usize).is_multiple_of(std::mem::size_of::<i32>()) {
                return Err(());
            }
            Ok(written as usize / std::mem::size_of::<i32>())
        },
        |pid| sysprims_proc::is_live(pid).map_err(|_| ()),
    )
}

#[cfg(target_os = "macos")]
fn macos_group_snapshot_with<F, L>(mut list: F, mut is_live: L) -> Result<Vec<u32>, ()>
where
    F: FnMut(Option<&mut [i32]>) -> Result<usize, ()>,
    L: FnMut(u32) -> Result<bool, ()>,
{
    for _ in 0..=MAX_COMPLETION_OBSERVATION_RETRIES {
        let reported = list(None)?;
        if reported >= MAX_COMPLETION_OBSERVED_PIDS {
            return Err(());
        }
        let capacity = reported + 1;
        let mut pids = Vec::new();
        pids.try_reserve_exact(capacity).map_err(|_| ())?;
        pids.resize(capacity, 0_i32);
        let count = list(Some(&mut pids))?;
        if count >= capacity {
            continue;
        }
        let mut members = Vec::new();
        for pid in pids.into_iter().take(count).filter(|pid| *pid > 0) {
            let pid = pid as u32;
            if pid > sysprims_signal::MAX_SAFE_PID {
                return Err(());
            }
            if is_live(pid)? {
                members.push(pid);
            }
        }
        return Ok(members);
    }
    Err(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn observe_containment_completion(_pgid: u32, _session_id: u32) -> ContainmentCompletionEvidence {
    unknown_completion(ContainmentObservation::UnsupportedPlatform)
}

pub fn contained_child_has_exited<C: ContainmentChild>(
    guard: &ContainmentGuard<C>,
) -> SysprimsResult<bool> {
    let child = guard
        .child
        .as_ref()
        .expect("active containment guard retains its child");
    if child.process_id() != Some(guard.identity.pid) {
        return Err(SysprimsError::invalid_argument(
            "owned child identity changed; refusing containment operation",
        ));
    }
    if let Some(receipt) = guard.session_receipt.as_ref() {
        if receipt.child_pid() != guard.identity.pid
            || receipt.process_group_id() != guard.pgid
            || receipt.session_id() != guard.session_id
        {
            return Err(SysprimsError::invalid_argument(
                "sealed session receipt no longer matches the containment guard",
            ));
        }
    }

    sysprims_proc::is_live(guard.identity.pid).map(|live| !live)
}

pub fn drop_contained_impl<C: ContainmentChild>(guard: &mut ContainmentGuard<C>) {
    if validate_containment_before_signal(guard).is_err() {
        return;
    }
    let child = guard
        .child
        .as_mut()
        .expect("active containment guard retains its child");

    let _ = sysprims_signal::killpg(guard.pgid, SIGKILL);
    if wait_for_contained_child(child, Duration::from_secs(2)).unwrap_or(false) {
        guard.finalized = true;
    }
}

fn validate_signal(signal: i32, name: &str) -> SysprimsResult<()> {
    #[cfg(target_os = "linux")]
    let max_signal = libc::SIGRTMAX();
    #[cfg(not(target_os = "linux"))]
    let max_signal = 31;

    if signal <= 0 || signal > max_signal {
        return Err(SysprimsError::invalid_argument(format!(
            "{name} must be between 1 and {max_signal}"
        )));
    }
    Ok(())
}

fn wait_for_contained_child<C: ContainmentChild>(
    child: &mut C,
    timeout: Duration,
) -> SysprimsResult<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(true) => return Ok(true),
            Ok(false) if Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            Ok(false) => return Ok(false),
            Err(error) => {
                return Err(SysprimsError::system(
                    format!("failed to reap contained child: {error}"),
                    error.raw_os_error().unwrap_or(0),
                ));
            }
        }
    }
}

fn wait_for_contained_child_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match sysprims_proc::is_live(pid) {
            Ok(false) | Err(SysprimsError::NotFound { .. }) => return true,
            Ok(true) | Err(_) if Instant::now() < deadline => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Ok(true) | Err(_) => return false,
        }
    }
}

fn reap_contained_child<C: ContainmentChild>(child: &mut C) -> SysprimsResult<bool> {
    child.try_wait().map_err(|error| {
        SysprimsError::system(
            format!("failed to reap contained child: {error}"),
            error.raw_os_error().unwrap_or(0),
        )
    })
}

pub fn spawn_in_group_impl(config: SpawnInGroupConfig) -> SysprimsResult<SpawnInGroupResult> {
    let command = config.argv[0].as_str();
    if command.is_empty() {
        return Err(SysprimsError::invalid_argument(
            "argv[0] (command) must not be empty",
        ));
    }

    let args: Vec<&str> = config.argv.iter().skip(1).map(|s| s.as_str()).collect();

    let mut cmd = Command::new(command);
    cmd.args(args);

    if let Some(cwd) = config.cwd.as_deref() {
        if !cwd.is_empty() {
            cmd.current_dir(cwd);
        }
    }

    if let Some(env) = config.env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }

    // New process group: child becomes leader (pid == pgid).
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SysprimsError::not_found_command(command)
        } else if e.kind() == std::io::ErrorKind::PermissionDenied {
            SysprimsError::permission_denied_command(command)
        } else {
            SysprimsError::spawn_failed(command, e.to_string())
        }
    })?;

    let pid = child.id();

    Ok(SpawnInGroupResult {
        schema_id: SPAWN_IN_GROUP_RESULT_V1,
        timestamp: sysprims_core::time::now_rfc3339(),
        platform: get_platform(),
        pid,
        pgid: Some(pid),
        tree_kill_reliability: "best_effort".to_string(),
        warnings: vec![
            "PID-only grouped spawn does not retain an owned child and sealed receipt".to_string(),
        ],
    })
}

pub fn run_with_timeout_impl(
    command: &str,
    args: &[&str],
    timeout: Duration,
    config: &TimeoutConfig,
) -> SysprimsResult<TimeoutOutcome> {
    let mut cmd = Command::new(command);
    cmd.args(args);

    if config.grouping == GroupingMode::GroupByDefault {
        return run_contained_with_timeout(cmd, timeout, config);
    }

    let mut child = spawn_timeout_child(cmd, command)?;
    let child_pid = child.id() as i32;
    let start = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(TimeoutOutcome::Completed {
                    exit_status: status,
                });
            }
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(POLL_INTERVAL),
            Ok(None) => return kill_foreground_child(child_pid, &mut child, config),
            Err(error) => {
                return Err(SysprimsError::system(
                    format!("wait failed: {error}"),
                    error.raw_os_error().unwrap_or(0),
                ));
            }
        }
    }
}

fn spawn_timeout_child(mut command: Command, program: &str) -> SysprimsResult<Child> {
    command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SysprimsError::not_found_command(program)
        } else if error.kind() == std::io::ErrorKind::PermissionDenied {
            SysprimsError::permission_denied_command(program)
        } else {
            SysprimsError::spawn_failed(program, error.to_string())
        }
    })
}

struct StatusChild {
    child: Child,
    exit_status: Option<ExitStatus>,
}

impl ContainmentChild for StatusChild {
    fn process_id(&self) -> Option<u32> {
        Some(self.child.id())
    }

    fn try_wait(&mut self) -> std::io::Result<bool> {
        if self.exit_status.is_some() {
            return Ok(true);
        }
        self.exit_status = self.child.try_wait()?;
        Ok(self.exit_status.is_some())
    }
}

fn run_contained_with_timeout(
    mut command: Command,
    timeout: Duration,
    config: &TimeoutConfig,
) -> SysprimsResult<TimeoutOutcome> {
    let program = command.get_program().to_string_lossy().into_owned();
    let (hook, pending_receipt) = sysprims_session::prepare_session_acquisition()?;
    // SAFETY: this is the command's sole pre-exec session/group acquisition
    // hook, prepared before fork and restricted to async-signal-safe work.
    unsafe {
        command.pre_exec(move || hook.acquire());
    }

    let child = spawn_timeout_child(command, &program)?;
    let receipt = match pending_receipt.into_receipt(child.id()) {
        Ok(receipt) => receipt,
        Err(error) => {
            cleanup_failed_spawn_child(child);
            return Err(error);
        }
    };
    let child = StatusChild {
        child,
        exit_status: None,
    };
    let mut guard = match contain_acquired_session_impl(child, receipt) {
        Ok(guard) => guard,
        Err(adoption) => {
            cleanup_failed_spawn_child(adoption.child.child);
            return Err(adoption.error);
        }
    };

    let termination = TerminateTreeConfig {
        grace_timeout_ms: duration_millis_u64(config.kill_after),
        kill_timeout_ms: 2_000,
        signal: config.signal,
        kill_signal: SIGKILL,
    };
    let start = Instant::now();
    loop {
        if guard.try_complete(termination.clone())?.is_some() {
            let child = guard.into_child().map_err(|_| {
                SysprimsError::system("completed containment guard remained active", 0)
            })?;
            let exit_status = child.exit_status.ok_or_else(|| {
                SysprimsError::system("contained child completed without an exit status", 0)
            })?;
            return Ok(TimeoutOutcome::Completed { exit_status });
        }
        if start.elapsed() >= timeout {
            let outcome = guard.terminate(termination)?;
            // The legacy timeout contract reports escalation when the
            // post-grace force-kill phase was reached, even if the final
            // group signal found no remaining signalable member.
            let escalated = outcome.escalated || outcome.signal_sent.is_some();
            return Ok(TimeoutOutcome::TimedOut {
                signal_sent: config.signal,
                escalated,
                tree_kill_reliability: outcome.tree_kill_reliability,
            });
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn kill_foreground_child(
    pid: i32,
    child: &mut Child,
    config: &TimeoutConfig,
) -> SysprimsResult<TimeoutOutcome> {
    let reliability = TreeKillReliability::BestEffort;

    let _ = sysprims_signal::kill(pid as u32, config.signal);
    let escalation_deadline = Instant::now() + config.kill_after;
    while Instant::now() < escalation_deadline {
        if child.try_wait().ok().flatten().is_some() {
            return Ok(TimeoutOutcome::TimedOut {
                signal_sent: config.signal,
                escalated: false,
                tree_kill_reliability: reliability,
            });
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    let _ = sysprims_signal::force_kill(pid as u32);
    let _ = child.wait();
    Ok(TimeoutOutcome::TimedOut {
        signal_sent: config.signal,
        escalated: true,
        tree_kill_reliability: reliability,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::path::{Path, PathBuf};

    struct SequencedIdentityChild {
        child: Child,
        pid: u32,
        process_id_calls: Cell<usize>,
        unavailable_on_call: usize,
    }

    impl ContainmentChild for SequencedIdentityChild {
        fn process_id(&self) -> Option<u32> {
            let call = self.process_id_calls.get() + 1;
            self.process_id_calls.set(call);
            (call != self.unavailable_on_call).then_some(self.pid)
        }

        fn try_wait(&mut self) -> std::io::Result<bool> {
            self.child.try_wait().map(|status| status.is_some())
        }
    }

    fn spawn_with_session_receipt(mut command: Command) -> (Child, UnixSessionReceipt) {
        let (hook, pending) = sysprims_session::prepare_session_acquisition().unwrap();
        // SAFETY: the helper installs the prepared hook as the command's sole
        // child session/group acquirer.
        unsafe {
            command.pre_exec(move || hook.acquire());
        }
        let child = command.spawn().unwrap();
        let receipt = pending.into_receipt(child.id()).unwrap();
        (child, receipt)
    }

    fn exec_gate_path(scene: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sysprims-timeout-exec-gate-{}-{scene}",
            std::process::id()
        ))
    }

    fn command_waiting_for_two_execs(gate: &Path, final_script: &str) -> Command {
        let mut command = Command::new("sh");
        // After acquisition, the leader execs `env`, which then execs `bash`.
        // This keeps the regression deterministic across two executable-image
        // transitions before the requested scene begins.
        command.args([
            "-c",
            "while [ ! -e \"$1\" ]; do sleep 0.01; done; exec env bash -c \"$2\"",
            "sysprims-exec-gate",
            gate.to_str().expect("temporary path must be UTF-8"),
            final_script,
        ]);
        command
    }

    fn open_exec_gate(gate: &Path) {
        std::fs::write(gate, b"continue").expect("failed to open exec gate");
    }

    fn remove_exec_gate(gate: &Path) {
        let _ = std::fs::remove_file(gate);
    }

    fn wait_for_executable_change(pid: u32, original: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if sysprims_proc::get_process(pid)
                .ok()
                .and_then(|process| process.exe_path)
                .is_some_and(|exe| exe != original)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "receipt-bound child did not cross the exec gate"
            );
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    #[test]
    fn receipt_specific_entry_point_creates_one_shot_guaranteed_guard() {
        let mut command = Command::new("sleep");
        command.arg("60");
        let (child, receipt) = spawn_with_session_receipt(command);
        let mut guard =
            contain_acquired_session_impl(child, receipt).expect("receipt consumption failed");
        assert_eq!(
            guard.tree_kill_reliability(),
            TreeKillReliability::Guaranteed
        );

        let outcome = guard
            .terminate(TerminateTreeConfig {
                grace_timeout_ms: 0,
                kill_timeout_ms: 500,
                ..TerminateTreeConfig::default()
            })
            .expect("contained termination failed");
        assert!(outcome.exited);
        assert!(guard.terminate(TerminateTreeConfig::default()).is_err());
        assert!(guard.into_child().is_ok());
    }

    #[test]
    fn receipt_bound_explicit_termination_allows_multiple_execs() {
        let gate = exec_gate_path("explicit");
        remove_exec_gate(&gate);
        let command = command_waiting_for_two_execs(&gate, "sleep 60 & wait");
        let mut guard = spawn_contained_impl(command).expect("contained spawn failed");
        let original_exe = guard.identity().exe_path.clone();

        open_exec_gate(&gate);
        wait_for_executable_change(guard.identity().pid, &original_exe);
        remove_exec_gate(&gate);

        let outcome = guard
            .terminate(TerminateTreeConfig {
                grace_timeout_ms: 10,
                kill_timeout_ms: 500,
                ..TerminateTreeConfig::default()
            })
            .expect("receipt-bound termination must survive multiple execs");
        assert_eq!(
            outcome.tree_kill_reliability,
            TreeKillReliability::Guaranteed
        );
        assert!(matches!(
            outcome.completion,
            ContainmentCompletionEvidence::Empty { .. }
        ));
        assert!(outcome.exited);

        let mut child = match guard.into_child() {
            Ok(child) => child,
            Err(_) => panic!("finalized guard must return child"),
        };
        assert!(child
            .try_wait()
            .expect("reaped child status must remain available")
            .is_some());
    }

    #[test]
    fn receipt_bound_natural_completion_allows_multiple_execs() {
        let gate = exec_gate_path("natural");
        remove_exec_gate(&gate);
        let command = command_waiting_for_two_execs(&gate, "sleep 60 & exit 0");
        let mut guard = spawn_contained_impl(command).expect("contained spawn failed");

        open_exec_gate(&gate);

        let deadline = Instant::now() + Duration::from_secs(5);
        let outcome = loop {
            if let Some(outcome) = guard
                .try_complete(TerminateTreeConfig {
                    grace_timeout_ms: 10,
                    kill_timeout_ms: 500,
                    ..TerminateTreeConfig::default()
                })
                .expect("receipt-bound completion must survive multiple execs")
            {
                break outcome;
            }
            assert!(
                Instant::now() < deadline,
                "receipt-bound leader did not exit naturally"
            );
            std::thread::sleep(POLL_INTERVAL);
        };
        remove_exec_gate(&gate);

        assert_eq!(
            outcome.tree_kill_reliability,
            TreeKillReliability::Guaranteed
        );
        assert!(matches!(
            outcome.completion,
            ContainmentCompletionEvidence::Empty { .. }
        ));
        assert!(outcome.exited);

        let mut child = match guard.into_child() {
            Ok(child) => child,
            Err(_) => panic!("finalized guard must return child"),
        };
        assert!(child
            .try_wait()
            .expect("reaped child status must remain available")
            .is_some());
    }

    #[test]
    fn receipt_bound_exit_between_identity_and_group_lookup_uses_fast_exit() {
        let ownership_rechecked = std::cell::Cell::new(false);
        let identity_metadata_verified =
            reconcile_group_identity_after_live_check(true, 42, 42, -1, -1, || {
                ownership_rechecked.set(true);
                Ok(true)
            })
            .expect("matching exited-unreaped ownership must preserve containment");

        assert!(!identity_metadata_verified);
        assert!(ownership_rechecked.get());
    }

    #[test]
    fn post_validation_signal_failure_requires_empty_completion() {
        let observation = ContainmentObservation::LinuxProcfsProcessGroup;
        let warning = reconcile_group_signal_failure(
            SysprimsError::not_found(42),
            || ContainmentCompletionEvidence::Empty { observation },
            "empty group",
        )
        .expect("an empty observed group may proceed to exact-child reap");
        assert_eq!(warning, "empty group");

        let survivors = reconcile_group_signal_failure(
            SysprimsError::not_found(42),
            || ContainmentCompletionEvidence::Survivors {
                observation,
                observed_count: 1,
                survivor_pids: vec![43],
            },
            "must not be returned",
        )
        .expect_err("live survivors must preserve the signal failure");
        assert!(matches!(survivors, SysprimsError::NotFound { .. }));

        let unknown = reconcile_group_signal_failure(
            SysprimsError::not_found(42),
            || ContainmentCompletionEvidence::Unknown { observation },
            "must not be returned",
        )
        .expect_err("unknown completion must preserve the signal failure");
        assert!(matches!(unknown, SysprimsError::NotFound { .. }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_permission_denied_signal_failure_requires_empty_completion() {
        let warning = reconcile_group_signal_failure(
            SysprimsError::permission_denied(42, "signal_group"),
            || ContainmentCompletionEvidence::Empty {
                observation: ContainmentObservation::MacosLibprocProcessGroup,
            },
            "empty group",
        )
        .expect("macOS EPERM with an empty observed group may proceed to reap");
        assert_eq!(warning, "empty group");
    }

    #[test]
    fn live_group_mismatch_after_identity_check_still_fails_closed() {
        let error = reconcile_group_identity_after_live_check(true, 42, 42, 41, 42, || Ok(false))
            .expect_err("live group mismatch must not become a fast-exit path");

        assert!(matches!(error, SysprimsError::InvalidArgument { .. }));
    }

    #[test]
    fn live_session_mismatch_after_identity_check_still_fails_closed() {
        let error = reconcile_group_identity_after_live_check(true, 42, 42, 42, 41, || Ok(false))
            .expect_err("live session mismatch must not become a fast-exit path");

        assert!(matches!(error, SysprimsError::InvalidArgument { .. }));
    }

    #[test]
    fn escalation_revalidates_the_owned_child_before_signaling() {
        let mut command = Command::new("sleep");
        command.arg("60");
        let (child, receipt) = spawn_with_session_receipt(command);
        let pid = child.id();
        let child = SequencedIdentityChild {
            child,
            pid,
            process_id_calls: Cell::new(0),
            unavailable_on_call: 3,
        };
        let mut guard =
            contain_acquired_session_impl(child, receipt).expect("receipt consumption failed");

        let error = guard
            .terminate(TerminateTreeConfig {
                signal: libc::SIGSTOP,
                kill_signal: libc::SIGCONT,
                grace_timeout_ms: 0,
                kill_timeout_ms: 100,
            })
            .expect_err("lost child identity must stop pre-escalation signaling");
        assert!(matches!(error, SysprimsError::InvalidArgument { .. }));
    }

    #[test]
    fn receipt_bound_termination_exit_race_stress() {
        for _ in 0..32 {
            let mut command = Command::new("sleep");
            command.arg("60");
            let (child, receipt) = spawn_with_session_receipt(command);
            let mut guard = contain_acquired_session_impl(child, receipt)
                .expect("receipt consumption failed during stress run");

            let outcome = guard
                .terminate(TerminateTreeConfig {
                    grace_timeout_ms: 0,
                    kill_timeout_ms: 500,
                    ..TerminateTreeConfig::default()
                })
                .expect("receipt-bound termination lost the exit race");
            assert!(outcome.exited);
        }
    }

    #[test]
    fn wrong_child_consumes_receipt_and_returns_child_ownership() {
        let mut acquired_command = Command::new("sleep");
        acquired_command.arg("60");
        let (mut acquired_child, receipt) = spawn_with_session_receipt(acquired_command);
        let wrong_child = Command::new("sleep").arg("60").spawn().unwrap();
        let wrong_pid = wrong_child.id();

        let error = match contain_acquired_session_impl(wrong_child, receipt) {
            Ok(_) => panic!("receipt must not pair with a different child"),
            Err(error) => error,
        };
        assert_eq!(error.child.id(), wrong_pid);

        let mut returned_child = error.child;
        returned_child.kill().unwrap();
        returned_child.wait().unwrap();
        acquired_child.kill().unwrap();
        acquired_child.wait().unwrap();
    }

    #[test]
    fn reaped_child_cannot_consume_receipt_as_guaranteed() {
        let (mut child, receipt) = spawn_with_session_receipt(Command::new("true"));
        child.wait().unwrap();

        let error = match contain_acquired_session_impl(child, receipt) {
            Ok(_) => panic!("reaped child must not retain guaranteed ownership"),
            Err(error) => error,
        };
        assert!(matches!(error.error, SysprimsError::InvalidArgument { .. }));
    }

    #[test]
    fn guaranteed_guard_fails_closed_after_external_reap() {
        let mut command = Command::new("sleep");
        command.arg("60");
        let (child, receipt) = spawn_with_session_receipt(command);
        let mut guard = contain_acquired_session_impl(child, receipt).unwrap();
        let child = guard.child.as_mut().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();

        let error = guard
            .terminate(TerminateTreeConfig::default())
            .expect_err("lost reap ownership must fail before group signaling");
        assert!(matches!(error, SysprimsError::InvalidArgument { .. }));
    }

    #[test]
    fn fast_exit_receipt_never_loses_child_ownership() {
        let (child, receipt) = spawn_with_session_receipt(Command::new("true"));
        let mut guard =
            contain_acquired_session_impl(child, receipt).expect("fast exit must remain owned");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if guard
                .try_complete(TerminateTreeConfig {
                    grace_timeout_ms: 0,
                    kill_timeout_ms: 100,
                    ..TerminateTreeConfig::default()
                })
                .unwrap()
                .is_some()
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(POLL_INTERVAL);
        }
        assert!(guard.into_child().is_ok());
    }

    #[test]
    fn timeout_completes_fast_command() {
        let result = run_with_timeout_impl(
            "echo",
            &["hello"],
            Duration::from_secs(10),
            &TimeoutConfig::default(),
        )
        .unwrap();

        assert!(matches!(result, TimeoutOutcome::Completed { .. }));
    }

    #[test]
    fn timeout_triggers_on_slow_command() {
        let result = run_with_timeout_impl(
            "sleep",
            &["60"],
            Duration::from_millis(100),
            &TimeoutConfig {
                kill_after: Duration::from_millis(100),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(matches!(result, TimeoutOutcome::TimedOut { .. }));
    }

    #[test]
    fn timeout_returns_not_found_for_missing_command() {
        let result = run_with_timeout_impl(
            "nonexistent_command_12345",
            &[],
            Duration::from_secs(10),
            &TimeoutConfig::default(),
        );

        assert!(matches!(result, Err(SysprimsError::NotFoundCommand { .. })));
    }

    #[test]
    fn pid_only_grouped_spawn_is_not_reported_as_guaranteed() {
        let result = spawn_in_group_impl(SpawnInGroupConfig {
            argv: vec!["true".to_string()],
            cwd: None,
            env: None,
        })
        .unwrap();

        assert_eq!(result.tree_kill_reliability, "best_effort");
        assert!(!result.warnings.is_empty());
        // SAFETY: this process is our child; waitpid supplies its only reap.
        unsafe {
            libc::waitpid(result.pid as libc::pid_t, std::ptr::null_mut(), 0);
        }
    }

    #[test]
    fn normal_completion_is_observed_without_early_reap() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 0.1"]);
        let mut guard = spawn_contained_impl(command).expect("contained spawn failed");
        let deadline = Instant::now() + Duration::from_secs(5);

        loop {
            if let Some(outcome) = guard
                .try_complete(TerminateTreeConfig {
                    grace_timeout_ms: 0,
                    kill_timeout_ms: 500,
                    ..TerminateTreeConfig::default()
                })
                .expect("normal completion failed")
            {
                assert!(outcome.exited);
                assert!(matches!(
                    outcome.completion,
                    ContainmentCompletionEvidence::Empty { .. }
                ));
                break;
            }
            assert!(Instant::now() < deadline, "contained child did not exit");
            std::thread::sleep(POLL_INTERVAL);
        }

        assert!(guard.into_child().is_ok());
    }

    #[test]
    fn completion_observation_requires_a_stable_snapshot() {
        let mut attempts = 0;
        let completion =
            stabilize_completion(ContainmentObservation::LinuxProcfsProcessGroup, || {
                attempts += 1;
                Ok(if attempts == 1 {
                    vec![41]
                } else {
                    vec![29, 41]
                })
            });
        assert_eq!(attempts, 3);
        assert_eq!(
            completion,
            ContainmentCompletionEvidence::Survivors {
                observation: ContainmentObservation::LinuxProcfsProcessGroup,
                observed_count: 2,
                survivor_pids: vec![29, 41],
            }
        );
    }

    #[test]
    fn completion_observation_failure_is_unknown() {
        assert_eq!(
            stabilize_completion(ContainmentObservation::LinuxProcfsProcessGroup, || Err(())),
            ContainmentCompletionEvidence::Unknown {
                observation: ContainmentObservation::LinuxProcfsProcessGroup,
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_stat_membership_parser_handles_complex_command_names() {
        assert_eq!(
            parse_linux_stat_membership(b"17 (worker ) pool) S 3 17 9 0 0"),
            Ok(('S', 17, 9))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_stat_membership_parser_ignores_non_utf8_command_names() {
        let stat = b"17 (worker \xff pool) S 3 17 9 0 0";
        assert_eq!(parse_linux_stat_membership(stat), Ok(('S', 17, 9)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_procfs_visibility_rejects_restrictive_or_unverifiable_mounts() {
        let unrestricted = b"29 24 0:25 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw\n";
        let explicit_unrestricted = b"29 24 0:25 / /proc rw,nosuid,nodev,noexec,relatime,hidepid=0 - proc proc rw,hidepid=0\n";
        let omitted_pid_visibility =
            b"29 24 0:25 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw,hidepid=2\n";
        let bind_mount = b"29 24 0:25 /123 /proc rw - proc proc rw\n";
        let masked_pid =
            b"29 24 0:25 / /proc rw - proc proc rw\n30 29 0:26 / /proc/123 rw - tmpfs tmpfs rw\n";

        assert!(linux_procfs_visibility_is_complete(unrestricted, 29));
        assert!(linux_procfs_visibility_is_complete(
            explicit_unrestricted,
            29
        ));
        assert!(!linux_procfs_visibility_is_complete(
            omitted_pid_visibility,
            29
        ));
        assert!(!linux_procfs_visibility_is_complete(bind_mount, 29));
        assert!(!linux_procfs_visibility_is_complete(masked_pid, 29));
        assert!(!linux_procfs_visibility_is_complete(
            b"29 24 0:25 / /host-proc rw - proc proc rw\n",
            29
        ));
        assert!(!linux_procfs_visibility_is_complete(b"malformed", 29));
        assert!(!linux_procfs_visibility_is_complete(unrestricted, 30));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_restrictive_visibility_is_unknown_without_enumeration() {
        let completion = observe_linux_completion_with(false, || {
            panic!("restricted procfs must not be enumerated")
        });
        assert_eq!(
            completion,
            ContainmentCompletionEvidence::Unknown {
                observation: ContainmentObservation::LinuxProcfsProcessGroup,
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_procfs_mount_id_parser_reads_effective_mount() {
        assert_eq!(
            linux_procfs_mount_id(b"pos:\t0\nflags:\t0100000\nmnt_id:\t29\n"),
            Some(29)
        );
        assert_eq!(linux_procfs_mount_id(b"mnt_id:\tnot-a-number\n"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_procfs_identity_must_match_caller_namespace() {
        let stat = b"17 (observer) S 3 17 9 0 0";
        assert!(linux_procfs_identity_matches(stat, 17, 17, 9));
        assert!(!linux_procfs_identity_matches(stat, 1, 17, 9));
        assert!(!linux_procfs_identity_matches(stat, 17, 4, 9));
        assert!(!linux_procfs_identity_matches(stat, 17, 17, 5));
        assert!(!linux_procfs_identity_matches(b"malformed", 17, 17, 9));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_full_buffer_race_exhausts_bounded_retries() {
        let mut fetches = 0;
        let result = macos_group_snapshot_with(
            |buffer| match buffer {
                None => Ok(1),
                Some(buffer) => {
                    fetches += 1;
                    Ok(buffer.len())
                }
            },
            |_| Ok(true),
        );
        assert_eq!(fetches, MAX_COMPLETION_OBSERVATION_RETRIES + 1);
        assert_eq!(result, Err(()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_snapshot_returns_complete_live_members() {
        let result = macos_group_snapshot_with(
            |buffer| match buffer {
                None => Ok(2),
                Some(buffer) => {
                    buffer[0] = 23;
                    buffer[1] = 17;
                    Ok(2)
                }
            },
            |_| Ok(true),
        );
        assert_eq!(result, Ok(vec![23, 17]));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_snapshot_failure_is_unknown_completion() {
        let completion =
            stabilize_completion(ContainmentObservation::MacosLibprocProcessGroup, || {
                macos_group_snapshot_with(
                    |_| Err(()),
                    |_| panic!("liveness must not run after enumeration failure"),
                )
            });
        assert_eq!(
            completion,
            ContainmentCompletionEvidence::Unknown {
                observation: ContainmentObservation::MacosLibprocProcessGroup,
            }
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_owned_group_reports_survivors_and_guard_drop_cleans_them() {
        use std::process::Stdio;

        let mut command = Command::new("sleep");
        command
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut guard = spawn_contained_impl(command).expect("contained spawn failed");
        let pgid = guard.identity.pid;
        let outcome = guard
            .terminate(TerminateTreeConfig {
                grace_timeout_ms: 10,
                kill_timeout_ms: 25,
                signal: libc::SIGSTOP,
                kill_signal: libc::SIGSTOP,
            })
            .expect("contained observation failed");

        match outcome.completion {
            ContainmentCompletionEvidence::Survivors {
                observation,
                observed_count,
                survivor_pids,
            } => {
                assert_eq!(
                    observation,
                    ContainmentObservation::MacosLibprocProcessGroup
                );
                assert_eq!(observed_count as usize, survivor_pids.len());
                assert!(survivor_pids.contains(&pgid));
            }
            other => panic!("expected survivor evidence, got {other:?}"),
        }
        assert!(!outcome.exited);
        assert!(outcome.timed_out);

        drop(guard);
        wait_for_macos_group_empty(pgid);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_owned_group_cleans_descendants_after_leader_exit() {
        use std::io::{BufRead, BufReader};
        use std::process::Stdio;

        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "(trap '' TERM; printf 'descendant-ready\\n'; sleep 60) & sleep 0.2",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut guard = spawn_contained_impl(command).expect("contained spawn failed");
        let pgid = guard.identity.pid;
        let stdout = guard
            .child
            .as_mut()
            .and_then(|child| child.stdout.take())
            .expect("contained child stdout unavailable");
        let mut readiness = String::new();
        BufReader::new(stdout)
            .read_line(&mut readiness)
            .expect("failed to read descendant readiness");
        assert_eq!(readiness, "descendant-ready\n");
        assert!(
            macos_group_snapshot(pgid)
                .expect("failed to observe ready descendant")
                .into_iter()
                .any(|pid| pid != pgid),
            "ready descendant was not live in the owned group"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let outcome = loop {
            if let Some(outcome) = guard
                .try_complete(TerminateTreeConfig {
                    grace_timeout_ms: 10,
                    kill_timeout_ms: 500,
                    ..TerminateTreeConfig::default()
                })
                .expect("contained completion failed")
            {
                break outcome;
            }
            assert!(Instant::now() < deadline, "group leader did not exit");
            std::thread::sleep(POLL_INTERVAL);
        };

        assert!(outcome.exited);
        assert!(matches!(
            outcome.completion,
            ContainmentCompletionEvidence::Empty {
                observation: ContainmentObservation::MacosLibprocProcessGroup,
            }
        ));
        wait_for_macos_group_empty(pgid);
        assert!(guard.into_child().is_ok());
    }

    #[cfg(target_os = "macos")]
    fn wait_for_macos_group_empty(pgid: u32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if matches!(macos_group_snapshot(pgid), Ok(members) if members.is_empty()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "owned macOS process group still has live members"
            );
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    #[test]
    fn foreground_mode_does_not_create_process_group() {
        let config = TimeoutConfig {
            grouping: GroupingMode::Foreground,
            kill_after: Duration::from_millis(100),
            ..Default::default()
        };

        let result =
            run_with_timeout_impl("sleep", &["60"], Duration::from_millis(100), &config).unwrap();

        if let TimeoutOutcome::TimedOut {
            tree_kill_reliability,
            ..
        } = result
        {
            assert_eq!(tree_kill_reliability, TreeKillReliability::BestEffort);
        } else {
            panic!("Expected timeout");
        }
    }
}
