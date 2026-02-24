use std::time::Duration;

use clap::{Parser, Subcommand};
use sysprims_core::SysprimsError;
use sysprims_core::{
    get_platform,
    schema::{BATCH_KILL_RESULT_V1, PROCESS_INFO_SAMPLED_V1},
};
use sysprims_proc::{
    cpu_total_time_ns, descendants_with_config, get_process, list_fds, listening_ports, snapshot,
    snapshot_filtered, CpuMode as ProcCpuMode, DescendantsConfig, FdFilter, FdKind, PortFilter,
    ProcessFilter, Protocol,
};
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

    /// List descendants of a process.
    ///
    /// Traverses the process tree from a root PID, showing direct children
    /// (level 1) by default. Use --max-levels to go deeper.
    Descendants(DescendantsArgs),

    /// Kill descendants of a process.
    ///
    /// Traverses the process tree from a root PID and sends signals to
    /// matching descendants. Defaults to preview mode unless --yes is provided.
    KillDescendants(KillDescendantsArgs),

    /// List open file descriptors for a process.
    Fds(FdsArgs),

    /// List listening port bindings.
    Ports(PortsArgs),
}

#[derive(Parser, Debug)]
struct KillArgs {
    /// Target process ID(s) (or process group ID with --group).
    ///
    /// Not required when using --list.
    #[arg(value_name = "PID", num_args = 0..)]
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

    /// Filter by parent PID.
    #[arg(long, value_name = "PID", conflicts_with = "list")]
    ppid: Option<u32>,

    /// Filter by process name (substring match, case-insensitive).
    #[arg(long, value_name = "NAME", conflicts_with = "list")]
    name: Option<String>,

    /// Filter by username.
    #[arg(long, value_name = "USER", conflicts_with = "list")]
    user: Option<String>,

    /// Filter by minimum CPU usage (0-100, lifetime estimate).
    ///
    /// Set `SYSPRIMS_NO_HINTS=1` to suppress contextual CPU hints.
    #[arg(long, value_name = "PERCENT", conflicts_with = "list")]
    cpu_above: Option<f64>,

    /// Filter by minimum memory usage in KB.
    #[arg(long, value_name = "KB", conflicts_with = "list")]
    memory_above: Option<u64>,

    /// Filter by minimum process age (e.g., "5s", "1m", "2h").
    #[arg(long, value_name = "DURATION", conflicts_with = "list")]
    running_for: Option<String>,

    /// Print matched targets but do not send signals.
    #[arg(long, conflicts_with = "list")]
    dry_run: bool,

    /// Proceed with kill when using filters.
    ///
    /// When selecting targets via filters (rather than explicit PIDs), sysprims defaults
    /// to a non-destructive preview unless --yes is provided.
    #[arg(long, conflicts_with = "list")]
    yes: bool,

    /// Proceed even if CLI safety checks would normally refuse.
    #[arg(long, conflicts_with = "list")]
    force: bool,
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

    /// Filter by parent PID.
    #[arg(long, value_name = "PID")]
    ppid: Option<u32>,

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

    /// Filter by minimum process age (e.g., "5s", "1m", "2h").
    #[arg(long, value_name = "DURATION")]
    running_for: Option<String>,

    /// Sort by field (pid, name, cpu, memory).
    #[arg(long, value_name = "FIELD", default_value = "pid")]
    sort: String,
}

#[derive(Parser, Debug)]
struct DescendantsArgs {
    /// Root process ID to traverse from.
    #[arg(value_name = "PID")]
    pid: u32,

    /// Maximum traversal depth (1 = children only, "all" = full subtree).
    #[arg(long, value_name = "N", default_value = "1")]
    max_levels: String,

    /// Output as JSON (default).
    #[arg(long, conflicts_with_all = ["table", "tree"])]
    json: bool,

    /// Output as human-readable table (flat, grouped by level).
    #[arg(long, conflicts_with_all = ["json", "tree"])]
    table: bool,

    /// Output as ASCII art tree with hierarchy visualization.
    #[arg(long, conflicts_with_all = ["json", "table"])]
    tree: bool,

    /// Filter by process name (substring match, case-insensitive).
    #[arg(long, value_name = "NAME")]
    name: Option<String>,

    /// Filter by username.
    #[arg(long, value_name = "USER")]
    user: Option<String>,

    /// Filter by minimum CPU usage (0-100).
    ///
    /// Set `SYSPRIMS_NO_HINTS=1` to suppress contextual CPU hints.
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
    /// When provided with `--cpu-mode monitor`, cpu_percent is computed as a
    /// rate over this interval instead of a lifetime-average estimate.
    #[arg(long, value_name = "DURATION")]
    sample: Option<String>,

    /// Filter by minimum memory usage in KB.
    #[arg(long, value_name = "KB")]
    memory_above: Option<u64>,

    /// Filter by minimum process age (e.g., "5s", "1m", "2h").
    #[arg(long, value_name = "DURATION")]
    running_for: Option<String>,
}

#[derive(Parser, Debug)]
struct KillDescendantsArgs {
    /// Root process ID to traverse from.
    #[arg(value_name = "PID")]
    pid: u32,

