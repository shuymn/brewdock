#![warn(clippy::pedantic, clippy::nursery)]

use anyhow::{Context, Result};
use brewdock_core::{HostTag, HttpBottleDownloader, HttpFormulaRepository, Layout, Orchestrator};
use clap::Parser;

mod commands;
mod hint;
mod trace;
mod verbosity;

#[cfg(test)]
mod testutil;

pub(crate) use verbosity::Verbosity;

/// Fast Homebrew bottle installer.
#[derive(Parser)]
#[command(name = "bd", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Show what would be done without executing.
    #[arg(long, global = true)]
    dry_run: bool,

    /// Increase log detail.
    #[arg(long, global = true, conflicts_with = "quiet")]
    verbose: bool,

    /// Suppress non-error output.
    #[arg(long, global = true, conflicts_with = "verbose")]
    quiet: bool,
}

impl Cli {
    const fn verbosity(&self) -> Verbosity {
        if self.verbose {
            Verbosity::Verbose
        } else if self.quiet {
            Verbosity::Quiet
        } else {
            Verbosity::Normal
        }
    }
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Install formulae.
    Install {
        /// Formula names to install.
        #[arg(required = true)]
        formulae: Vec<String>,
    },
    /// Update formula index.
    Update,
    /// Upgrade installed formulae.
    Upgrade {
        /// Formula names to upgrade (all if empty).
        formulae: Vec<String>,
    },
    /// Show outdated formulae.
    Outdated {
        /// Formula names to check (all if empty).
        formulae: Vec<String>,
    },
    /// Search available formulae.
    Search {
        /// Search pattern (substring match).
        pattern: String,
    },
    /// Show formula information.
    Info {
        /// Formula name.
        formula: String,
    },
    /// List installed formulae.
    List,
    /// Remove stale caches and downloads.
    Cleanup,
    /// Check for potential problems.
    Doctor,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let verbosity = cli.verbosity();
    trace::init_tracing(verbosity).context("tracing init failed")?;

    let layout = Layout::production();
    let host_tag = HostTag::detect().context("platform detection failed")?;
    let orchestrator = Orchestrator::new(
        HttpFormulaRepository::new(),
        HttpBottleDownloader::new(),
        layout,
        host_tag,
    );
    let result = match cli.command {
        Commands::Install { formulae } => {
            commands::install::run(&orchestrator, &formulae, cli.dry_run, verbosity).await
        }
        Commands::Update => commands::update::run(&orchestrator, cli.dry_run, verbosity).await,
        Commands::Upgrade { formulae } => {
            Box::pin(commands::upgrade::run(
                &orchestrator,
                &formulae,
                cli.dry_run,
                verbosity,
            ))
            .await
        }
        Commands::Outdated { formulae } => {
            commands::outdated::run(&orchestrator, &formulae, verbosity).await
        }
        Commands::Search { pattern } => {
            commands::search::run(&orchestrator, &pattern, verbosity).await
        }
        Commands::Info { formula } => commands::info::run(&orchestrator, &formula, verbosity).await,
        Commands::List => commands::list::run(&orchestrator, verbosity),
        Commands::Cleanup => commands::cleanup::run(&orchestrator, cli.dry_run, verbosity),
        Commands::Doctor => commands::doctor::run(&orchestrator, verbosity),
    };

    if let Err(err) = &result
        && let Some(hint) = hint::for_error(err)
    {
        eprintln!("hint: {hint}");
    }

    result
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_parses_single_formula() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "install", "jq"])?;
        assert!(matches!(
            cli.command,
            Commands::Install { ref formulae } if formulae.len() == 1 && formulae[0] == "jq"
        ));
        Ok(())
    }

    #[test]
    fn test_install_parses_multiple_formulae() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "install", "jq", "wget"])?;
        assert!(matches!(
            cli.command,
            Commands::Install { ref formulae } if formulae.len() == 2
                && formulae[0] == "jq"
                && formulae[1] == "wget"
        ));
        Ok(())
    }

    #[test]
    fn test_install_requires_at_least_one_formula() {
        let result = Cli::try_parse_from(["bd", "install"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_parses() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "update"])?;
        assert!(matches!(cli.command, Commands::Update));
        Ok(())
    }

    #[test]
    fn test_upgrade_parses_without_args() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "upgrade"])?;
        assert!(matches!(
            cli.command,
            Commands::Upgrade { ref formulae } if formulae.is_empty()
        ));
        Ok(())
    }

    #[test]
    fn test_upgrade_parses_with_formula() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "upgrade", "jq"])?;
        assert!(matches!(
            cli.command,
            Commands::Upgrade { ref formulae } if formulae.len() == 1 && formulae[0] == "jq"
        ));
        Ok(())
    }

    #[test]
    fn test_dry_run_flag_parsed() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "--dry-run", "install", "jq"])?;
        assert!(cli.dry_run);
        Ok(())
    }

    #[test]
    fn test_dry_run_flag_after_subcommand() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "install", "--dry-run", "jq"])?;
        assert!(cli.dry_run);
        Ok(())
    }

    #[test]
    fn test_verbose_flag_parsed() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "--verbose", "install", "jq"])?;
        assert_eq!(cli.verbosity(), Verbosity::Verbose);
        Ok(())
    }

    #[test]
    fn test_quiet_flag_parsed() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "--quiet", "install", "jq"])?;
        assert_eq!(cli.verbosity(), Verbosity::Quiet);
        Ok(())
    }

    #[test]
    fn test_verbose_and_quiet_conflict() {
        let result = Cli::try_parse_from(["bd", "--verbose", "--quiet", "install", "jq"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_outdated_parses_without_args() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "outdated"])?;
        assert!(matches!(
            cli.command,
            Commands::Outdated { ref formulae } if formulae.is_empty()
        ));
        Ok(())
    }

    #[test]
    fn test_outdated_parses_with_formula() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "outdated", "jq"])?;
        assert!(matches!(
            cli.command,
            Commands::Outdated { ref formulae } if formulae.len() == 1 && formulae[0] == "jq"
        ));
        Ok(())
    }

    #[test]
    fn test_search_parses() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "search", "jq"])?;
        assert!(matches!(
            cli.command,
            Commands::Search { ref pattern } if pattern == "jq"
        ));
        Ok(())
    }

    #[test]
    fn test_search_requires_pattern() {
        let result = Cli::try_parse_from(["bd", "search"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_info_parses() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "info", "jq"])?;
        assert!(matches!(
            cli.command,
            Commands::Info { ref formula } if formula == "jq"
        ));
        Ok(())
    }

    #[test]
    fn test_info_requires_formula() {
        let result = Cli::try_parse_from(["bd", "info"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_list_parses() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "list"])?;
        assert!(matches!(cli.command, Commands::List));
        Ok(())
    }

    #[test]
    fn test_cleanup_parses() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "cleanup"])?;
        assert!(matches!(cli.command, Commands::Cleanup));
        Ok(())
    }

    #[test]
    fn test_doctor_parses() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["bd", "doctor"])?;
        assert!(matches!(cli.command, Commands::Doctor));
        Ok(())
    }
}
