use std::path::Path;

use brewdock_cellar::{
    InstallReason, InstallReceipt, ReceiptDependency, ReceiptSource, ReceiptSourceVersions,
    RelocationManifest, RelocationScope, atomic_symlink_replace, materialize,
    relocate_keg_with_manifest, unlink,
};
use brewdock_formula::{Formula, FormulaCache};

use crate::{
    BrewdockError,
    orchestrate::{
        InstallMethod, MaterializedPayload, PendingSourcePayload, PrefetchedPayload, pkg_version,
    },
};

/// Default tap name for receipt source metadata.
const TAP_NAME: &str = "homebrew/core";

/// Prefix for formula source paths in receipt metadata.
const FORMULA_PATH_PREFIX: &str = "@@HOMEBREW_PREFIX@@/Library/Taps/homebrew/homebrew-core/Formula";

pub fn build_receipt(
    method: &InstallMethod,
    install_reason: InstallReason,
    time: Option<f64>,
    runtime_dependencies: Vec<ReceiptDependency>,
    source: ReceiptSource,
) -> InstallReceipt {
    match method {
        InstallMethod::Bottle(_) => {
            InstallReceipt::for_bottle(install_reason, time, runtime_dependencies, source)
        }
        InstallMethod::Source(_) => {
            InstallReceipt::for_source(install_reason, time, runtime_dependencies, source)
        }
    }
}

/// Copies extracted bottle content into a keg and patches binaries/text.
///
/// Shared by both the parallel install path and the serial upgrade path.
#[expect(
    clippy::too_many_arguments,
    reason = "thin delegation to materialize + relocate_keg_with_manifest"
)]
pub fn materialize_and_relocate_bottle(
    source_dir: &Path,
    keg_path: &Path,
    opt_dir: &Path,
    prefix: &Path,
    formula_name: &str,
    relocation_scope: RelocationScope,
) -> Result<(), BrewdockError> {
    let relocation_manifest = RelocationManifest::derive(source_dir)?;
    materialize(source_dir, keg_path, opt_dir, formula_name)?;
    relocate_keg_with_manifest(keg_path, prefix, relocation_scope, &relocation_manifest)?;
    Ok(())
}

/// Materializes a prefetched payload into a [`MaterializedPayload`].
///
/// This is a free function (not a method) so it can be moved into
/// [`tokio::task::spawn_blocking`] without borrowing the orchestrator.
pub fn materialize_prefetched_payload(
    payload: PrefetchedPayload,
    formula_name: &str,
    opt_dir: &Path,
    prefix: &Path,
) -> Result<MaterializedPayload, BrewdockError> {
    match payload {
        PrefetchedPayload::Bottle {
            source_dir,
            keg_path,
            relocation_scope,
        } => {
            materialize_and_relocate_bottle(
                &source_dir,
                &keg_path,
                opt_dir,
                prefix,
                formula_name,
                relocation_scope,
            )?;
            Ok(MaterializedPayload::Bottle { keg_path })
        }
        PrefetchedPayload::Source {
            source_root,
            plan,
            _tempdir: tempdir,
        } => Ok(MaterializedPayload::PendingSource(Box::new(
            PendingSourcePayload {
                source_root,
                plan,
                _tempdir: tempdir,
            },
        ))),
    }
}

pub fn refresh_opt_link(
    keg_path: &Path,
    opt_dir: &Path,
    formula_name: &str,
) -> Result<(), BrewdockError> {
    std::fs::create_dir_all(opt_dir)?;
    let opt_link = opt_dir.join(formula_name);
    atomic_symlink_replace(keg_path, &opt_link)?;
    Ok(())
}

pub fn cleanup_failed_install(
    keg_path: &Path,
    prefix: &Path,
    opt_dir: &Path,
    formula_name: &str,
) -> Result<(), BrewdockError> {
    let _ = unlink(keg_path, prefix);
    let opt_link = opt_dir.join(formula_name);
    if opt_link.symlink_metadata().is_ok() {
        std::fs::remove_file(&opt_link)?;
    }

    if keg_path.exists() {
        std::fs::remove_dir_all(keg_path)?;
    }

    // Clean up temp keg left by an interrupted atomic materialize.
    if let Some(parent) = keg_path.parent()
        && let Some(version) = keg_path.file_name().and_then(|name| name.to_str())
    {
        let temp_keg = parent.join(format!(".{version}.brewdock-tmp"));
        if temp_keg.exists() {
            std::fs::remove_dir_all(&temp_keg)?;
        }
    }

    Ok(())
}

pub fn build_receipt_deps(formula: &Formula, cache: &FormulaCache) -> Vec<ReceiptDependency> {
    formula
        .dependencies
        .iter()
        .filter_map(|dep_name| cache.get(dep_name))
        .map(|dep| ReceiptDependency {
            full_name: dep.full_name.clone(),
            version: dep.versions.stable.clone(),
            revision: dep.revision,
            pkg_version: pkg_version(&dep.versions.stable, dep.revision),
            declared_directly: true,
        })
        .collect()
}

pub fn build_receipt_source(formula: &Formula) -> ReceiptSource {
    let first_char = formula.name.chars().next().unwrap_or('_');
    ReceiptSource {
        path: format!("{FORMULA_PATH_PREFIX}/{first_char}/{}.rb", formula.name),
        tap: TAP_NAME.to_owned(),
        spec: "stable".to_owned(),
        versions: ReceiptSourceVersions {
            stable: formula.versions.stable.clone(),
            head: formula.versions.head.clone(),
            version_scheme: 0,
        },
    }
}