    /// Maximum traversal depth (1 = children only, "all" = full subtree).
    #[arg(long, value_name = "N", default_value = "1")]
    max_levels: String,

    /// Signal name or number (default: TERM).
    #[arg(
        short = 's',
        long = "signal",
        value_name = "SIGNAL",
        default_value = "TERM"
    )]
    signal: String,

    /// Filter by process name (substring match, case-insensitive).
    #[arg(long, value_name = "NAME")]
    name: Option<String>,

    /// Filter by username.
    #[arg(long, value_name = "USER")]
    user: Option<String>,

    /// Filter by minimum CPU usage (0-100).
    ///
    /// Set `SYSPRIMS_NO_HINTS=1` to suppress contextual CPU hints.
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
    /// When provided with `--cpu-mode monitor`, cpu_percent is computed as a
    /// rate over this interval instead of a lifetime-average estimate.
    #[arg(long, value_name = "DURATION")]
    sample: Option<String>,

    /// Filter by minimum memory usage in KB.
    #[arg(long, value_name = "KB")]
    memory_above: Option<u64>,

    /// Filter by minimum process age (e.g., "5s", "1m", "2h").
    #[arg(long, value_name = "DURATION")]
    running_for: Option<String>,

    /// Print matched targets but do not send signals.
    #[arg(long)]
    dry_run: bool,

    /// Proceed with kill (default is preview mode).
    #[arg(long)]
    yes: bool,

    /// Proceed even if CLI safety checks would normally refuse.
    #[arg(long)]
    force: bool,

    /// Output as JSON.
    #[arg(long)]
    json: bool,
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

#[derive(Parser, Debug)]
struct FdsArgs {
    /// Target process ID.
    #[arg(long, value_name = "PID")]
    pid: u32,

    /// Output as JSON.
    #[arg(long)]
    json: bool,

    /// Output as human-readable table.
    #[arg(long, conflicts_with = "json")]
    table: bool,

    /// Filter by fd kind.
    #[arg(long, value_enum, value_name = "KIND")]
    kind: Option<FdKindArg>,
}

#[derive(Parser, Debug)]
struct PortsArgs {
    /// Output as JSON.
    #[arg(long)]
    json: bool,

    /// Output as human-readable table.
    #[arg(long, conflicts_with = "json")]
    table: bool,

    /// Filter by protocol.
    #[arg(long, value_enum, value_name = "PROTO")]
    protocol: Option<ProtocolArg>,

    /// Filter by local port.
    #[arg(long, value_name = "PORT")]
    local_port: Option<u16>,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
enum ProtocolArg {
    Tcp,
    Udp,
}

impl From<ProtocolArg> for Protocol {
    fn from(value: ProtocolArg) -> Self {
        match value {
            ProtocolArg::Tcp => Protocol::Tcp,
            ProtocolArg::Udp => Protocol::Udp,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
enum FdKindArg {
    File,
    Socket,
    Pipe,
    Unknown,
}

impl From<FdKindArg> for FdKind {
    fn from(value: FdKindArg) -> Self {
        match value {
            FdKindArg::File => FdKind::File,
            FdKindArg::Socket => FdKind::Socket,
            FdKindArg::Pipe => FdKind::Pipe,
            FdKindArg::Unknown => FdKind::Unknown,
        }
    }
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
        Command::Descendants(args) => run_descendants(args),
        Command::KillDescendants(args) => run_kill_descendants(args),
        Command::Fds(args) => run_fds(args),
        Command::Ports(args) => run_ports(args),
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

    if args.group
        && (args.ppid.is_some()
            || args.name.is_some()
            || args.user.is_some()
            || args.cpu_above.is_some()
            || args.memory_above.is_some()
            || args.running_for.is_some())
    {
        return Err(SysprimsError::invalid_argument(
            "--group cannot be combined with process filters",
        ));
    }

    maybe_emit_cpu_above_hint(args.cpu_above, args.json, CpuMode::Lifetime, false);

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

    let running_for_secs = args
        .running_for
        .as_deref()
        .map(parse_duration)
        .transpose()?
        .map(|d| d.as_secs());

    let filter_used = args.ppid.is_some()
        || args.name.is_some()
        || args.user.is_some()
        || args.cpu_above.is_some()
        || args.memory_above.is_some()
        || running_for_secs.is_some();

    if args.pids.is_empty() && !filter_used {
        return Err(SysprimsError::invalid_argument(
            "kill requires at least one PID or a filter (e.g. --name/--ppid/--cpu-above)",
        ));
    }

    // When using any filters, resolve the target PIDs via snapshot filtering.
    // This gives AND semantics across all specified options.
    let (targets, filter_snapshot) = if filter_used {
        let filter = ProcessFilter {
            name_contains: args.name.clone(),
            user_equals: args.user.clone(),
            cpu_above: args.cpu_above,
            memory_above_kb: args.memory_above,
            pid_in: if args.pids.is_empty() {
                None
            } else {
                Some(args.pids.clone())
            },
            ppid: args.ppid,
            running_for_at_least_secs: running_for_secs,
            ..Default::default()
        };

        let snap = snapshot_filtered(&filter)?;
        let mut pids: Vec<u32> = snap.processes.iter().map(|p| p.pid).collect();
        pids.sort_unstable();
        pids.dedup();
        (pids, Some(snap))
    } else {
        (args.pids.clone(), None)
    };

    if targets.is_empty() {
        if args.json {
            let out = BatchKillResultJson {
                schema_id,
                signal_sent: signal_num,
                succeeded: vec![],
                failed: vec![],
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&out).expect("serialize json")
            );
        }
        return Ok(0);
    }

    // Apply additional CLI safety checks only when targets were selected via filters.
    // (Explicit PIDs preserve the existing "do what I said" behavior.)
    let mut safe_targets = targets;
    if filter_used && !args.force {
        let self_pid = std::process::id();
        let parent_pid = get_process(self_pid).ok().map(|p| p.ppid);

        let before = safe_targets.len();
        safe_targets.retain(|&pid| pid != self_pid);
        safe_targets.retain(|&pid| pid != 1);
        if let Some(ppid) = parent_pid {
            safe_targets.retain(|&pid| pid != ppid);
        }
        let removed = before.saturating_sub(safe_targets.len());
        if removed > 0 {
            eprintln!(
                "Skipped {removed} unsafe targets (self/PID1/parent); use --force to override"
            );
        }
    }

    // Default to a preview when selecting by filters unless --yes is provided.
    if args.dry_run || (filter_used && args.pids.is_empty() && !args.yes) {
        if args.json {
            if let Some(snap) = filter_snapshot {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&snap).expect("serialize json")
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&safe_targets).expect("serialize json")
                );
            }
        } else {
            for pid in &safe_targets {
                println!("{pid}");
            }
            if filter_used && args.pids.is_empty() && !args.yes && !args.dry_run {
                eprintln!("Refusing to send signals for filter-based selection without --yes (use --dry-run to preview)");
            }
        }

