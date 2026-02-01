use std::time::Duration;

use clap::{Parser, Subcommand};
use sysprims_core::SysprimsError;
use sysprims_core::{
    get_platform,
    schema::{BATCH_KILL_RESULT_V1, PROCESS_INFO_SAMPLED_V1},
};
use sysprims_proc::{cpu_total_time_ns, get_process, snapshot, snapshot_filtered, ProcessFilter};
use sysprims_signal::match_signal_names;
use sysprims_timeout::{run_with_timeout, GroupingMode, TimeoutConfig, TimeoutOutcome};
use tracing::info;
use tracing_subscriber::{filter::EnvFilter, fmt, prelude::*};

/// A cross-platform process utility toolkit.
#[derive(Parser, Debug)]
#[command(name = "sysprims", version, about, long_about = None)]
struct Cli {
    /// The format for log output.
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    log_format: LogFormat,

    /// The minimum log level to display.
    #[arg(long, value_name = "LEVEL", default_value = "info")]
    log_level: tracing::Level,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Send a signal to a process.
    Kill(KillArgs),

    /// Run a command with a timeout.
    ///
    /// If the command runs longer than the specified duration, it will be
    /// terminated. By default, the entire process tree is killed (not just
    /// the direct child).
    Timeout(TimeoutArgs),

    /// Display process information.
    ///
    /// Lists processes with optional filtering. Output is JSON by default
    /// for automation, or table format for human consumption.
    Pstat(PstatArgs),

    /// Terminate a process tree by PID.
    ///
    /// This is best-effort cross-platform termination of a PID and its descendants.
    /// On Unix this uses process groups when possible; on Windows it uses Job Objects.
    TerminateTree(TerminateTreeArgs),
}

#[derive(Parser, Debug)]
struct KillArgs {
    /// Target process ID(s) (or process group ID with --group).
    ///
    /// Not required when using --list.
    #[arg(value_name = "PID", required_unless_present = "list", num_args = 1..)]
    pids: Vec<u32>,

    /// Signal name, pattern, or number (default: TERM).
    #[arg(
        short = 's',
        long = "signal",
        value_name = "SIGNAL",
        default_value = "TERM"
    )]
    signal: String,

    /// List signal names, or get number for a signal name.
    ///
    /// Without argument: list all signals in table format.
    /// With argument: print the signal number for the given name.
    #[arg(short = 'l', long = "list", value_name = "SIGNAL", num_args = 0..=1)]
    list: Option<Option<String>>,

    /// Send signal to process group instead of single process.
    ///
    /// On Unix, uses killpg() to signal all processes in the group.
    /// On Windows, returns an error (process groups not supported).
    #[arg(short = 'g', long = "group", conflicts_with = "list")]
    group: bool,

    /// Output JSON batch result.
    #[arg(long, conflicts_with = "list")]
    json: bool,
}

#[derive(Parser, Debug)]
struct TimeoutArgs {
    /// Timeout duration (e.g., "5s", "1m", "500ms").
    ///
    /// Supports: ms (milliseconds), s (seconds), m (minutes), h (hours).
    /// Plain numbers are treated as seconds.
    #[arg(value_name = "DURATION")]
    duration: String,

    /// Command to execute.
    #[arg(value_name = "COMMAND")]
    command: String,

    /// Arguments to pass to the command.
    #[arg(value_name = "ARGS", trailing_var_arg = true)]
    args: Vec<String>,

    /// Signal to send on timeout (default: TERM).
    #[arg(
        short = 's',
        long = "signal",
        value_name = "SIGNAL",
        default_value = "TERM"
    )]
    signal: String,

    /// Send SIGKILL if command still running after this duration.
    ///
    /// Supports same format as main duration. Default: 10s.
    #[arg(short = 'k', long = "kill-after", value_name = "DURATION")]
    kill_after: Option<String>,

    /// Run command in foreground (don't create process group).
    ///
    /// Only the direct child will be killed on timeout, not its descendants.
    /// Use this when the command needs terminal access.
    #[arg(long)]
    foreground: bool,

    /// Exit with the same status as the command, even on timeout.
    ///
    /// When a timeout occurs, returns 128+signal (SIGKILL if escalation occurs).
    #[arg(long)]
    preserve_status: bool,
}

