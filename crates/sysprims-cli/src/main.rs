use clap::Parser;
use sysprims_core::get_platform;
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
    println!("Platform: {}", get_platform());
    info!("Main logic finished.");
}