        return Ok(0);
    }

    // Non-group: multi-PID supported.
    let batch = sysprims_signal::kill_many(&safe_targets, signal_num)?;
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
// Descendants command
// ============================================================================

/// Parse --max-levels value: accepts a positive integer or "all" (→ u32::MAX).
fn parse_max_levels(s: &str) -> Result<u32, SysprimsError> {
    if s.eq_ignore_ascii_case("all") {
        return Ok(u32::MAX);
    }
    s.parse::<u32>().map_err(|_| {
        SysprimsError::invalid_argument(format!(
            "invalid max-levels '{}': expected a positive integer or 'all'",
            s
        ))
    })
}

/// Build a ProcessFilter from descendants/kill-descendants shared args.
fn build_descendants_filter(
    name: &Option<String>,
    user: &Option<String>,
    cpu_above: Option<f64>,
    memory_above: Option<u64>,
    running_for: &Option<String>,
) -> Result<Option<ProcessFilter>, SysprimsError> {
    let running_for_secs = running_for
        .as_deref()
        .map(parse_duration)
        .transpose()?
        .map(|d| d.as_secs());

    let has_filter = name.is_some()
        || user.is_some()
        || cpu_above.is_some()
        || memory_above.is_some()
        || running_for_secs.is_some();

    if !has_filter {
        return Ok(None);
    }

    Ok(Some(ProcessFilter {
        name_contains: name.clone(),
        user_equals: user.clone(),
        cpu_above,
        memory_above_kb: memory_above,
        running_for_at_least_secs: running_for_secs,
        ..Default::default()
    }))
}

fn to_proc_cpu_mode(mode: CpuMode) -> ProcCpuMode {
    match mode {
        CpuMode::Lifetime => ProcCpuMode::Lifetime,
        CpuMode::Monitor => ProcCpuMode::Monitor,
    }
}

fn cpu_mode_flag_explicit_from_argv() -> bool {
    std::env::args().any(|arg| arg == "--cpu-mode" || arg.starts_with("--cpu-mode="))
}

fn hints_disabled_from_env() -> bool {
    matches!(std::env::var("SYSPRIMS_NO_HINTS"), Ok(v) if v == "1")
}

fn should_emit_cpu_above_hint_base(
    cpu_above: Option<f64>,
    json_output: bool,
    cpu_mode: CpuMode,
    cpu_mode_explicit: bool,
    hints_disabled: bool,
) -> bool {
    cpu_above.is_some()
        && !json_output
        && cpu_mode == CpuMode::Lifetime
        && !cpu_mode_explicit
        && !hints_disabled
}

fn maybe_emit_cpu_above_hint(
    cpu_above: Option<f64>,
    json_output: bool,
    cpu_mode: CpuMode,
    cpu_mode_explicit: bool,
) {
    if should_emit_cpu_above_hint_base(
        cpu_above,
        json_output,
        cpu_mode,
        cpu_mode_explicit,
        hints_disabled_from_env(),
    ) {
        eprintln!(
            "hint: --cpu-above uses lifetime CPU averaging; spinning/bursty processes may not appear."
        );
        eprintln!("      try: --cpu-mode monitor --sample 3s");
    }
}

fn run_descendants(args: DescendantsArgs) -> Result<i32, SysprimsError> {
    maybe_emit_cpu_above_hint(
        args.cpu_above,
        args.json,
        args.cpu_mode.clone(),
        cpu_mode_flag_explicit_from_argv(),
    );

    let max_levels = parse_max_levels(&args.max_levels)?;

    let filter = build_descendants_filter(
        &args.name,
        &args.user,
        args.cpu_above,
        args.memory_above,
        &args.running_for,
    )?;
    let sample_duration = args.sample.as_deref().map(parse_duration).transpose()?;
    let config = DescendantsConfig {
        root_pid: args.pid,
        max_levels: Some(max_levels),
        filter,
        cpu_mode: to_proc_cpu_mode(args.cpu_mode),
        sample_duration,
    };

    let result = descendants_with_config(config)?;

    if args.tree {
        let root_info = get_process(args.pid).ok();
        print_descendants_tree(&result, root_info.as_ref());
    } else if args.table {
        for level in &result.levels {
            println!("--- Level {} ---", level.level);
            print_process_table(&level.processes);
        }
        println!(
            "\nTotal: {} descendants found, {} matched filter",
            result.total_found, result.matched_by_filter
        );
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).expect("serialize json")
        );
    }

    Ok(0)
}