#[derive(Parser, Debug)]
struct PstatArgs {
    /// Output as JSON (default for automation).
    #[arg(long)]
    json: bool,

    /// Output as human-readable table.
    #[arg(long, conflicts_with = "json")]
    table: bool,

    /// Show only a specific process by PID.
    #[arg(long, value_name = "PID")]
    pid: Option<u32>,

    /// Filter by process name (substring match, case-insensitive).
    #[arg(long, value_name = "NAME")]
    name: Option<String>,

    /// Filter by username.
    #[arg(long, value_name = "USER")]
    user: Option<String>,

    /// Filter by minimum CPU usage.
    ///
    /// Notes:
    /// - In `lifetime` mode, values are best-effort and generally fall in 0-100.
    /// - In `monitor`/sampling modes, values may exceed 100 when a process uses multiple cores.
    #[arg(long, value_name = "PERCENT")]
    cpu_above: Option<f64>,

    /// CPU measurement mode.
    ///
    /// - lifetime: lifetime-average estimate (may under-report recent spikes)
    /// - monitor: sampled CPU over a short interval (Activity Monitor / top style)
    #[arg(long, value_enum, value_name = "MODE", default_value = "lifetime")]
    cpu_mode: CpuMode,

    /// Sample CPU usage over an interval (e.g., "250ms").
    ///
    /// When provided, `cpu_percent` is computed as a rate over this interval
    /// instead of a lifetime-average estimate.
    #[arg(long, value_name = "DURATION")]
    sample: Option<String>,

    /// Limit output to the top N processes (after filtering).
    #[arg(long, value_name = "N")]
    top: Option<usize>,

    /// Filter by minimum memory usage in KB.
    #[arg(long, value_name = "KB")]
    memory_above: Option<u64>,

    /// Sort by field (pid, name, cpu, memory).
    #[arg(long, value_name = "FIELD", default_value = "pid")]
    sort: String,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
enum CpuMode {
    /// Lifetime-average CPU usage (best-effort), normalized 0-100.
    Lifetime,
    /// Sampled CPU usage over an interval (Activity Monitor / top style).
    Monitor,
}

#[derive(Parser, Debug)]
struct TerminateTreeArgs {
    /// Target process ID.
    #[arg(value_name = "PID")]
    pid: u32,

    /// Grace period before escalation (default: 5s).
    #[arg(long, value_name = "DURATION", default_value = "5s")]
    grace: String,

    /// Send kill_signal if still running after this duration (default: 10s).
    #[arg(long, value_name = "DURATION", default_value = "10s")]
    kill_after: String,

    /// Signal used for the grace period (default: TERM).
    #[arg(long, value_name = "SIGNAL", default_value = "TERM")]
    signal: String,

    /// Signal used for forced termination (default: KILL).
    #[arg(long, value_name = "SIGNAL", default_value = "KILL")]
    kill_signal: String,

    /// Refuse to terminate if the PID's start time does not match.
    #[arg(long, value_name = "UNIX_MS")]
    require_start_time_ms: Option<u64>,

    /// Refuse to terminate if the PID's executable path does not match.
    #[arg(long, value_name = "PATH")]
    require_exe_path: Option<String>,

    /// Proceed even if identity checks fail.
    #[arg(long)]
    force: bool,

