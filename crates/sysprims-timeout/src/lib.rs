//! sysprims-timeout: Process timeout with group-by-default tree management.
//!
//! This crate provides:
//! - Process execution with timeout ([`run_with_timeout`])
//! - Group-by-default semantics (cooperative process groups signaled on timeout)
//! - Signal escalation (SIGTERM → SIGKILL after configurable delay)
//! - Observable fallback status for tree-kill reliability
//!
//! # Group-by-Default
//!
//! The core differentiator of sysprims over GNU timeout. When a command times
//! out, the acquired process group is signaled, not just the direct child:
//!
//! - **Unix**: Creates a new process group; child is group leader
//! - **Windows**: Creates a Job Object with `KILL_ON_JOB_CLOSE`
//!
//! This gives race-free group acquisition and signaling eligibility. It is not
//! an OS-enforced promise that descendants cannot later leave the group.
//!
//! # Examples
//!
//! ```no_run
//! use std::time::Duration;
//! use sysprims_timeout::{run_with_timeout, TimeoutConfig, TimeoutOutcome};
//!
//! let result = run_with_timeout(
//!     "sleep",
//!     &["60"],
//!     Duration::from_secs(5),
//!     TimeoutConfig::default(),
//! ).unwrap();
//!
//! match result {
//!     TimeoutOutcome::Completed { exit_status } => {
//!         println!("Command completed: {:?}", exit_status);
//!     }
//!     TimeoutOutcome::TimedOut { signal_sent, escalated, .. } => {
//!         println!("Timed out, sent signal {}, escalated: {}", signal_sent, escalated);
//!     }
//! }
//! ```

use std::process::{Child, Command, ExitStatus};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sysprims_core::schema::TERMINATE_TREE_RESULT_V1;
use sysprims_core::time::now_rfc3339;
use sysprims_core::{get_platform, SysprimsError, SysprimsResult};
use sysprims_proc::wait_pid;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

// Re-export signal constants for convenience
pub use sysprims_signal::{SIGKILL, SIGTERM};
#[cfg(windows)]
pub use windows::{PreparedWindowsJob, WindowsJobReceipt};

/// Process grouping strategy.
///
/// Controls whether timeout creates a process group (Unix) or Job Object
/// (Windows) to enable tree-kill on timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupingMode {
    /// Create new process group (Unix) or Job Object (Windows).
    /// Signal the acquired group/job on timeout. **This is the default.**
    #[default]
    GroupByDefault,

    /// Run in foreground. Only kills direct child on timeout.
    /// Use when the child must inherit the parent's process group.
    Foreground,
}

/// Configuration for timeout execution.
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    /// Initial signal to send on timeout.
    ///
    /// Default: `SIGTERM` (15)
    pub signal: i32,

    /// Delay before escalating to SIGKILL if process doesn't exit.
    ///
    /// Default: 10 seconds
    pub kill_after: Duration,

    /// Process grouping strategy.
    ///
    /// Default: `GroupByDefault`
    pub grouping: GroupingMode,

    /// Propagate child exit code when command completes normally.
    ///
    /// When `true`, the timeout exit code matches the child's exit code.
    /// When `false`, successful completion returns exit code 0.
    ///
    /// Default: `false`
    pub preserve_status: bool,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            signal: SIGTERM,
            kill_after: Duration::from_secs(10),
            grouping: GroupingMode::GroupByDefault,
            preserve_status: false,
        }
    }
}

/// Reliability of tree-kill operation.
///
/// Distinguishes proven spawn-time acquisition, post-spawn containment with an
/// escape window, and direct-child best effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeKillReliability {
    /// Spawn-time containment acquisition and group-signaling eligibility were
    /// proven. On Unix, descendants can still leave a cooperative process
    /// group later; this is not a non-escape guarantee.
    Guaranteed,

    /// A containment capability is owned, but it was attached after the child
    /// started and descendants may have escaped before attachment completed.
    Unproven,

    /// Best-effort only. No owned, proven containment capability is retained;
    /// coverage may be limited to the direct child.
    BestEffort,
}

/// Strength of the acquired containment boundary.
///
/// This is independent of [`TreeKillReliability`]. Reliability describes when
/// termination authority was acquired; boundary strength describes whether
/// descendants are constrained by a cooperative group or a kernel-enforced
/// non-breakaway Job.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainmentBoundaryStrength {
    /// The exact Windows child was assigned before first execution to an owned
    /// Job configured without either breakaway mode.
    KernelEnforcedJob,
    /// A Unix session/process-group boundary. Descendants may later leave.
    CooperativeGroup,
    /// The stronger boundary property was not proven.
    Unknown,
}

pub(crate) const MAX_COMPLETION_OBSERVED_PIDS: usize = 4096;
pub(crate) const MAX_COMPLETION_OBSERVATION_RETRIES: usize = 3;