// ============================================================================
// ASCII Tree Rendering (C2)
// ============================================================================

/// Format elapsed seconds as a human-readable duration string.
fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        if s == 0 {
            format!("{m}m")
        } else {
            format!("{m}m{s}s")
        }
    } else if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h{m}m")
        }
    } else {
        let d = secs / 86400;
        let h = (secs % 86400) / 3600;
        if h == 0 {
            format!("{d}d")
        } else {
            format!("{d}d{h}h")
        }
    }
}

/// Format memory in KB as a human-readable string.
fn format_memory(kb: u64) -> String {
    if kb < 1024 {
        format!("{kb}K")
    } else if kb < 1024 * 1024 {
        format!("{}M", kb / 1024)
    } else {
        format!("{:.1}G", kb as f64 / (1024.0 * 1024.0))
    }
}

/// CPU threshold indicator.
fn cpu_indicator(cpu: f64) -> &'static str {
    if cpu > 90.0 {
        " ★ HIGH"
    } else if cpu > 50.0 {
        " ⚠ WARN"
    } else {
        ""
    }
}

/// Extract a useful cmdline hint beyond the process name.
///
/// Returns a short snippet when the cmdline contains information not
/// already conveyed by the process name (e.g., extension identity).
fn cmdline_hint(name: &str, cmdline: &[String]) -> Option<String> {
    if cmdline.len() <= 1 {
        return None;
    }

    // Look for common patterns that identify the workload.
    // Example: VSCodium extension helpers have --type=... and extension names.
    for arg in &cmdline[1..] {
        // Skip short flags and paths.
        if arg.starts_with('-') && !arg.starts_with("--type=") {
            continue;
        }
        // Find extension identifiers (e.g., "vscode.markdown-language-features").
        if arg.contains("language-features")
            || arg.contains("extension-host")
            || arg.contains("vscode.")
        {
            // Return the last path segment as the hint.
            let hint = arg.rsplit('/').next().unwrap_or(arg);
            let hint = hint.rsplit('\\').next().unwrap_or(hint);
            if hint != name && hint.len() > 2 {
                let truncated = if hint.len() > 40 { &hint[..40] } else { hint };
                return Some(truncated.to_string());
            }
        }
    }
    None
}

/// Format a single tree node line.
fn format_tree_node(proc: &sysprims_proc::ProcessInfo) -> String {
    let mem = format_memory(proc.memory_kb);
    let elapsed = format_elapsed(proc.elapsed_seconds);
    let indicator = cpu_indicator(proc.cpu_percent);
    let hint = cmdline_hint(&proc.name, &proc.cmdline)
        .map(|h| format!(" ({h})"))
        .unwrap_or_default();

    format!(
        "{} {}{} [{:.1}% CPU, {}, {}]{}",
        proc.pid, proc.name, hint, proc.cpu_percent, mem, elapsed, indicator
    )
}

