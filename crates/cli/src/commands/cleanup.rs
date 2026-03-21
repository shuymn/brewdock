use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::Verbosity;

/// Runs the cleanup command.
///
/// # Errors
///
/// Returns an error if cleanup fails.
pub fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    dry_run: bool,
    verbosity: Verbosity,
) -> Result<()> {
    let result = orchestrator.cleanup(dry_run).context("cleanup failed")?;

    if !verbosity.is_quiet() {
        let action = if dry_run { "Would remove" } else { "Removed" };
        let total = result.blobs_removed + result.stores_removed;
        if total == 0 {
            println!("Nothing to clean up");
        } else {
            println!(
                "{action} {blobs} blob(s) and {stores} store(s), freeing {bytes}",
                blobs = result.blobs_removed,
                stores = result.stores_removed,
                bytes = format_bytes(result.bytes_freed),
            );
        }
    }

    Ok(())
}

#[allow(clippy::cast_precision_loss)]
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1_048_576), "1.0 MiB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
    }
}
