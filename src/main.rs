use std::path::PathBuf;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use commands::LogKind;
use nbac::ui;

mod commands;

/// Nix builder on Apple container: an on-demand aarch64-linux remote builder.
#[derive(Parser)]
#[command(name = "nbac", version)]
struct Cli {
    /// Path to the configuration file
    #[arg(
        long,
        global = true,
        default_value = "/etc/nbac/config.toml",
        value_name = "PATH"
    )]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Verify the runtime, generate keys, build the image, and boot the machine
    Setup,
    /// Probe runtime, machine, SSH, and remote store health
    Status,
    /// Start the builder machine
    Start,
    /// Stop the builder machine
    Stop,
    /// Destroy and recreate the machine (deletes the guest /nix store)
    Reset,
    /// Open an interactive SSH session on the builder
    Ssh {
        /// Arguments passed through to ssh
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run as SSH ProxyCommand: ensure the machine is up, then exec the transport
    #[command(hide = true)]
    Proxy,
    /// Check runtime health, reachability, and the image contract
    Doctor {
        /// Apply recovery steps
        #[arg(long)]
        fix: bool,
    },
    /// Build a trivial aarch64-linux derivation on the builder
    Test,
    /// Run nix-collect-garbage inside the guest
    Gc,
    /// Show guest logs
    Logs {
        /// Log to show
        log: LogKind,
        /// Keep the log open and print new entries
        #[arg(long)]
        follow: bool,
        /// Number of trailing lines to print
        #[arg(long, value_name = "N")]
        lines: Option<u64>,
    },
    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },
}

fn main() {
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let config = cli.config;
    match cli.command {
        Command::Setup => commands::cmd_setup(&config),
        Command::Status => commands::cmd_status(&config),
        Command::Start => commands::cmd_start(&config),
        Command::Stop => commands::cmd_stop(&config),
        Command::Reset => commands::cmd_reset(&config),
        Command::Ssh { args } => commands::cmd_ssh(&config, &args),
        Command::Proxy => commands::cmd_proxy(&config),
        Command::Doctor { fix } => commands::cmd_doctor(&config, fix),
        Command::Test => commands::cmd_test(&config),
        Command::Gc => commands::cmd_gc(&config),
        Command::Logs { log, follow, lines } => commands::cmd_logs(&config, log, follow, lines),
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "nbac", &mut std::io::stdout());
            Ok(())
        }
    }
}