/// Print an ASCII art tree from a DescendantsResult.
fn print_descendants_tree(
    result: &sysprims_proc::DescendantsResult,
    root_info: Option<&sysprims_proc::ProcessInfo>,
) {
    // Print root node.
    if let Some(root) = root_info {
        println!("{}", format_tree_node(root));
    } else {
        println!("{} (root)", result.root_pid);
    }

    // Build a PID → children map from the level data.
    // Level 1 children have ppid == root_pid, level 2 have ppid in level 1, etc.
    let mut children_map: std::collections::HashMap<u32, Vec<&sysprims_proc::ProcessInfo>> =
        std::collections::HashMap::new();

    for level in &result.levels {
        for proc in &level.processes {
            children_map.entry(proc.ppid).or_default().push(proc);
        }
    }

    // Sort children within each parent by PID for stable output.
    for children in children_map.values_mut() {
        children.sort_by_key(|p| p.pid);
    }

    // Recursive tree printer.
    fn print_subtree(
        pid: u32,
        prefix: &str,
        children_map: &std::collections::HashMap<u32, Vec<&sysprims_proc::ProcessInfo>>,
    ) {
        if let Some(children) = children_map.get(&pid) {
            let count = children.len();
            for (i, child) in children.iter().enumerate() {
                let is_last = i == count - 1;
                let connector = if is_last { "└── " } else { "├── " };
                let node_line = format_tree_node(child);
                println!("{prefix}{connector}{node_line}");

                let child_prefix = if is_last {
                    format!("{prefix}    ")
                } else {
                    format!("{prefix}│   ")
                };
                print_subtree(child.pid, &child_prefix, children_map);
            }
        }
    }

    print_subtree(result.root_pid, "", &children_map);

    // Summary footer.
    let high_cpu = result
        .levels
        .iter()
        .flat_map(|l| &l.processes)
        .filter(|p| p.cpu_percent > 90.0)
        .count();
    let warn_cpu = result
        .levels
        .iter()
        .flat_map(|l| &l.processes)
        .filter(|p| p.cpu_percent > 50.0 && p.cpu_percent <= 90.0)
        .count();

    println!();
    println!(
        "Total: {} processes in subtree, {} matched filter",
        result.total_found, result.matched_by_filter
    );
    if high_cpu > 0 || warn_cpu > 0 {
        println!("★ = CPU > 90%, ⚠ = CPU > 50%");
    }
}