/// Read-only mechanism used to observe containment membership.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainmentObservation {
    /// Linux `/proc` process-group and session scan.
    LinuxProcfsProcessGroup,
    /// macOS libproc process-group scan.
    MacosLibprocProcessGroup,
    /// Windows Job process-ID-list query.
    WindowsJobProcessIdList,
    /// No trustworthy platform observation mechanism was available.
    UnsupportedPlatform,
}

/// Point-in-time evidence observed after the final containment cleanup action.
///
/// This evidence is independent of leader reap and containment acquisition
/// reliability. Survivor PIDs are evidence only: PID reuse makes them unsafe to
/// pass to process-signaling APIs or treat as suggested cleanup targets.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ContainmentCompletionEvidence {
    /// A complete supported observation found no live containment members.
    Empty { observation: ContainmentObservation },
    /// A complete supported observation found live containment members.
    Survivors {
        observation: ContainmentObservation,
        observed_count: u32,
        survivor_pids: Vec<u32>,
    },
    /// A complete, trustworthy observation was unavailable.
    Unknown { observation: ContainmentObservation },
}

pub(crate) fn unknown_completion(
    observation: ContainmentObservation,
) -> ContainmentCompletionEvidence {
    ContainmentCompletionEvidence::Unknown { observation }
}

pub(crate) fn completion_from_pids(
    observation: ContainmentObservation,
    mut pids: Vec<u32>,
) -> ContainmentCompletionEvidence {
    if pids.len() >= MAX_COMPLETION_OBSERVED_PIDS
        || pids
            .iter()
            .any(|pid| *pid == 0 || *pid > sysprims_signal::MAX_SAFE_PID)
    {
        return unknown_completion(observation);
    }

    let observed_count = pids.len();
    pids.sort_unstable();
    pids.dedup();
    if pids.len() != observed_count {
        return unknown_completion(observation);
    }
    if pids.is_empty() {
        ContainmentCompletionEvidence::Empty { observation }
    } else {
        ContainmentCompletionEvidence::Survivors {
            observation,
            observed_count: pids.len() as u32,
            survivor_pids: pids,
        }
    }
}

impl TreeKillReliability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Guaranteed => "guaranteed",
            Self::Unproven => "unproven",
            Self::BestEffort => "best_effort",
        }
    }
}

impl ContainmentBoundaryStrength {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KernelEnforcedJob => "kernel_enforced_job",
            Self::CooperativeGroup => "cooperative_group",
            Self::Unknown => "unknown",
        }
    }
}

/// Stable process evidence captured when containment is acquired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainmentIdentity {
    pub pid: u32,
    /// Zero only when a sealed same-spawn receipt is paired with an already
    /// exited, unreaped child before live metadata can be captured.
    pub start_time_unix_ms: u64,
    /// Observational executable metadata captured when containment is acquired.
    ///
    /// Unix guards backed by a sealed same-spawn receipt allow this value to
    /// change across `exec` and may leave it empty when it is unavailable.
    /// Adopted guards require it to remain available and unchanged. It is also
    /// empty for the receipt-bound fast-exit state described above.
    pub exe_path: String,
}

/// Minimal owned-child contract used by [`ContainmentGuard`].
///
/// Consumers can wrap an external child type without exposing PTY handles or
/// terminal data to sysprims. The adapter must retain exclusive reap ownership
/// from receipt consumption through guard finalization; `process_id` must name
/// that unreaped child for the adapter's lifetime.
pub trait ContainmentChild {
    fn process_id(&self) -> Option<u32>;
    /// Poll and reap the child when it has exited.
    fn try_wait(&mut self) -> std::io::Result<bool>;

    #[cfg(windows)]
    fn raw_process_handle(&self) -> Option<std::os::windows::io::RawHandle>;
}

impl ContainmentChild for Child {
    fn process_id(&self) -> Option<u32> {
        Some(self.id())
    }

    fn try_wait(&mut self) -> std::io::Result<bool> {
        Child::try_wait(self).map(|status| status.is_some())
    }

    #[cfg(windows)]
    fn raw_process_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        use std::os::windows::io::AsRawHandle;
        Some(self.as_raw_handle())
    }
}

/// Error returned when containment cannot be acquired.
///
/// The child is returned so acquisition failure never silently loses the
/// caller's only reap handle.
pub struct ContainmentAdoptionError<C> {
    pub error: SysprimsError,
    pub child: C,
}

impl<C> std::fmt::Debug for ContainmentAdoptionError<C> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContainmentAdoptionError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl<C> std::fmt::Display for ContainmentAdoptionError<C> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl<C> std::error::Error for ContainmentAdoptionError<C> {}

/// Error returned when an owned contained spawn cannot complete.
#[derive(Debug)]
pub enum ContainmentSpawnError {
    Spawn(SysprimsError),
    Adoption(SysprimsError),
}

