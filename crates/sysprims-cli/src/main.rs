use std::time::Duration;

use clap::{Parser, Subcommand};
use sysprims_core::get_platform;
use sysprims_core::SysprimsError;
use sysprims_signal::{kill, kill_by_name, match_signal_names};
use sysprims_timeout::{run_with_timeout, GroupingMode, TimeoutConfig, TimeoutOutcome};
use tracing::info;
use tracing_subscriber::{filter::EnvFilter, fmt, prelude::*};

/// A cross-platform process utility toolkit.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
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
}

#[derive(Parser, Debug)]
struct KillArgs {
    /// Target process ID.
    pid: u32,

    /// Signal name, pattern, or number (default: TERM).
    #[arg(short = 's', long = "signal", value_name = "SIGNAL", default_value = "TERM")]
    signal: String,
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
    #[arg(short = 's', long = "signal", value_name = "SIGNAL", default_value = "TERM")]
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
    #[arg(long)]
    preserve_status: bool,
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
        Command::Kill(args) => {
            run_kill(args)?;
            Ok(0)
        }
        Command::Timeout(args) => run_timeout(args),
    }
}

enum SignalTarget {
    Number(i32),
    Name(String),
}

fn parse_signal_arg(signal_arg: &str) -> Result<SignalTarget, SysprimsError> {
    let trimmed = signal_arg.trim();
    if trimmed.is_empty() {
        return Err(SysprimsError::invalid_argument(
            "signal cannot be empty",
        ));
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

fn run_kill(args: KillArgs) -> Result<(), SysprimsError> {
    let signal = parse_signal_arg(&args.signal)?;
    match signal {
        SignalTarget::Number(number) => kill(args.pid, number),
        SignalTarget::Name(name) => kill_by_name(args.pid, &name),
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
                // Return 128 + signal number
                Ok(128 + signal_sent)
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

/// Parse a duration string like "5s", "100ms", "2m", "1h", or just "5" (seconds).
fn parse_duration(s: &str) -> Result<Duration, SysprimsError> {
    let s = s.trim();

    // Try to parse as plain number (seconds)
    if let Ok(secs) = s.parse::<f64>() {
        if secs < 0.0 {
            return Err(SysprimsError::invalid_argument("duration cannot be negative"));
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
        return Err(SysprimsError::invalid_argument("duration cannot be negative"));
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
        .or_else(|| sysprims_signal::get_signal_number(&format!("SIG{}", trimmed.to_ascii_uppercase())))
        .ok_or_else(|| SysprimsError::invalid_argument(format!("unknown signal '{}'", trimmed)))
}