    /// Output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
enum LogFormat {
    /// Human-readable text format.
    Text,
    /// Machine-readable JSON format.
    Json,
}

fn main() {
    let cli = Cli::parse();

    // Initialize the tracing subscriber
    let filter = EnvFilter::from_default_env().add_directive(cli.log_level.into());

    match cli.log_format {
        LogFormat::Text => {
            tracing_subscriber::registry()
                .with(fmt::layer().with_writer(std::io::stderr))
                .with(filter)
                .init();
        }
        LogFormat::Json => {
            tracing_subscriber::registry()
                .with(fmt::layer().json().with_writer(std::io::stderr))
                .with(filter)
                .init();
        }
    }

    info!("Initialization complete. Starting main logic.");
    if let Some(command) = cli.command {
        match run_command(command) {
            Ok(exit_code) => {
                info!("Main logic finished.");
                std::process::exit(exit_code);
            }
            Err(err) => {
                eprintln!("Error: {err}");
                std::process::exit(1);
            }
        }
    } else {
        println!("Platform: {}", get_platform());
    }
    info!("Main logic finished.");
}

fn run_command(command: Command) -> Result<i32, SysprimsError> {
    match command {
        Command::Kill(args) => run_kill(args),
        Command::Timeout(args) => run_timeout(args),
        Command::Pstat(args) => run_pstat(args),
        Command::TerminateTree(args) => {
            run_terminate_tree(args)?;
            Ok(0)
        }
    }
}

enum SignalTarget {
    Number(i32),
    Name(String),
}

fn parse_signal_arg(signal_arg: &str) -> Result<SignalTarget, SysprimsError> {
    let trimmed = signal_arg.trim();
    if trimmed.is_empty() {
        return Err(SysprimsError::invalid_argument("signal cannot be empty"));
    }

    if trimmed.contains('*') || trimmed.contains('?') {
        let matches = match_signal_names(trimmed);
        return match matches.len() {
            0 => Err(SysprimsError::invalid_argument(format!(
                "signal pattern '{trimmed}' matched no signals"
            ))),
            1 => Ok(SignalTarget::Name(matches[0].to_string())),
            _ => Err(SysprimsError::invalid_argument(format!(
                "signal pattern '{trimmed}' matched multiple signals: {}",
                matches.join(", ")
            ))),
        };
    }

    if let Ok(number) = trimmed.parse::<i32>() {
        return Ok(SignalTarget::Number(number));
    }

    Ok(SignalTarget::Name(trimmed.to_string()))
}

#[derive(serde::Serialize)]
struct BatchKillFailureJson {
    pid: u32,
    error: String,
}

#[derive(serde::Serialize)]
struct BatchKillResultJson {
    schema_id: &'static str,
    signal_sent: i32,
    succeeded: Vec<u32>,
    failed: Vec<BatchKillFailureJson>,
}

fn run_kill(args: KillArgs) -> Result<i32, SysprimsError> {
    // Handle --list flag
    if let Some(list_arg) = args.list {
        return run_kill_list(list_arg);
    }

    if args.group && args.pids.len() != 1 {
        return Err(SysprimsError::invalid_argument(
            "--group requires exactly one PID",
        ));
    }

    // Parse signal
    let signal = parse_signal_arg(&args.signal)?;
    let signal_num = match signal {
        SignalTarget::Number(n) => n,
        SignalTarget::Name(ref name) => sysprims_signal::get_signal_number(name)
            .or_else(|| sysprims_signal::get_signal_number(&name.to_ascii_uppercase()))
            .or_else(|| {
                sysprims_signal::get_signal_number(&format!("SIG{}", name.to_ascii_uppercase()))
            })
            .ok_or_else(|| SysprimsError::invalid_argument(format!("unknown signal '{}'", name)))?,
    };

    let schema_id = BATCH_KILL_RESULT_V1;

    // Send signal to process or process group
    if args.group {
        let pgid = args.pids[0];
        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        match sysprims_signal::killpg(pgid, signal_num) {
            Ok(()) => succeeded.push(pgid),
            Err(e) => failed.push(BatchKillFailureJson {
                pid: pgid,
                error: e.to_string(),
            }),
        }

        if args.json {
            let out = BatchKillResultJson {
                schema_id,
                signal_sent: signal_num,
                succeeded,
                failed,
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&out).expect("serialize json")
            );
            return Ok(if out.failed.is_empty() { 0 } else { 1 });
        }

        if !failed.is_empty() {
            for f in failed {
                eprintln!("PID {}: {}", f.pid, f.error);
            }
            return Ok(1);
        }

        return Ok(0);
    }

    // Non-group: multi-PID supported.
    let batch = sysprims_signal::kill_many(&args.pids, signal_num)?;
    let failed: Vec<BatchKillFailureJson> = batch
        .failed
        .into_iter()
        .map(|f| BatchKillFailureJson {
            pid: f.pid,
            error: f.error.to_string(),
        })
        .collect();

    if args.json {
        let out = BatchKillResultJson {
            schema_id,
            signal_sent: signal_num,
            succeeded: batch.succeeded,
            failed,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&out).expect("serialize json")
        );
        return Ok(if out.failed.is_empty() { 0 } else { 1 });
    }

