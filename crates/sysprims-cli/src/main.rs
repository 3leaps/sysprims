use clap::{Parser, Subcommand};
use sysprims_core::get_platform;
use sysprims_core::SysprimsError;
use sysprims_signal::{kill, kill_by_name, match_signal_names};
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
}

#[derive(Parser, Debug)]
struct KillArgs {
    /// Target process ID.
    pid: u32,

    /// Signal name, pattern, or number (default: TERM).
    #[arg(short = 's', long = "signal", value_name = "SIGNAL", default_value = "TERM")]
    signal: String,
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
        if let Err(err) = run_command(command) {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    } else {
        println!("Platform: {}", get_platform());
    }
    info!("Main logic finished.");
}

fn run_command(command: Command) -> Result<(), SysprimsError> {
    match command {
        Command::Kill(args) => run_kill(args),
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
