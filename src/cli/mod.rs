//! Command-line interface.
//!
//! One binary, many subcommands (SPEC.md §40, IMPLEMENTATION.md §1). Anything
//! not yet implemented says so rather than pretending to succeed.

mod config_cmd;
mod version_cmd;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::DEFAULT_CONFIG_PATH;
use crate::telemetry::LogFormat;

/// Cluster Sentinel.
#[derive(Debug, Parser)]
#[command(name = "sentinel", version, about, long_about = None)]
pub struct Cli {
    /// Path to the configuration file.
    #[arg(long, short = 'c', global = true, env = "SENTINEL_CONFIG", default_value = DEFAULT_CONFIG_PATH)]
    pub config: PathBuf,

    /// Increase log verbosity; repeat for more.
    #[arg(long, short = 'v', global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Emit logs as JSON.
    #[arg(long, global = true)]
    pub log_json: bool,

    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Log format implied by the flags.
    pub fn log_format(&self) -> LogFormat {
        if self.log_json {
            LogFormat::Json
        } else {
            LogFormat::Auto
        }
    }
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print version information.
    Version {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Inspect and validate configuration.
    Config {
        /// The configuration subcommand.
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

/// `sentinel config ...`.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Validate the configuration file.
    Check {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show the effective configuration and where each value came from.
    Show {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

/// Run the CLI. Returns the process exit code.
pub async fn run(cli: Cli) -> anyhow::Result<i32> {
    match &cli.command {
        Command::Version { json } => version_cmd::run(*json),
        Command::Config { command } => config_cmd::run(&cli, command).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn version_parses() {
        let cli = Cli::try_parse_from(["sentinel", "version"]).expect("parse");
        assert!(matches!(cli.command, Command::Version { json: false }));
    }

    #[test]
    fn config_check_parses_with_a_custom_path() {
        let cli = Cli::try_parse_from(["sentinel", "--config", "/tmp/x.toml", "config", "check"]).expect("parse");
        assert_eq!(cli.config, PathBuf::from("/tmp/x.toml"));
        assert!(matches!(
            cli.command,
            Command::Config {
                command: ConfigCommand::Check { json: false }
            }
        ));
    }

    #[test]
    fn the_config_path_defaults_to_the_documented_location() {
        let cli = Cli::try_parse_from(["sentinel", "version"]).expect("parse");
        assert_eq!(cli.config, PathBuf::from(DEFAULT_CONFIG_PATH));
    }

    #[test]
    fn verbosity_counts_up() {
        let cli = Cli::try_parse_from(["sentinel", "-vv", "version"]).expect("parse");
        assert_eq!(cli.verbose, 2);
    }

    #[test]
    fn an_unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["sentinel", "teleport"]).is_err());
    }
}