    if !failed.is_empty() {
        for f in failed {
            eprintln!("PID {}: {}", f.pid, f.error);
        }
        return Ok(1);
    }

    Ok(0)
}

/// Handle `kill --list` command.
fn run_kill_list(signal_name: Option<String>) -> Result<i32, SysprimsError> {
    if let Some(name) = signal_name {
        // Print signal number for a specific signal name
        let num = sysprims_signal::get_signal_number(&name)
            .or_else(|| sysprims_signal::get_signal_number(&name.to_ascii_uppercase()))
            .or_else(|| {
                sysprims_signal::get_signal_number(&format!("SIG{}", name.to_ascii_uppercase()))
            })
            .ok_or_else(|| SysprimsError::invalid_argument(format!("unknown signal '{}'", name)))?;
        println!("{}", num);
    } else {
        // Print all signals in table format
        print_signal_table();
    }
    Ok(0)
}

/// Print all signals in table format (similar to `kill -l`).
fn print_signal_table() {
    use sysprims_signal::list_signals;

    let signals = list_signals();
    let mut col = 0;

    for signal in signals {
        if let Some(num) = sysprims_signal::get_signal_number(&signal.name) {
            print!("{:>2}) {:<10}", num, &signal.name);
            col += 1;
            if col % 4 == 0 {
                println!();
            }
        }
    }

    // Print final newline if we didn't end on a full row
    if col % 4 != 0 {
        println!();
    }
}

// ============================================================================
// Timeout command
// ============================================================================

/// Exit codes per GNU timeout convention:
/// - 124: Command timed out
/// - 125: Timeout command itself failed
/// - 126: Command found but cannot be invoked
/// - 127: Command not found
/// - 137: Command killed by SIGKILL (128 + 9)
mod exit_codes {
    pub const TIMEOUT: i32 = 124;
    pub const INTERNAL_ERROR: i32 = 125;
    pub const CANNOT_INVOKE: i32 = 126;
    pub const NOT_FOUND: i32 = 127;
}

fn run_timeout(args: TimeoutArgs) -> Result<i32, SysprimsError> {
    // Parse duration
    let timeout = parse_duration(&args.duration)?;

    // Parse kill_after duration
    let kill_after = match &args.kill_after {
        Some(d) => parse_duration(d)?,
        None => Duration::from_secs(10),
    };

    // Parse signal
    let signal = resolve_signal(&args.signal)?;

    // Build config
    let config = TimeoutConfig {
        signal,
        kill_after,
        grouping: if args.foreground {
            GroupingMode::Foreground
        } else {
            GroupingMode::GroupByDefault
        },
        preserve_status: args.preserve_status,
    };

    // Convert args to &str slice
    let arg_refs: Vec<&str> = args.args.iter().map(|s| s.as_str()).collect();

    // Run with timeout
    info!(
        command = %args.command,
        timeout_ms = timeout.as_millis() as u64,
        signal = signal,
        "Running command with timeout"
    );

    match run_with_timeout(&args.command, &arg_refs, timeout, config) {
        Ok(TimeoutOutcome::Completed { exit_status }) => {
            // Command completed within timeout
            if args.preserve_status {
                Ok(exit_status.code().unwrap_or(0))
            } else {
                Ok(0)
            }
        }
        Ok(TimeoutOutcome::TimedOut {
            signal_sent,
            escalated,
            tree_kill_reliability,
        }) => {
            info!(
                signal_sent = signal_sent,
                escalated = escalated,
                reliability = ?tree_kill_reliability,
                "Command timed out"
            );

            if args.preserve_status {
                let exit_signal = if escalated {
                    sysprims_signal::SIGKILL
                } else {
                    signal_sent
                };
                Ok(128 + exit_signal)
            } else {
                Ok(exit_codes::TIMEOUT)
            }
        }
        Err(SysprimsError::NotFoundCommand { .. }) => Ok(exit_codes::NOT_FOUND),
        Err(SysprimsError::PermissionDeniedCommand { .. }) => Ok(exit_codes::CANNOT_INVOKE),
        Err(e) => {
            eprintln!("timeout: {}", e);
            Ok(exit_codes::INTERNAL_ERROR)
        }
    }
}

// ============================================================================
// TerminateTree command
// ============================================================================

