use std::fmt::Write;

use brewdock_core::{
    CleanupResult, DiagnosticEntry, FormulaInfo, InstalledKeg, OutdatedEntry, PlanEntry,
    UpgradePlanEntry,
};

pub fn render_install_plan(plan: &[PlanEntry]) -> String {
    if plan.is_empty() {
        return "Nothing to install.\n".to_owned();
    }

    let mut output = String::from("Install plan\n");
    for entry in plan {
        let _ = writeln!(
            output,
            "  - {} {} [{}]",
            entry.name, entry.version, entry.method
        );
    }
    output
}

pub fn render_install_summary(installed: &[String]) -> String {
    if installed.is_empty() {
        return "Nothing installed.\n".to_owned();
    }

    let mut output = String::from("Installed\n");
    for name in installed {
        let _ = writeln!(output, "  - {name}");
    }
    output
}

pub fn render_update_dry_run() -> String {
    "Update plan\n  - refresh formula index\n".to_owned()
}

pub fn render_update_summary(count: usize) -> String {
    format!("Updated formula index\n  - cached {count} formulae\n")
}

pub fn render_upgrade_plan(plan: &[UpgradePlanEntry]) -> String {
    if plan.is_empty() {
        return "Already up to date.\n".to_owned();
    }

    let mut output = String::from("Upgrade plan\n");
    for entry in plan {
        let _ = writeln!(
            output,
            "  - {} {} -> {} [{}]",
            entry.name, entry.from_version, entry.to_version, entry.method
        );
    }
    output
}

pub fn render_upgrade_summary(upgraded: &[String]) -> String {
    if upgraded.is_empty() {
        return "Already up to date.\n".to_owned();
    }

    let mut output = String::from("Upgraded\n");
    for name in upgraded {
        let _ = writeln!(output, "  - {name}");
    }
    output
}

pub fn render_search_results(pattern: &str, results: &[String]) -> String {
    if results.is_empty() {
        return format!("No formulae found for \"{pattern}\".\n");
    }

    let mut output = format!("Search results for \"{pattern}\"\n");
    for name in results {
        let _ = writeln!(output, "  - {name}");
    }
    output
}

pub fn render_info(info: &FormulaInfo) -> String {
    let mut output = format!("{} {}", info.name, info.version);
    if info.keg_only {
        output.push_str(" [keg-only]");
    }
    output.push('\n');

    if let Some(desc) = &info.desc {
        let _ = writeln!(output, "Description: {desc}");
    }
    if let Some(homepage) = &info.homepage {
        let _ = writeln!(output, "Homepage: {homepage}");
    }
    if let Some(license) = &info.license {
        let _ = writeln!(output, "License: {license}");
    }

    let bottle = if info.bottle_available {
        "available"
    } else {
        "not available"
    };
    let _ = writeln!(output, "Bottle: {bottle}");

    match &info.installed_version {
        Some(installed) => {
            let _ = writeln!(output, "Installed: {installed}");
        }
        None => output.push_str("Installed: no\n"),
    }

    if !info.dependencies.is_empty() {
        let _ = writeln!(output, "Dependencies: {}", info.dependencies.join(", "));
    }

    output
}

pub fn render_list(kegs: &[InstalledKeg]) -> String {
    if kegs.is_empty() {
        return "No formulae installed.\n".to_owned();
    }

    let mut output = String::from("Installed formulae\n");
    for keg in kegs {
        let _ = writeln!(output, "  - {} {}", keg.name, keg.pkg_version);
    }
    output
}

pub fn render_outdated(entries: &[OutdatedEntry]) -> String {
    if entries.is_empty() {
        return "All formulae are up to date.\n".to_owned();
    }

    let mut output = String::from("Outdated formulae\n");
    for entry in entries {
        let _ = writeln!(
            output,
            "  - {} {} -> {}",
            entry.name, entry.current_version, entry.latest_version
        );
    }
    output
}

pub fn render_cleanup(result: &CleanupResult, dry_run: bool) -> String {
    let action = if dry_run {
        "Cleanup plan"
    } else {
        "Cleanup complete"
    };
    let total = result.blobs_removed + result.stores_removed;
    if total == 0 {
        return format!("{action}\n  - nothing to clean up\n");
    }

    format!(
        "{action}\n  - blobs: {blobs}\n  - stores: {stores}\n  - freed: {bytes}\n",
        blobs = result.blobs_removed,
        stores = result.stores_removed,
        bytes = format_bytes(result.bytes_freed),
    )
}

pub fn render_doctor(diagnostics: &[DiagnosticEntry]) -> String {
    let mut output = String::from("Doctor\n");
    for entry in diagnostics {
        let _ = writeln!(output, "  - [{}] {}", entry.category, entry.message);
    }
    output
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
    use brewdock_core::{DiagnosticCategory, InstallMethod, SourceBuildPlan};

    use super::*;

    #[test]
    fn test_render_install_plan() {
        let plan = vec![PlanEntry {
            name: "jq".to_owned(),
            version: "1.7".to_owned(),
            method: InstallMethod::Source(SourceBuildPlan {
                formula_name: "jq".to_owned(),
                version: "1.7".to_owned(),
                source_url: "https://example.com/jq.tar.gz".to_owned(),
                source_checksum: Some("abc".to_owned()),
                build_dependencies: Vec::new(),
                runtime_dependencies: Vec::new(),
                prefix: "/opt/homebrew".into(),
                cellar_path: "/opt/homebrew/Cellar/jq/1.7".into(),
            }),
        }];

        let rendered = render_install_plan(&plan);
        assert!(rendered.contains("Install plan"));
        assert!(rendered.contains("jq 1.7 [source]"));
    }

    #[test]
    fn test_render_search_results() {
        let rendered = render_search_results("jq", &["jq".to_owned()]);
        assert!(rendered.contains("Search results"));
        assert!(rendered.contains("jq"));
    }

    #[test]
    fn test_render_doctor() {
        let rendered = render_doctor(&[DiagnosticEntry {
            category: DiagnosticCategory::Warning,
            message: "cache is stale".to_owned(),
        }]);
        assert!(rendered.contains("[warning] cache is stale"));
    }

    #[test]
    fn test_render_cleanup_empty() {
        let rendered = render_cleanup(
            &CleanupResult {
                blobs_removed: 0,
                stores_removed: 0,
                bytes_freed: 0,
            },
            false,
        );
        assert!(rendered.contains("nothing to clean up"));
    }
}