fn run_kill_descendants(args: KillDescendantsArgs) -> Result<i32, SysprimsError> {
    maybe_emit_cpu_above_hint(
        args.cpu_above,
        args.json,
        args.cpu_mode.clone(),
        cpu_mode_flag_explicit_from_argv(),
    );

    let max_levels = parse_max_levels(&args.max_levels)?;

    let filter = build_descendants_filter(
        &args.name,
        &args.user,
        args.cpu_above,
        args.memory_above,
        &args.running_for,
    )?;
    let sample_duration = args.sample.as_deref().map(parse_duration).transpose()?;
    let config = DescendantsConfig {
        root_pid: args.pid,
        max_levels: Some(max_levels),
        filter,
        cpu_mode: to_proc_cpu_mode(args.cpu_mode),
        sample_duration,
    };

    let result = descendants_with_config(config)?;

    // Collect all PIDs from all levels.
    let mut target_pids: Vec<u32> = result
        .levels
        .iter()
        .flat_map(|l| l.processes.iter().map(|p| p.pid))
        .collect();
    target_pids.sort_unstable();
    target_pids.dedup();

    // Never kill the root PID itself — descendants-only.
    target_pids.retain(|&pid| pid != args.pid);

    if target_pids.is_empty() {
        if args.json {
            let schema_id = BATCH_KILL_RESULT_V1;
            let out = BatchKillResultJson {
                schema_id,
                signal_sent: 0,
                succeeded: vec![],
                failed: vec![],
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&out).expect("serialize json")
            );
        }
        return Ok(0);
    }

    // Safety: drop self, PID 1, parent unless --force.
    if !args.force {
        let self_pid = std::process::id();
        let parent_pid = get_process(self_pid).ok().map(|p| p.ppid);

        let before = target_pids.len();
        target_pids.retain(|&pid| pid != self_pid && pid != 1);
        if let Some(ppid) = parent_pid {
            target_pids.retain(|&pid| pid != ppid);
        }
        let removed = before.saturating_sub(target_pids.len());
        if removed > 0 {
            eprintln!(
                "Skipped {removed} unsafe targets (self/PID1/parent); use --force to override"
            );
        }
    }

    // Default to preview unless --yes.
    if args.dry_run || !args.yes {
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("serialize json")
            );
        } else {
            for pid in &target_pids {
                println!("{pid}");
            }
            if !args.yes && !args.dry_run {
                eprintln!("Refusing to send signals without --yes (use --dry-run to preview)");
            }
        }
        return Ok(0);
    }

    // Parse signal.
    let signal_num = resolve_signal(&args.signal)?;

    let batch = sysprims_signal::kill_many(&target_pids, signal_num)?;
    let failed: Vec<BatchKillFailureJson> = batch
        .failed
        .into_iter()
        .map(|f| BatchKillFailureJson {
            pid: f.pid,
            error: f.error.to_string(),
        })
        .collect();

    if args.json {
        let schema_id = BATCH_KILL_RESULT_V1;
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
        for f in &failed {
            eprintln!("PID {}: {}", f.pid, f.error);
        }
        return Ok(1);
    }

    Ok(0)
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

    // If specific PID requested, route through snapshot envelope for schema compliance.
    if let Some(pid) = args.pid {
        // Preserve `get_process(pid)` error semantics (NotFound vs PermissionDenied),
        // while still returning a schema-compliant snapshot envelope for JSON output.
        let mut proc_opt = match get_process(pid) {
            Ok(p) => Some(p),
            Err(SysprimsError::NotFound { .. }) => None,
            Err(e) => return Err(e),
        };

        let mut sampled = false;
        if sampling {
            if sample_duration.is_zero() {
                return Err(SysprimsError::invalid_argument(
                    "sample duration must be > 0",
                ));
            }

            if let Some(ref proc0) = proc_opt {
                let sample = sample_duration;
                let start0 = proc0.start_time_unix_ms;
                let cpu0 = cpu_total_time_ns(proc0.pid)?;
                std::thread::sleep(sample);

                match get_process(pid) {
                    Ok(mut proc1) => {
                        // PID reuse guard: only compute if start time matches.
                        if start0.is_none()
                            || proc1.start_time_unix_ms.is_none()
                            || start0 == proc1.start_time_unix_ms
                        {
                            let cpu1 = cpu_total_time_ns(proc1.pid)?;
                            let dt_ns = sample.as_nanos() as f64;
                            let delta = cpu1.saturating_sub(cpu0) as f64;
                            if dt_ns > 0.0 {
                                proc1.cpu_percent = (delta / dt_ns) * 100.0;
                            }
                        }

                        proc_opt = Some(proc1);
                    }
                    Err(SysprimsError::NotFound { .. }) => {
                        proc_opt = None;
                    }
                    Err(e) => return Err(e),
                }

                // Sampling changes CPU semantics (can exceed 100 for multi-core).
                sampled = true;
            }
        }

        if args.table {
            if let Some(p) = proc_opt {
                print_process_table(&[p]);
                return Ok(0);
            }
            return Err(SysprimsError::not_found(pid));
        }

        // Default to JSON: always emit the snapshot envelope shape.
        // We reuse `snapshot()` as the source of timestamp + schema_id.
        let mut snap = snapshot()?;
        snap.processes = proc_opt.into_iter().collect();
        if sampled {
            snap.schema_id = PROCESS_INFO_SAMPLED_V1;
        }

        println!("{}", serde_json::to_string_pretty(&snap).unwrap());
        return Ok(if snap.processes.is_empty() { 1 } else { 0 });
    }

    // Parse --running-for duration.
    let running_for_secs = args
        .running_for
        .as_deref()
        .map(parse_duration)
        .transpose()?
        .map(|d| d.as_secs());

    // Build filter from args.
    // When sampling CPU, apply cpu_above after sampling so we don't filter out
    // processes due to lifetime-average CPU values.
    let base_filter = ProcessFilter {
        name_contains: args.name.clone(),
        user_equals: args.user.clone(),
        cpu_above: if sampling { None } else { args.cpu_above },
        memory_above_kb: args.memory_above,
        ppid: args.ppid,
        running_for_at_least_secs: running_for_secs,
        ..Default::default()
    };

    let has_filter = base_filter.name_contains.is_some()
        || base_filter.user_equals.is_some()
        || base_filter.cpu_above.is_some()
        || base_filter.memory_above_kb.is_some()
        || base_filter.ppid.is_some()
        || base_filter.running_for_at_least_secs.is_some();

    let mut snap = if has_filter {
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
        let mut snap1 = if has_filter {
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

// ============================================================================
// Fds command
// ============================================================================

fn fd_kind_str(kind: FdKind) -> &'static str {
    match kind {
        FdKind::File => "file",
        FdKind::Socket => "socket",
        FdKind::Pipe => "pipe",
        FdKind::Unknown => "unknown",
    }
}

fn run_fds(args: FdsArgs) -> Result<i32, SysprimsError> {
    let filter = args.kind.map(|k| FdFilter {
        kind: Some(k.into()),
    });

    let snapshot = match filter.as_ref() {
        Some(f) => list_fds(args.pid, Some(f))?,
        None => list_fds(args.pid, None)?,
    };

    if args.table {
        print_fd_table(&snapshot.fds);
        for w in snapshot.warnings {
            eprintln!("Warning: {w}");
        }
        return Ok(0);
    }

    // Default to JSON
    println!("{}", serde_json::to_string_pretty(&snapshot).unwrap());
    Ok(0)
}

fn print_fd_table(fds: &[sysprims_proc::FdInfo]) {
    println!("{:>5} {:<8} TARGET", "FD", "KIND");
    println!("{:-<80}", "");

    if fds.is_empty() {
        println!("(no visible file descriptors)");
        return;
    }

    for fd in fds {
        let target = fd.path.as_deref().unwrap_or("-");
        println!(
            "{:>5} {:<8} {}",
            fd.fd,
            fd_kind_str(fd.kind),
            truncate(target, 72)
        );
    }
}

// ============================================================================
// Ports command
// ============================================================================

fn protocol_str(p: Protocol) -> &'static str {
    match p {
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
    }
}

fn format_local_addr_port(addr: Option<std::net::IpAddr>, port: u16) -> String {
    match addr {
        Some(std::net::IpAddr::V4(a)) => format!("{}:{}", a, port),
        Some(std::net::IpAddr::V6(a)) => format!("[{}]:{}", a, port),
        None => format!("*:{}", port),
    }
}

fn run_ports(args: PortsArgs) -> Result<i32, SysprimsError> {
    let filter = PortFilter {
        protocol: args.protocol.map(Into::into),
        local_port: args.local_port,
    };

    let snapshot = if filter.protocol.is_some() || filter.local_port.is_some() {
        listening_ports(Some(&filter))?
    } else {
        listening_ports(None)?
    };

    if args.table {
        print_ports_table(&snapshot.bindings);
        for w in snapshot.warnings {
            eprintln!("Warning: {w}");
        }
        return Ok(0);
    }

    // Default to JSON
    println!("{}", serde_json::to_string_pretty(&snapshot).unwrap());
    Ok(0)
}