fn run_terminate_tree(args: TerminateTreeArgs) -> Result<(), SysprimsError> {
    // Hard safety checks for interactive CLI usage.
    // The library allows terminating any pid > 0 (subject to OS permission), but the CLI
    // defaults to refusing the most footgun targets unless the user explicitly opts in.
    let self_pid = std::process::id();
    if args.pid == self_pid && !args.force {
        return Err(SysprimsError::invalid_argument(
            "refusing to terminate the sysprims process itself (use --force to override)",
        ));
    }

    if args.pid == 1 && !args.force {
        return Err(SysprimsError::invalid_argument(
            "refusing to terminate PID 1 (use --force to override)",
        ));
    }

    // Refuse killing our parent by default; this often means killing the caller's shell/terminal.
    if let Ok(self_info) = get_process(self_pid) {
        if args.pid == self_info.ppid && !args.force {
            return Err(SysprimsError::invalid_argument(
                "refusing to terminate the parent process (use --force to override)",
            ));
        }
    }

    // Optional PID identity checks to prevent PID reuse mistakes.
    if args.require_start_time_ms.is_some() || args.require_exe_path.is_some() {
        let info = get_process(args.pid)?;

        if let Some(expected) = args.require_start_time_ms {
            let actual = info.start_time_unix_ms.ok_or_else(|| {
                SysprimsError::invalid_argument(
                    "start_time_unix_ms unavailable; cannot enforce --require-start-time-ms",
                )
            })?;
            if actual != expected && !args.force {
                return Err(SysprimsError::invalid_argument(format!(
                    "PID identity mismatch: start_time_unix_ms expected {expected}, got {actual} (use --force to override)"
                )));
            }
        }

        if let Some(ref expected) = args.require_exe_path {
            let actual = info.exe_path.clone().ok_or_else(|| {
                SysprimsError::invalid_argument(
                    "exe_path unavailable; cannot enforce --require-exe-path",
                )
            })?;
            if &actual != expected && !args.force {
                return Err(SysprimsError::invalid_argument(format!(
                    "PID identity mismatch: exe_path expected '{expected}', got '{actual}' (use --force to override)"
                )));
            }
        }
    }

    let grace = parse_duration(&args.grace)?;
    let kill_after = parse_duration(&args.kill_after)?;
    let signal = resolve_signal(&args.signal)?;
    let kill_signal = resolve_signal(&args.kill_signal)?;

    // Mirror ADR-0011 bounds (avoid dangerous casts).
    if args.pid > sysprims_signal::MAX_SAFE_PID {
        return Err(SysprimsError::invalid_argument(format!(
            "pid {} exceeds maximum safe value {}",
            args.pid,
            sysprims_signal::MAX_SAFE_PID
        )));
    }

    let cfg = sysprims_timeout::TerminateTreeConfig {
        grace_timeout_ms: grace.as_millis() as u64,
        kill_timeout_ms: kill_after.as_millis() as u64,
        signal,
        kill_signal,
    };

    let result = sysprims_timeout::terminate_tree(args.pid, cfg)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        // Human summary
        println!(
            "terminate-tree: pid={} tree_kill_reliability={} warnings={}",
            result.pid,
            result.tree_kill_reliability,
            result.warnings.len()
        );
        for w in result.warnings {
            println!("warning: {w}");
        }
    }

    Ok(())
}

/// Parse a duration string like "5s", "100ms", "2m", "1h", or just "5" (seconds).
fn parse_duration(s: &str) -> Result<Duration, SysprimsError> {
    let s = s.trim();

    // Try to parse as plain number (seconds)
    if let Ok(secs) = s.parse::<f64>() {
        if secs < 0.0 {
            return Err(SysprimsError::invalid_argument(
                "duration cannot be negative",
            ));
        }
        return Ok(Duration::from_secs_f64(secs));
    }

    // Try to parse with suffix
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60.0)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600.0)
    } else {
        return Err(SysprimsError::invalid_argument(format!(
            "invalid duration '{}': expected number or number with suffix (ms, s, m, h)",
            s
        )));
    };

    let num: f64 = num_str.trim().parse().map_err(|_| {
        SysprimsError::invalid_argument(format!("invalid duration '{}': not a valid number", s))
    })?;

    if num < 0.0 {
        return Err(SysprimsError::invalid_argument(
            "duration cannot be negative",
        ));
    }

    Ok(Duration::from_secs_f64(num * multiplier))
}