impl std::fmt::Display for ContainmentSpawnError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(error) => error.fmt(formatter),
            Self::Adoption(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ContainmentSpawnError {}

/// An owned, stateful containment capability and child lifecycle.
///
/// Termination and normal completion mutably borrow the guard and are one-shot.
/// Dropping an active guard kills the contained tree and makes a bounded attempt
/// to reap the child. The guard never reconstructs containment from a
/// caller-supplied PID.
pub struct ContainmentGuard<C: ContainmentChild> {
    child: Option<C>,
    identity: ContainmentIdentity,
    reliability: TreeKillReliability,
    boundary_strength: ContainmentBoundaryStrength,
    finalized: bool,
    #[cfg(unix)]
    pgid: u32,
    #[cfg(unix)]
    session_id: u32,
    #[cfg(unix)]
    session_receipt: Option<sysprims_session::UnixSessionReceipt>,
    #[cfg(windows)]
    job: std::os::windows::io::OwnedHandle,
}

impl<C: ContainmentChild> ContainmentGuard<C> {
    pub fn identity(&self) -> &ContainmentIdentity {
        &self.identity
    }

    pub fn tree_kill_reliability(&self) -> TreeKillReliability {
        self.reliability
    }

    pub fn boundary_strength(&self) -> ContainmentBoundaryStrength {
        self.boundary_strength
    }

    /// Recover the child adapter after successful termination or completion.
    pub fn into_child(mut self) -> Result<C, Self> {
        if self.finalized {
            Ok(self
                .child
                .take()
                .expect("finalized guard retains its child"))
        } else {
            Err(self)
        }
    }

    /// Finalize a normally exited child without surrendering containment early.
    ///
    /// This observes leader exit without reaping. When the leader has exited,
    /// the method cleans any remaining descendants before reaping and returns
    /// the resulting outcome. It returns `Ok(None)` while the leader is running.
    pub fn try_complete(
        &mut self,
        config: TerminateTreeConfig,
    ) -> SysprimsResult<Option<ContainmentOutcome>> {
        if self.finalized {
            return Err(SysprimsError::invalid_argument(
                "containment guard has already been finalized",
            ));
        }

        #[cfg(unix)]
        if !unix::contained_child_has_exited(self)? {
            return Ok(None);
        }

        #[cfg(windows)]
        if !windows::contained_child_has_exited(self)? {
            return Ok(None);
        }

        self.terminate(config).map(Some)
    }

    /// Terminate the owned containment once, preserving the child until reap.
    pub fn terminate(&mut self, config: TerminateTreeConfig) -> SysprimsResult<ContainmentOutcome> {
        if self.finalized {
            return Err(SysprimsError::invalid_argument(
                "containment guard has already been finalized",
            ));
        }

        #[cfg(unix)]
        return unix::terminate_contained_impl(self, config);

        #[cfg(windows)]
        return windows::terminate_contained_impl(self, config);
    }
}

impl<C: ContainmentChild> Drop for ContainmentGuard<C> {
    fn drop(&mut self) {
        if self.finalized || self.child.is_none() {
            return;
        }

        #[cfg(unix)]
        unix::drop_contained_impl(self);

        #[cfg(windows)]
        windows::drop_contained_impl(self);
    }
}

/// Result from terminating an owned containment guard.
#[derive(Debug, Clone, Serialize)]
pub struct ContainmentOutcome {
    pub identity: ContainmentIdentity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_sent: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kill_signal: Option<i32>,
    pub escalated: bool,
    pub exited: bool,
    pub timed_out: bool,
    pub tree_kill_reliability: TreeKillReliability,
    pub boundary_strength: ContainmentBoundaryStrength,
    /// Point-in-time membership evidence collected before leader reap or Job release.
    pub completion: ContainmentCompletionEvidence,
    pub warnings: Vec<String>,
}

/// Adopt an already-running child into an owned containment guard.
///
/// Post-spawn adoption is reported as [`TreeKillReliability::Unproven`]
/// because descendants may escape before acquisition completes.
pub fn adopt_contained<C: ContainmentChild>(
    child: C,
) -> Result<ContainmentGuard<C>, ContainmentAdoptionError<C>> {
    #[cfg(unix)]
    return unix::adopt_contained_impl(child);

    #[cfg(windows)]
    return windows::adopt_contained_impl(child);
}

/// Consume a sealed same-spawn session receipt with its owned child.
///
/// This is the Unix entry point for an external spawner that installed
/// [`sysprims_session::UnixSessionAcquisitionHook`] in place of its own
/// `setsid`/`setpgid` acquisition. The receipt is destroyed on every failure,
/// and the child is returned through [`ContainmentAdoptionError`].
///
/// # Safety
///
/// `child` must be the same spawned child that emitted `receipt`. The adapter
/// must hold exclusive, unreaped ownership, keep `process_id()` stable, and
/// perform no external wait or reap until the guard finalizes. Violating these
/// requirements can let PID reuse redirect a group signal to an unrelated
/// process. Runtime ownership and identity checks remain defense-in-depth.
#[cfg(unix)]
pub unsafe fn contain_acquired_session<C: ContainmentChild>(
    child: C,
    receipt: sysprims_session::UnixSessionReceipt,
) -> Result<ContainmentGuard<C>, ContainmentAdoptionError<C>> {
    unix::contain_acquired_session_impl(child, receipt)
}

/// Consume a sealed pre-execution Windows Job receipt with its owned child.
///
/// The receipt can only be produced by [`PreparedWindowsJob::assign_process`],
/// which assigns and verifies the exact process in a non-breakaway Job before
/// this transition. The child remains suspended until the adapter has received
/// the guard and explicitly resumes its separately owned primary thread.
///
/// # Safety
///
/// `child` must own the exact process handle passed to
/// [`PreparedWindowsJob::assign_process`]. The adapter must still own the
/// corresponding suspended primary thread, must not have resumed it, and must
/// transfer exclusive wait/reap authority to the returned guard.
#[cfg(windows)]
pub unsafe fn contain_acquired_windows_job<C: ContainmentChild>(
    child: C,
    receipt: WindowsJobReceipt,
) -> Result<ContainmentGuard<C>, ContainmentAdoptionError<C>> {
    windows::contain_acquired_windows_job_impl(child, receipt)
}

/// Spawn a standard-library child and return its owned containment guard.
///
/// Unix uses the same sealed spawn-time session receipt as
/// [`contain_acquired_session`] and reports guaranteed acquisition/group
/// signaling eligibility. Windows fails closed until a create-suspended Job
/// assignment path is available; use [`adopt_contained`] for explicitly
/// unproven adoption.
pub fn spawn_contained(command: Command) -> Result<ContainmentGuard<Child>, ContainmentSpawnError> {
    #[cfg(unix)]
    return unix::spawn_contained_impl(command);

    #[cfg(windows)]
    return windows::spawn_contained_impl(command);
}

fn capture_containment_identity(pid: u32) -> SysprimsResult<ContainmentIdentity> {
    capture_containment_identity_with_mode(pid, ContainmentIdentityValidation::Strict)
}

#[cfg(unix)]
fn capture_receipt_bound_containment_identity(pid: u32) -> SysprimsResult<ContainmentIdentity> {
    capture_containment_identity_with_mode(
        pid,
        ContainmentIdentityValidation::ReceiptBoundExecVolatile,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainmentIdentityValidation {
    Strict,
    #[cfg(unix)]
    ReceiptBoundExecVolatile,
}

fn capture_containment_identity_with_mode(
    pid: u32,
    mode: ContainmentIdentityValidation,
) -> SysprimsResult<ContainmentIdentity> {
    if pid == 0 || pid > sysprims_signal::MAX_SAFE_PID {
        return Err(SysprimsError::invalid_argument(format!(
            "pid {pid} is outside the safe process identity range"
        )));
    }

    let process = sysprims_proc::get_process(pid)?;
    let start_time_unix_ms = process.start_time_unix_ms.ok_or_else(|| {
        SysprimsError::system(
            "process start time unavailable during containment acquisition",
            0,
        )
    })?;
    let exe_path = match (mode, process.exe_path.filter(|path| !path.is_empty())) {
        (_, Some(path)) => path,
        (ContainmentIdentityValidation::Strict, None) => {
            return Err(SysprimsError::system(
                "process executable unavailable during containment acquisition",
                0,
            ));
        }
        #[cfg(unix)]
        (ContainmentIdentityValidation::ReceiptBoundExecVolatile, None) => String::new(),
    };

    Ok(ContainmentIdentity {
        pid,
        start_time_unix_ms,
        exe_path,
    })
}

#[cfg(unix)]
fn verify_containment_identity(
    expected: &ContainmentIdentity,
    mode: ContainmentIdentityValidation,
) -> SysprimsResult<bool> {
    let process = match sysprims_proc::get_process(expected.pid) {
        Ok(process) => process,
        Err(SysprimsError::NotFound { .. } | SysprimsError::PermissionDenied { .. }) => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };

    verify_containment_identity_observation(
        expected,
        process.start_time_unix_ms,
        process.exe_path.as_deref(),
        mode,
    )
}

#[cfg(unix)]
fn verify_containment_identity_observation(
    expected: &ContainmentIdentity,
    actual_start_time_unix_ms: Option<u64>,
    actual_exe_path: Option<&str>,
    mode: ContainmentIdentityValidation,
) -> SysprimsResult<bool> {
    if actual_start_time_unix_ms.is_some_and(|actual| actual != expected.start_time_unix_ms) {
        return Err(SysprimsError::invalid_argument(
            "process identity changed; refusing containment operation",
        ));
    }

    match mode {
        ContainmentIdentityValidation::Strict => {
            if actual_exe_path.is_some_and(|actual| actual != expected.exe_path) {
                return Err(SysprimsError::invalid_argument(
                    "process identity changed; refusing containment operation",
                ));
            }
            Ok(actual_start_time_unix_ms.is_some() && actual_exe_path.is_some())
        }
        ContainmentIdentityValidation::ReceiptBoundExecVolatile => {
            Ok(actual_start_time_unix_ms.is_some())
        }
    }
}

// =============================================================================
// Terminate Tree (PID-based)
// =============================================================================

/// Configuration for PID-based terminate-tree.
///
/// This is intentionally small and conservative; higher-level spawn-in-group
/// APIs can provide stronger guarantees.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminateTreeConfig {
    /// Timeout before escalation.
    #[serde(default = "default_grace_timeout_ms")]
    pub grace_timeout_ms: u64,

    /// Timeout to wait after escalation signal.
    #[serde(default = "default_kill_timeout_ms")]
    pub kill_timeout_ms: u64,

    /// Signal to send first (default SIGTERM).
    #[serde(default = "default_grace_signal")]
    pub signal: i32,

    /// Signal to send on escalation (default SIGKILL).
    #[serde(default = "default_kill_signal")]
    pub kill_signal: i32,
}

fn default_grace_timeout_ms() -> u64 {
    10_000
}

fn default_kill_timeout_ms() -> u64 {
    2_000
}

fn default_grace_signal() -> i32 {
    SIGTERM
}

fn default_kill_signal() -> i32 {
    SIGKILL
}

impl Default for TerminateTreeConfig {
    fn default() -> Self {
        Self {
            grace_timeout_ms: default_grace_timeout_ms(),
            kill_timeout_ms: default_kill_timeout_ms(),
            signal: default_grace_signal(),
            kill_signal: default_kill_signal(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TerminateTreeResult {
    pub schema_id: &'static str,
    pub timestamp: String,
    pub platform: &'static str,
    pub pid: u32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgid: Option<u32>,

    pub signal_sent: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kill_signal: Option<i32>,
    pub escalated: bool,
    pub exited: bool,
    pub timed_out: bool,
    pub tree_kill_reliability: String,
    pub warnings: Vec<String>,
}

// =============================================================================
// Spawn In Group / Job
// =============================================================================

/// Configuration for [`spawn_in_group`].
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnInGroupConfig {
    /// argv[0] is the command, argv[1..] are args.
    pub argv: Vec<String>,

    /// Optional working directory.
    #[serde(default)]
    pub cwd: Option<String>,

    /// Environment variable overrides/additions.
    ///
    /// By default the child inherits the parent's environment.
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpawnInGroupResult {
    pub schema_id: &'static str,
    pub timestamp: String,
    pub platform: &'static str,
    pub pid: u32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgid: Option<u32>,

    pub tree_kill_reliability: String,
    pub warnings: Vec<String>,
}

/// Spawn a process in a new process group on Unix.
///
/// Use this when you would otherwise shell out to `setsid`/wrapper scripts to
/// make jobs kill-tree safe.
///
/// This PID-returning compatibility API cannot retain the owned child and
/// sealed receipt required for [`TreeKillReliability::Guaranteed`]. Unix
/// therefore reports `best_effort`; Windows fails closed because it cannot
/// retain an owned Job handle. Use [`spawn_contained`] instead.
///
/// # Examples
///
/// ```rust,no_run
/// use sysprims_timeout::{spawn_in_group, SpawnInGroupConfig};
///
/// // Replaces: setsid sleep 5
/// let result = spawn_in_group(SpawnInGroupConfig {
///     argv: vec!["sleep".into(), "5".into()],
///     cwd: None,
///     env: None,
/// })
/// .unwrap();
/// println!("spawned pid: {}", result.pid);
/// ```
pub fn spawn_in_group(config: SpawnInGroupConfig) -> SysprimsResult<SpawnInGroupResult> {
    if config.argv.is_empty() {
        return Err(SysprimsError::invalid_argument("argv must not be empty"));
    }

    #[cfg(unix)]
    return unix::spawn_in_group_impl(config);

    #[cfg(windows)]
    return windows::spawn_in_group_impl(config);
}

// Timestamp generation consolidated in sysprims_core::time::now_rfc3339()

/// Terminate a process by PID with escalation.
///
/// This compatibility API has no owned child or sealed containment receipt, so
/// it signals only the supplied PID and always reports `best_effort`. Use an
/// owned [`ContainmentGuard`] for process-group or Job Object termination.
///
/// # Examples
///
/// ```rust,no_run
/// use sysprims_timeout::{terminate_tree, TerminateTreeConfig};
///
/// // Replaces: kill -TERM 1234; sleep 2; kill -KILL 1234
/// let result = terminate_tree(1234, TerminateTreeConfig::default()).unwrap();
/// println!("exited={} timed_out={}", result.exited, result.timed_out);
/// ```
pub fn terminate_tree(
    pid: u32,
    config: TerminateTreeConfig,
) -> SysprimsResult<TerminateTreeResult> {
    if pid == 0 {
        return Err(SysprimsError::invalid_argument("pid must be > 0"));
    }

    // Defense in depth: avoid unsafe casts on Unix.
    // See ADR-0011 (PID Validation Safety).
    if pid > sysprims_signal::MAX_SAFE_PID {
        return Err(SysprimsError::invalid_argument(format!(
            "pid {} exceeds maximum safe value {}",
            pid,
            sysprims_signal::MAX_SAFE_PID
        )));
    }

    let mut warnings: Vec<String> = Vec::new();
    let pgid: Option<u32> = None;
    let reliability = TreeKillReliability::BestEffort;

    #[cfg(windows)]
    warnings.push(
        "Windows PID termination is best-effort; use an owned containment guard for Job Objects"
            .to_string(),
    );
    #[cfg(unix)]
    warnings.push(
        "PID-only termination has no sealed group capability; signaling the target PID only"
            .to_string(),
    );

    // Step 1: send graceful signal
    sysprims_signal::kill(pid, config.signal)?;

    // Step 2: wait for exit
    let grace = Duration::from_millis(config.grace_timeout_ms);
    let grace_wait = wait_pid(pid, grace)?;
    if grace_wait.exited {
        return Ok(TerminateTreeResult {
            schema_id: TERMINATE_TREE_RESULT_V1,
            timestamp: now_rfc3339(),
            platform: get_platform(),
            pid,
            pgid,
            signal_sent: config.signal,
            kill_signal: None,
            escalated: false,
            exited: true,
            timed_out: false,
            tree_kill_reliability: reliability.as_str().to_string(),
            warnings,
        });
    }

    // Step 3: escalate
    sysprims_signal::kill(pid, config.kill_signal)?;

    let kill_wait = wait_pid(pid, Duration::from_millis(config.kill_timeout_ms))?;
    let mut exited = kill_wait.exited;
    let mut timed_out = kill_wait.timed_out;

    // If we timed out, attempt one final best-effort confirmation.
    // On some platforms/permission contexts, a process may become unobservable
    // (or a zombie) even after it has exited.
    if timed_out {
        match sysprims_proc::get_process(pid) {
            Ok(_) => {
                // Still observable -> treat as still running.
            }
            Err(SysprimsError::NotFound { .. }) => {
                exited = true;
                timed_out = false;
                warnings.push("PID no longer found after timeout; treating as exited".to_string());
            }
            Err(SysprimsError::PermissionDenied { .. }) => {
                warnings.push("Permission denied while confirming exit after timeout".to_string());
            }
            Err(e) => {
                warnings.push(format!("Failed to confirm exit after timeout: {}", e));
            }
        }
    }

    Ok(TerminateTreeResult {
        schema_id: TERMINATE_TREE_RESULT_V1,
        timestamp: now_rfc3339(),
        platform: get_platform(),
        pid,
        pgid,
        signal_sent: config.signal,
        kill_signal: Some(config.kill_signal),
        escalated: true,
        exited,
        timed_out,
        tree_kill_reliability: reliability.as_str().to_string(),
        warnings,
    })
}

/// Outcome of timeout execution.
#[derive(Debug)]
pub enum TimeoutOutcome {
    /// Command completed within timeout.
    Completed {
        /// Exit status of the child process.
        exit_status: ExitStatus,
    },

    /// Command timed out and was killed.
    TimedOut {
        /// Signal that was sent to terminate the process.
        signal_sent: i32,

        /// Whether escalation to SIGKILL occurred.
        ///
        /// `true` if the process didn't exit after receiving `signal_sent`
        /// and had to be forcefully killed with SIGKILL.
        escalated: bool,

        /// Whether tree-kill was reliable.
        ///
        /// `Guaranteed` when spawn-time acquisition and group/job signaling
        /// eligibility were proven.
        /// `BestEffort` if only the direct child was killed.
        tree_kill_reliability: TreeKillReliability,
    },
}

/// Run a command with timeout.
///
/// Spawns the command and waits for it to complete or timeout. If the command
/// times out, the acquired process group or Job is signaled (when using
/// `GroupByDefault`). Cooperative descendants in that containment are targeted;
/// Unix descendants can still leave a process group later.
///
/// # Arguments
///
/// * `command` - Command to execute (path or name in PATH)
/// * `args` - Command arguments
/// * `timeout` - Maximum duration to wait for command completion
/// * `config` - Timeout configuration (signal, escalation, grouping)
///
/// # Returns
///
/// * `Ok(TimeoutOutcome::Completed { .. })` - Command finished within timeout
/// * `Ok(TimeoutOutcome::TimedOut { .. })` - Command was killed due to timeout
/// * `Err(SysprimsError)` - Failed to spawn or fatal error
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use sysprims_timeout::{run_with_timeout, TimeoutConfig};
///
/// // Replaces: timeout 300s make build
/// let result = run_with_timeout(
///     "make",
///     &["build"],
///     Duration::from_secs(300),
///     TimeoutConfig::default(),
/// );
/// ```
pub fn run_with_timeout(
    command: &str,
    args: &[&str],
    timeout: Duration,
    config: TimeoutConfig,
) -> SysprimsResult<TimeoutOutcome> {
    #[cfg(unix)]
    return unix::run_with_timeout_impl(command, args, timeout, &config);

    #[cfg(windows)]
    return windows::run_with_timeout_impl(command, args, timeout, &config);
}

/// Run a command with timeout using default configuration.
///
/// Equivalent to `run_with_timeout(command, args, timeout, TimeoutConfig::default())`.
///
/// Default configuration:
/// - Signal: SIGTERM
/// - Kill after: 10 seconds
/// - Grouping: GroupByDefault
/// - Preserve status: false
///
/// # Examples
///
/// ```rust,no_run
/// use std::time::Duration;
/// use sysprims_timeout::run_with_timeout_default;
///
/// // Replaces: timeout 2s sleep 1
/// let _ = run_with_timeout_default("sleep", &["1"], Duration::from_secs(2));
/// ```
pub fn run_with_timeout_default(
    command: &str,
    args: &[&str],
    timeout: Duration,
) -> SysprimsResult<TimeoutOutcome> {
    run_with_timeout(command, args, timeout, TimeoutConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    struct ChildWithoutPid;

    impl ContainmentChild for ChildWithoutPid {
        fn process_id(&self) -> Option<u32> {
            None
        }

        fn try_wait(&mut self) -> std::io::Result<bool> {
            Ok(false)
        }

        #[cfg(windows)]
        fn raw_process_handle(&self) -> Option<std::os::windows::io::RawHandle> {
            None
        }
    }

    #[test]
    fn default_config_uses_sigterm() {
        let config = TimeoutConfig::default();
        assert_eq!(config.signal, SIGTERM);
    }

    #[test]
    fn default_config_uses_group_by_default() {
        let config = TimeoutConfig::default();
        assert_eq!(config.grouping, GroupingMode::GroupByDefault);
    }

    #[test]
    fn default_config_kill_after_is_10_seconds() {
        let config = TimeoutConfig::default();
        assert_eq!(config.kill_after, Duration::from_secs(10));
    }

    #[test]
    fn default_config_does_not_preserve_status() {
        let config = TimeoutConfig::default();
        assert!(!config.preserve_status);
    }

    #[test]
    fn reliability_strings_include_unproven() {
        assert_eq!(TreeKillReliability::Guaranteed.as_str(), "guaranteed");
        assert_eq!(TreeKillReliability::Unproven.as_str(), "unproven");
        assert_eq!(TreeKillReliability::BestEffort.as_str(), "best_effort");
    }

    #[test]
    fn boundary_strength_strings_are_independent() {
        assert_eq!(
            ContainmentBoundaryStrength::KernelEnforcedJob.as_str(),
            "kernel_enforced_job"
        );
        assert_eq!(
            ContainmentBoundaryStrength::CooperativeGroup.as_str(),
            "cooperative_group"
        );
        assert_eq!(ContainmentBoundaryStrength::Unknown.as_str(), "unknown");
    }

    #[test]
    #[cfg(unix)]
    fn receipt_bound_identity_treats_executable_as_observational() {
        let expected = ContainmentIdentity {
            pid: 42,
            start_time_unix_ms: 1_000,
            exe_path: "/before-exec".to_string(),
        };

        assert!(verify_containment_identity_observation(
            &expected,
            Some(1_000),
            Some("/after-two-execs"),
            ContainmentIdentityValidation::ReceiptBoundExecVolatile,
        )
        .unwrap());
        assert!(verify_containment_identity_observation(
            &expected,
            Some(1_000),
            None,
            ContainmentIdentityValidation::ReceiptBoundExecVolatile,
        )
        .unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn receipt_bound_identity_requires_matching_observable_start_time() {
        let expected = ContainmentIdentity {
            pid: 42,
            start_time_unix_ms: 1_000,
            exe_path: "/before-exec".to_string(),
        };

        assert!(!verify_containment_identity_observation(
            &expected,
            None,
            Some("/after-exec"),
            ContainmentIdentityValidation::ReceiptBoundExecVolatile,
        )
        .unwrap());
        assert!(matches!(
            verify_containment_identity_observation(
                &expected,
                Some(2_000),
                Some("/after-exec"),
                ContainmentIdentityValidation::ReceiptBoundExecVolatile,
            ),
            Err(SysprimsError::InvalidArgument { .. })
        ));
    }

    #[test]
    #[cfg(unix)]
    fn adopted_identity_keeps_strict_executable_validation() {
        let expected = ContainmentIdentity {
            pid: 42,
            start_time_unix_ms: 1_000,
            exe_path: "/before-exec".to_string(),
        };

        assert!(matches!(
            verify_containment_identity_observation(
                &expected,
                Some(1_000),
                Some("/after-exec"),
                ContainmentIdentityValidation::Strict,
            ),
            Err(SysprimsError::InvalidArgument { .. })
        ));
        assert!(!verify_containment_identity_observation(
            &expected,
            Some(1_000),
            None,
            ContainmentIdentityValidation::Strict,
        )
        .unwrap());
    }

    #[test]
    fn completion_evidence_sorts_unique_safe_pids() {
        assert_eq!(
            completion_from_pids(
                ContainmentObservation::LinuxProcfsProcessGroup,
                vec![42, 7, 19],
            ),
            ContainmentCompletionEvidence::Survivors {
                observation: ContainmentObservation::LinuxProcfsProcessGroup,
                observed_count: 3,
                survivor_pids: vec![7, 19, 42],
            }
        );
    }

    #[test]
    fn duplicate_completion_evidence_is_unknown() {
        assert_eq!(
            completion_from_pids(ContainmentObservation::LinuxProcfsProcessGroup, vec![7, 7],),
            ContainmentCompletionEvidence::Unknown {
                observation: ContainmentObservation::LinuxProcfsProcessGroup,
            }
        );
    }

    #[test]
    fn completion_evidence_at_pid_limit_is_unknown() {
        let pids = (2..2 + MAX_COMPLETION_OBSERVED_PIDS as u32).collect();
        assert_eq!(
            completion_from_pids(ContainmentObservation::LinuxProcfsProcessGroup, pids),
            ContainmentCompletionEvidence::Unknown {
                observation: ContainmentObservation::LinuxProcfsProcessGroup,
            }
        );
    }

    #[test]
    fn completion_evidence_serializes_as_a_tagged_receipt() {
        let value = serde_json::to_value(ContainmentCompletionEvidence::Survivors {
            observation: ContainmentObservation::MacosLibprocProcessGroup,
            observed_count: 2,
            survivor_pids: vec![17, 23],
        })
        .expect("completion evidence should serialize");
        assert_eq!(
            value,
            serde_json::json!({
                "status": "survivors",
                "observation": "macos_libproc_process_group",
                "observed_count": 2,
                "survivor_pids": [17, 23],
            })
        );
    }

    #[test]
    fn failed_adoption_returns_child_ownership() {
        let error = match adopt_contained(ChildWithoutPid) {
            Ok(_) => panic!("adoption without a pid must fail"),
            Err(error) => error,
        };
        assert!(matches!(error.error, SysprimsError::InvalidArgument { .. }));
        assert!(error.child.process_id().is_none());
    }

    #[test]
    fn terminate_tree_rejects_pid_zero() {
        let err = terminate_tree(0, TerminateTreeConfig::default()).unwrap_err();
        assert!(matches!(err, SysprimsError::InvalidArgument { .. }));
    }

    #[test]
    #[cfg(unix)]
    fn terminate_tree_kills_spawned_child() {
        // SAFETY: We spawn this process ourselves and control its PID.
        let mut child = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to spawn sleep process");

        let pid = child.id();
        let result = terminate_tree(
            pid,
            TerminateTreeConfig {
                grace_timeout_ms: 100,
                kill_timeout_ms: 5000,
                ..TerminateTreeConfig::default()
            },
        )
        .expect("terminate_tree should succeed");

        assert_eq!(result.pid, pid);
        assert!(
            result.exited,
            "expected child to be exited, got: {result:?}"
        );
        assert!(!result.timed_out, "unexpected timeout: {result:?}");
        assert_eq!(result.tree_kill_reliability, "best_effort");
        assert_eq!(result.pgid, None);

        let _ = child.wait();
    }

    #[test]
    #[cfg(windows)]
    fn terminate_tree_kills_spawned_child() {
        // SAFETY: We spawn this process ourselves and control its PID.
        let mut child = Command::new("cmd")
            .args(["/C", "ping -n 60 127.0.0.1 >NUL"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to spawn ping sleep process");

        let pid = child.id();
        let result = terminate_tree(
            pid,
            TerminateTreeConfig {
                grace_timeout_ms: 100,
                kill_timeout_ms: 5000,
                ..TerminateTreeConfig::default()
            },
        )
        .expect("terminate_tree should succeed");

        assert_eq!(result.pid, pid);
        let _ = child.wait();
    }
}
