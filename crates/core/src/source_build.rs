use brewdock_formula::{Formula, FormulaName, NamedEntry, Requirement};

use crate::{
    HostTag, Layout,
    error::SourceBuildError,
    orchestrate::{SourceBuildPlan, pkg_version},
};

mod archive;
mod runner;

pub use archive::{extract_source_archive, source_archive_filename};
pub use runner::run_source_build;

pub fn build_source_plan(
    formula: &Formula,
    layout: &Layout,
    host_tag: &HostTag,
) -> Result<SourceBuildPlan, SourceBuildError> {
    if let Some(requirement) = formula
        .requirements
        .iter()
        .find(|requirement| !is_satisfied_system_requirement(requirement, host_tag))
    {
        return Err(SourceBuildError::UnsupportedRequirement(
            requirement_name(requirement).to_owned(),
        ));
    }

    let stable = formula
        .urls
        .stable
        .as_ref()
        .ok_or_else(|| SourceBuildError::UnsupportedSourceArchive(formula.name.clone()))?;
    let source_checksum = stable.checksum.clone().ok_or_else(|| {
        SourceBuildError::MissingSourceChecksum(FormulaName::from(formula.name.clone()))
    })?;
    if archive::source_archive_kind(&stable.url).is_none() {
        return Err(SourceBuildError::UnsupportedSourceArchive(
            stable.url.clone(),
        ));
    }
    let version = pkg_version(&formula.versions.stable, formula.revision);
    let cellar_path = layout.cellar().join(&formula.name).join(&version);
    Ok(SourceBuildPlan {
        formula_name: formula.name.clone(),
        version,
        source_url: stable.url.clone(),
        source_checksum: Some(source_checksum),
        build_dependencies: formula.build_dependencies.clone(),
        runtime_dependencies: formula.dependencies.clone(),
        prefix: layout.prefix().to_path_buf(),
        cellar_path,
    })
}

fn requirement_name(requirement: &Requirement) -> &str {
    match requirement {
        Requirement::Name(name) => name,
        Requirement::Detailed(detail) => &detail.name,
    }
}

fn is_satisfied_system_requirement(requirement: &Requirement, host_tag: &HostTag) -> bool {
    match requirement {
        Requirement::Name(name) => {
            matches!(name.as_str(), "xcode" | "macos" | "arch" | "maximum_macos")
        }
        Requirement::Detailed(detail) => match detail.name.as_str() {
            "arch" => matches_arch_requirement(detail, host_tag),
            "macos" => matches_minimum_macos_requirement(detail, host_tag),
            "maximum_macos" => matches_maximum_macos_requirement(detail, host_tag),
            "xcode" => matches_xcode_requirement(detail),
            _ => false,
        },
    }
}

fn matches_arch_requirement(detail: &NamedEntry, host_tag: &HostTag) -> bool {
    detail
        .version
        .as_deref()
        .is_none_or(|required_arch| required_arch == host_tag.arch())
}

fn matches_minimum_macos_requirement(detail: &NamedEntry, host_tag: &HostTag) -> bool {
    matches_macos_version_bound(detail, host_tag, |host, required| host >= required)
}

fn matches_maximum_macos_requirement(detail: &NamedEntry, host_tag: &HostTag) -> bool {
    matches_macos_version_bound(detail, host_tag, |host, required| host <= required)
}

fn matches_macos_version_bound<F>(detail: &NamedEntry, host_tag: &HostTag, satisfies: F) -> bool
where
    F: Fn(u16, u16) -> bool,
{
    detail.version.as_deref().is_none_or(|required_version| {
        parse_requirement_major(required_version)
            .zip(host_tag.macos_major())
            .is_some_and(|(required, host)| satisfies(host, required))
    })
}

fn matches_xcode_requirement(detail: &NamedEntry) -> bool {
    detail.version.as_deref().is_none_or(|required_version| {
        parse_requirement_major(required_version)
            .zip(detect_xcode_major_version())
            .is_some_and(|(required, host)| host >= required)
    })
}

fn parse_requirement_major(value: &str) -> Option<u16> {
    value.split('.').next()?.parse().ok()
}

fn detect_xcode_major_version() -> Option<u16> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Option<u16>> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let output = std::process::Command::new("xcodebuild")
            .arg("-version")
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .lines()
            .find_map(|line| line.strip_prefix("Xcode "))
            .and_then(parse_requirement_major)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detailed_macos_requirement_rejects_future_host()
    -> Result<(), Box<dyn std::error::Error>> {
        let host_tag: HostTag = "arm64_sequoia".parse()?;
        let requirement = Requirement::Detailed(NamedEntry {
            name: "macos".to_owned(),
            version: Some("26".to_owned()),
            contexts: Vec::new(),
            specs: Vec::new(),
        });

        assert!(!is_satisfied_system_requirement(&requirement, &host_tag));
        Ok(())
    }

    #[test]
    fn test_detailed_arch_requirement_rejects_mismatch() -> Result<(), Box<dyn std::error::Error>> {
        let host_tag: HostTag = "arm64_sequoia".parse()?;
        let requirement = Requirement::Detailed(NamedEntry {
            name: "arch".to_owned(),
            version: Some("x86_64".to_owned()),
            contexts: Vec::new(),
            specs: Vec::new(),
        });

        assert!(!is_satisfied_system_requirement(&requirement, &host_tag));
        Ok(())
    }

    #[test]
    fn test_detailed_maximum_macos_requirement_allows_compatible_host()
    -> Result<(), Box<dyn std::error::Error>> {
        let host_tag: HostTag = "arm64_sequoia".parse()?;
        let requirement = Requirement::Detailed(NamedEntry {
            name: "maximum_macos".to_owned(),
            version: Some("26".to_owned()),
            contexts: Vec::new(),
            specs: Vec::new(),
        });

        assert!(is_satisfied_system_requirement(&requirement, &host_tag));
        Ok(())
    }
}