/// Resolve signal name or number to signal number.
fn resolve_signal(s: &str) -> Result<i32, SysprimsError> {
    let trimmed = s.trim();

    // Try as number first
    if let Ok(num) = trimmed.parse::<i32>() {
        return Ok(num);
    }

    // Try as signal name (supports "TERM", "SIGTERM", "term", etc.)
    sysprims_signal::get_signal_number(trimmed)
        .or_else(|| sysprims_signal::get_signal_number(&trimmed.to_ascii_uppercase()))
        .or_else(|| {
            sysprims_signal::get_signal_number(&format!("SIG{}", trimmed.to_ascii_uppercase()))
        })
        .ok_or_else(|| SysprimsError::invalid_argument(format!("unknown signal '{}'", trimmed)))
}

// ============================================================================
// Pstat command
// ============================================================================

fn run_pstat(args: PstatArgs) -> Result<i32, SysprimsError> {
    let monitor_mode = args.cpu_mode == CpuMode::Monitor;
    let sampling = args.sample.is_some() || monitor_mode;
    let sample_duration = if sampling {
        if let Some(sample_s) = &args.sample {
            parse_duration(sample_s)?
        } else {
            std::time::Duration::from_secs(1)
        }
    } else {
        std::time::Duration::from_secs(0)
    };

    // If specific PID requested, just get that one
    if let Some(pid) = args.pid {
        let mut proc_info = get_process(pid)?;

        if sampling {
            let sample = sample_duration;
            let cpu0 = cpu_total_time_ns(pid)?;
            std::thread::sleep(sample);
            let cpu1 = cpu_total_time_ns(pid)?;
            let dt_ns = sample.as_nanos() as f64;
            let delta = cpu1.saturating_sub(cpu0) as f64;
            if dt_ns > 0.0 {
                proc_info.cpu_percent = (delta / dt_ns) * 100.0;
            }
        }

        if args.table {
            print_process_table(&[proc_info]);
        } else {
            // Default to JSON
            println!("{}", serde_json::to_string_pretty(&proc_info).unwrap());
        }
        return Ok(0);
    }

    // Build filter from args.
    // When sampling CPU, apply cpu_above after sampling so we don't filter out
    // processes due to lifetime-average CPU values.
    let base_filter = ProcessFilter {
        name_contains: args.name.clone(),
        user_equals: args.user.clone(),
        cpu_above: if sampling { None } else { args.cpu_above },
        memory_above_kb: args.memory_above,
        ..Default::default()
    };

    let mut snap = if base_filter.name_contains.is_some()
        || base_filter.user_equals.is_some()
        || base_filter.cpu_above.is_some()
        || base_filter.memory_above_kb.is_some()
    {
        snapshot_filtered(&base_filter)?
    } else {
        snapshot()?
    };

    if sampling {
        let sample = sample_duration;
        if sample.is_zero() {
            return Err(SysprimsError::invalid_argument(
                "sample duration must be > 0",
            ));
        }

        // Capture CPU totals at t0.
        let mut t0 = std::collections::HashMap::<u32, (Option<u64>, u64)>::new();
        for p in &snap.processes {
            if let Ok(cpu_ns) = cpu_total_time_ns(p.pid) {
                t0.insert(p.pid, (p.start_time_unix_ms, cpu_ns));
            }
        }

        std::thread::sleep(sample);

        // Refresh snapshot (same base filter) for current fields.
        let mut snap1 = if base_filter.name_contains.is_some()
            || base_filter.user_equals.is_some()
            || base_filter.cpu_above.is_some()
            || base_filter.memory_above_kb.is_some()
        {
            snapshot_filtered(&base_filter)?
        } else {
            snapshot()?
        };

        let dt_ns = sample.as_nanos() as f64;
        if dt_ns > 0.0 {
            for p in &mut snap1.processes {
                if let Ok(cpu1) = cpu_total_time_ns(p.pid) {
                    if let Some((start0, cpu0)) = t0.get(&p.pid) {
                        // PID reuse guard: only compute if start time matches.
                        if start0.is_some()
                            && p.start_time_unix_ms.is_some()
                            && start0 != &p.start_time_unix_ms
                        {
                            continue;
                        }
                        let delta = cpu1.saturating_sub(*cpu0) as f64;
                        p.cpu_percent = (delta / dt_ns) * 100.0;
                    }
                }
            }
        }

        // Apply cpu_above after sampling.
        if let Some(threshold) = args.cpu_above {
            snap1.processes.retain(|p| p.cpu_percent >= threshold);
        }

        // Sampling changes CPU semantics (can exceed 100 for multi-core).
        snap1.schema_id = PROCESS_INFO_SAMPLED_V1;

        snap = snap1;
    }

    // Sort processes
    if sampling && args.sort == "pid" {
        sort_processes(&mut snap.processes, "cpu");
    } else {
        sort_processes(&mut snap.processes, &args.sort);
    }

    // Top N
    if let Some(n) = args.top {
        if snap.processes.len() > n {
            snap.processes.truncate(n);
        }
    }

    // Output
    if args.table {
        print_process_table(&snap.processes);
    } else {
        // Default to JSON
        println!("{}", serde_json::to_string_pretty(&snap).unwrap());
    }

    Ok(0)
}

