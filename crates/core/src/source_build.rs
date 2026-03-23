use brewdock_formula::{Formula, FormulaName, Requirement};

use crate::{
    Layout,
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
) -> Result<SourceBuildPlan, SourceBuildError> {
    if let Some(requirement) = formula.requirements.first() {
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