fn print_ports_table(bindings: &[sysprims_proc::PortBinding]) {
    println!(
        "{:>5} {:<22} {:<8} {:>7} NAME",
        "PROTO", "LOCAL", "STATE", "PID"
    );
    println!("{:-<80}", "");

    if bindings.is_empty() {
        println!("(no visible listening ports)");
        return;
    }

    for b in bindings {
        let local = format_local_addr_port(b.local_addr, b.local_port);
        let state = b.state.as_deref().unwrap_or("-");
        let pid = b
            .pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".to_string());
        let name = b.process.as_ref().map(|p| p.name.as_str()).unwrap_or("-");

        println!(
            "{:>5} {:<22} {:<8} {:>7} {}",
            protocol_str(b.protocol),
            truncate(&local, 22),
            truncate(state, 8),
            pid,
            truncate(name, 32)
        );
    }
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
    fn kill_parses_without_pid_but_runtime_rejects() {
        let cli = Cli::try_parse_from(["sysprims", "kill"]).unwrap();
        let Command::Kill(args) = cli.command.unwrap() else {
            panic!("expected kill command");
        };
        assert!(args.pids.is_empty());
        assert!(args.list.is_none());
    }

    #[test]
    fn kill_parses_filter_without_pid() {
        let cli = Cli::try_parse_from([
            "sysprims",
            "kill",
            "--name",
            "VSCodium Helper",
            "--cpu-above",
            "80",
        ])
        .unwrap();
        let Command::Kill(args) = cli.command.unwrap() else {
            panic!("expected kill command");
        };
        assert!(args.pids.is_empty());
        assert_eq!(args.name.as_deref(), Some("VSCodium Helper"));
        assert_eq!(args.cpu_above, Some(80.0));
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

    #[test]
    fn fds_requires_pid() {
        assert!(Cli::try_parse_from(["sysprims", "fds"]).is_err());
    }

    #[test]
    fn fds_parses_pid_and_kind() {
        let cli =
            Cli::try_parse_from(["sysprims", "fds", "--pid", "1234", "--kind", "socket"]).unwrap();
        let Command::Fds(args) = cli.command.unwrap() else {
            panic!("expected fds command");
        };
        assert_eq!(args.pid, 1234);
        assert!(matches!(args.kind, Some(FdKindArg::Socket)));
    }

    #[test]
    fn ports_parses_protocol_and_port() {
        let cli = Cli::try_parse_from([
            "sysprims",
            "ports",
            "--protocol",
            "tcp",
            "--local-port",
            "8080",
            "--table",
        ])
        .unwrap();
        let Command::Ports(args) = cli.command.unwrap() else {
            panic!("expected ports command");
        };

        assert!(args.table);
        assert!(!args.json);
        assert!(matches!(args.protocol, Some(ProtocolArg::Tcp)));
        assert_eq!(args.local_port, Some(8080));
    }

    #[test]
    fn descendants_parses_with_filters() {
        let cli = Cli::try_parse_from([
            "sysprims",
            "descendants",
            "1234",
            "--max-levels",
            "3",
            "--name",
            "Helper",
            "--cpu-above",
            "50",
            "--cpu-mode",
            "monitor",
            "--sample",
            "3s",
            "--running-for",
            "1m",
            "--tree",
        ])
        .unwrap();
        let Command::Descendants(args) = cli.command.unwrap() else {
            panic!("expected descendants command");
        };
        assert_eq!(args.pid, 1234);
        assert_eq!(args.max_levels, "3");
        assert_eq!(parse_max_levels(&args.max_levels).unwrap(), 3);
        assert_eq!(args.name.as_deref(), Some("Helper"));
        assert_eq!(args.cpu_above, Some(50.0));
        assert_eq!(args.cpu_mode, CpuMode::Monitor);
        assert_eq!(args.sample.as_deref(), Some("3s"));
        assert_eq!(args.running_for.as_deref(), Some("1m"));
        assert!(args.tree);
        assert!(!args.table);
        assert!(!args.json);
    }

    #[test]
    fn descendants_output_modes_conflict() {
        // --tree and --json conflict
        assert!(
            Cli::try_parse_from(["sysprims", "descendants", "1234", "--tree", "--json",]).is_err()
        );

        // --tree and --table conflict
        assert!(
            Cli::try_parse_from(["sysprims", "descendants", "1234", "--tree", "--table",]).is_err()
        );
    }

    #[test]
    fn kill_descendants_parses() {
        let cli = Cli::try_parse_from([
            "sysprims",
            "kill-descendants",
            "7825",
            "--max-levels",
            "2",
            "--signal",
            "KILL",
            "--cpu-above",
            "80",
            "--cpu-mode",
            "monitor",
            "--sample",
            "250ms",
            "--yes",
        ])
        .unwrap();
        let Command::KillDescendants(args) = cli.command.unwrap() else {
            panic!("expected kill-descendants command");
        };
        assert_eq!(args.pid, 7825);
        assert_eq!(args.max_levels, "2");
        assert_eq!(parse_max_levels(&args.max_levels).unwrap(), 2);
        assert_eq!(args.signal, "KILL");
        assert_eq!(args.cpu_above, Some(80.0));
        assert_eq!(args.cpu_mode, CpuMode::Monitor);
        assert_eq!(args.sample.as_deref(), Some("250ms"));
        assert!(args.yes);
    }

    #[test]
    fn pstat_parses_running_for() {
        let cli = Cli::try_parse_from([
            "sysprims",
            "pstat",
            "--cpu-above",
            "90",
            "--running-for",
            "5s",
        ])
        .unwrap();
        let Command::Pstat(args) = cli.command.unwrap() else {
            panic!("expected pstat command");
        };
        assert_eq!(args.cpu_above, Some(90.0));
        assert_eq!(args.running_for.as_deref(), Some("5s"));
    }

    #[test]
    fn cpu_above_hint_base_emits_for_lifetime_human_output() {
        assert!(should_emit_cpu_above_hint_base(
            Some(80.0),
            false,
            CpuMode::Lifetime,
            false,
            false,
        ));
    }

    #[test]
    fn cpu_above_hint_base_suppressed_for_json_output() {
        assert!(!should_emit_cpu_above_hint_base(
            Some(80.0),
            true,
            CpuMode::Lifetime,
            false,
            false,
        ));
    }

    #[test]
    fn cpu_above_hint_base_suppressed_for_monitor_mode() {
        assert!(!should_emit_cpu_above_hint_base(
            Some(80.0),
            false,
            CpuMode::Monitor,
            false,
            false,
        ));
    }

    #[test]
    fn cpu_above_hint_base_suppressed_when_cpu_mode_explicit() {
        assert!(!should_emit_cpu_above_hint_base(
            Some(80.0),
            false,
            CpuMode::Lifetime,
            true,
            false,
        ));
    }

    #[test]
    fn cpu_above_hint_base_suppressed_by_env_flag() {
        assert!(!should_emit_cpu_above_hint_base(
            Some(80.0),
            false,
            CpuMode::Lifetime,
            false,
            true,
        ));
    }

    #[test]
    fn format_elapsed_values() {
        assert_eq!(format_elapsed(0), "0s");
        assert_eq!(format_elapsed(45), "45s");
        assert_eq!(format_elapsed(60), "1m");
        assert_eq!(format_elapsed(90), "1m30s");
        assert_eq!(format_elapsed(3600), "1h");
        assert_eq!(format_elapsed(3661), "1h1m");
        assert_eq!(format_elapsed(86400), "1d");
        assert_eq!(format_elapsed(90000), "1d1h");
    }

    #[test]
    fn format_memory_values() {
        assert_eq!(format_memory(512), "512K");
        assert_eq!(format_memory(1024), "1M");
        assert_eq!(format_memory(75584), "73M");
        assert_eq!(format_memory(1048576), "1.0G");
    }

    #[test]
    fn cpu_indicator_thresholds() {
        assert_eq!(cpu_indicator(10.0), "");
        assert_eq!(cpu_indicator(50.0), "");
        assert_eq!(cpu_indicator(50.1), " ⚠ WARN");
        assert_eq!(cpu_indicator(90.0), " ⚠ WARN");
        assert_eq!(cpu_indicator(90.1), " ★ HIGH");
        assert_eq!(cpu_indicator(102.0), " ★ HIGH");
    }

    #[test]
    fn parse_max_levels_numeric() {
        assert_eq!(parse_max_levels("1").unwrap(), 1);
        assert_eq!(parse_max_levels("5").unwrap(), 5);
        assert_eq!(parse_max_levels("100").unwrap(), 100);
    }

    #[test]
    fn parse_max_levels_all_keyword() {
        assert_eq!(parse_max_levels("all").unwrap(), u32::MAX);
        assert_eq!(parse_max_levels("ALL").unwrap(), u32::MAX);
        assert_eq!(parse_max_levels("All").unwrap(), u32::MAX);
    }

    #[test]
    fn parse_max_levels_invalid() {
        assert!(parse_max_levels("abc").is_err());
        assert!(parse_max_levels("-1").is_err());
        assert!(parse_max_levels("").is_err());
    }

    #[test]
    fn descendants_accepts_all_keyword() {
        let cli = Cli::try_parse_from(["sysprims", "descendants", "1234", "--max-levels", "all"])
            .unwrap();
        let Command::Descendants(args) = cli.command.unwrap() else {
            panic!("expected descendants command");
        };
        assert_eq!(args.max_levels, "all");
        assert_eq!(parse_max_levels(&args.max_levels).unwrap(), u32::MAX);
    }

    #[test]
    fn kill_parses_running_for() {
        let cli = Cli::try_parse_from([
            "sysprims",
            "kill",
            "--name",
            "chrome",
            "--running-for",
            "30s",
        ])
        .unwrap();
        let Command::Kill(args) = cli.command.unwrap() else {
            panic!("expected kill command");
        };
        assert_eq!(args.name.as_deref(), Some("chrome"));
        assert_eq!(args.running_for.as_deref(), Some("30s"));
    }
}
