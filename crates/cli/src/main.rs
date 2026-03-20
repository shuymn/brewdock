#![warn(clippy::pedantic, clippy::nursery, clippy::cargo)]

use anyhow::Result;
use clap::Parser;

mod commands;

/// Fast Homebrew bottle installer.
#[derive(Parser)]
#[command(name = "bd", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
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
}

fn main() -> Result<()> {
    // Intentionally discard: fails harmlessly if a subscriber is already set (e.g., in tests).
    let _ = tracing_subscriber::fmt::try_init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Install { formulae } => commands::install::run(&formulae),
        Commands::Update => commands::update::run(),
        Commands::Upgrade { formulae } => commands::upgrade::run(&formulae),
    }
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
}