/// Sort processes by the specified field.
fn sort_processes(processes: &mut [sysprims_proc::ProcessInfo], field: &str) {
    match field.to_lowercase().as_str() {
        "pid" => processes.sort_by_key(|p| p.pid),
        "name" => processes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        "cpu" => processes.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "memory" | "mem" => processes.sort_by(|a, b| b.memory_kb.cmp(&a.memory_kb)),
        _ => processes.sort_by_key(|p| p.pid),
    }
}

/// Print processes in table format.
fn print_process_table(processes: &[sysprims_proc::ProcessInfo]) {
    // Header
    println!(
        "{:>7} {:>7} {:>6} {:>10} {:>8} {:<16} NAME",
        "PID", "PPID", "CPU%", "MEM(KB)", "STATE", "USER"
    );
    println!("{:-<80}", "");

    if processes.is_empty() {
        println!("(no matching processes)");
        return;
    }

    for p in processes {
        let user = p.user.as_deref().unwrap_or("-");
        let state = match p.state {
            sysprims_proc::ProcessState::Running => "R",
            sysprims_proc::ProcessState::Sleeping => "S",
            sysprims_proc::ProcessState::Stopped => "T",
            sysprims_proc::ProcessState::Zombie => "Z",
            sysprims_proc::ProcessState::Unknown => "?",
        };
        println!(
            "{:>7} {:>7} {:>6.1} {:>10} {:>8} {:<16} {}",
            p.pid,
            p.ppid,
            p.cpu_percent,
            p.memory_kb,
            state,
            truncate(user, 16),
            truncate(&p.name, 32)
        );
    }
}

/// Truncate string to max characters (not bytes).
///
/// Safe for UTF-8 strings with multi-byte characters.
fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s, // String has fewer than max_chars characters
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_requires_pid_unless_list_present() {
        assert!(Cli::try_parse_from(["sysprims", "kill"]).is_err());
    }

    #[test]
    fn kill_list_parses_without_pid() {
        let cli = Cli::try_parse_from(["sysprims", "kill", "-l"]).unwrap();
        let Command::Kill(args) = cli.command.unwrap() else {
            panic!("expected kill command");
        };
        assert!(args.pids.is_empty());
        assert!(matches!(args.list, Some(None)));
    }

    #[test]
    fn kill_list_parses_with_signal_name_arg() {
        let cli = Cli::try_parse_from(["sysprims", "kill", "-l", "TERM"]).unwrap();
        let Command::Kill(args) = cli.command.unwrap() else {
            panic!("expected kill command");
        };
        assert!(args.pids.is_empty());
        assert!(matches!(args.list, Some(Some(ref s)) if s == "TERM"));
    }

    #[test]
    fn kill_group_parses() {
        let cli = Cli::try_parse_from(["sysprims", "kill", "--group", "1234"]).unwrap();
        let Command::Kill(args) = cli.command.unwrap() else {
            panic!("expected kill command");
        };
        assert_eq!(args.pids, vec![1234]);
        assert!(args.group);
        assert_eq!(args.list, None);
    }
}
