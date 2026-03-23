use std::path::{Path, PathBuf};

use brewdock_bottle::{BlobStore, BottleDownloader, extract_tar_gz};
use brewdock_cellar::{
    InstallReason, InstallReceipt, PostInstallContext, PostInstallTransaction, RelocationScope,
    install_bottle_etc_var, link, lower_post_install_tier2, run_post_install,
    validate_post_install, write_receipt,
};
use brewdock_formula::{
    CellarType, Formula, FormulaCache, FormulaError, FormulaName, FormulaRepository,
    UnsupportedReason,
};
use futures::stream::{self, StreamExt, TryStreamExt};

use super::{
    AcquiredFormula, AcquiredPayload, ExecutionPlan, ExecutionPlanEntry, FinalizeContext,
    FinalizeStep, InstallContext, Orchestrator, PendingSourcePayload, build_receipt,
    build_receipt_deps, build_receipt_source, pkg_version, unix_timestamp_f64,
};
use crate::{
    BrewdockError,
    error::SourceBuildError,
    finalize::{cleanup_failed_install, materialize_and_relocate_bottle, refresh_opt_link},
    source_build::{extract_source_archive, run_source_build, source_archive_filename},
};

impl<R: FormulaRepository, D: BottleDownloader> Orchestrator<R, D> {
    /// Runs the install step of an upgrade through the shared staged executor.
    pub(super) async fn run_upgrade_install(
        &self,
        candidate: &super::UpgradeCandidate,
        install_context: &InstallContext<'_, '_>,
    ) -> Result<(), BrewdockError> {
        let (to_install, cache) = if matches!(candidate.method, super::InstallMethod::Source(_)) {
            self.resolve_upgrade_install_list(candidate).await?
        } else {
            let mut cache = FormulaCache::new();
            cache.insert(candidate.formula.clone());
            (vec![candidate.name.clone()], cache)
        };

        self.execute_install_plan(&candidate.name, &to_install, &cache, install_context)
            .await
    }

    async fn acquire_payload(
        &self,
        operation: &'static str,
        entry: &ExecutionPlanEntry<'_>,
        blob_store: &BlobStore,
    ) -> Result<AcquiredPayload, BrewdockError> {
        let formula = entry.formula;
        match &entry.method {
            super::InstallMethod::Bottle(selected_bottle) => {
                let blob_hit =
                    self.instrument_phase(operation, "check-blob-store", &formula.name, || {
                        blob_store.has(&selected_bottle.sha256)
                    })?;

                if blob_hit {
                    tracing::info!(
                        name = formula.name,
                        sha256 = selected_bottle.sha256,
                        "blob store hit, skipping download"
                    );
                } else {
                    let data = self
                        .instrument_async_phase(
                            operation,
                            "download-bottle",
                            &formula.name,
                            self.downloader
                                .download_verified(&selected_bottle.url, &selected_bottle.sha256),
                        )
                        .await?;

                    self.instrument_phase(operation, "store-bottle-blob", &formula.name, || {
                        blob_store.put(&selected_bottle.sha256, &data)
                    })?;
                }

                let version_str = pkg_version(&formula.versions.stable, formula.revision);
                let extract_dir = self.layout.store_dir().join(&selected_bottle.sha256);
                let source_dir = extract_dir.join(&formula.name).join(&version_str);

                if source_dir.exists() {
                    tracing::info!(name = formula.name, "extract dir hit, skipping extraction");
                } else {
                    let blob_path = blob_store.blob_path(&selected_bottle.sha256)?;
                    self.instrument_phase(operation, "extract-bottle", &formula.name, || {
                        extract_tar_gz(&blob_path, &extract_dir)
                    })?;
                }

                let keg_path = self.layout.cellar().join(&formula.name).join(&version_str);
                let relocation_scope =
                    if matches!(selected_bottle.cellar, CellarType::AnySkipRelocation) {
                        RelocationScope::TextOnly
                    } else {
                        RelocationScope::Full
                    };

                let name = formula.name.clone();
                let opt_dir = self.layout.opt_dir();
                let prefix = self.layout.prefix().to_path_buf();
                let span = tracing::info_span!(
                    "bd.phase",
                    operation = operation,
                    phase = "materialize-payload",
                    target = name.as_str(),
                );
                tokio::task::spawn_blocking(move || {
                    let _entered = span.enter();
                    materialize_and_relocate_bottle(
                        &source_dir,
                        &keg_path,
                        &opt_dir,
                        &prefix,
                        &name,
                        relocation_scope,
                    )?;
                    Ok(AcquiredPayload::Bottle { keg_path })
                })
                .await
                .map_err(|err| BrewdockError::from(std::io::Error::other(err)))?
            }
            super::InstallMethod::Source(plan) => {
                let checksum = plan.source_checksum.as_deref().ok_or_else(|| {
                    SourceBuildError::MissingSourceChecksum(FormulaName::from(
                        plan.formula_name.clone(),
                    ))
                })?;
                let data = self
                    .instrument_async_phase(
                        operation,
                        "download-source-archive",
                        &plan.formula_name,
                        self.downloader
                            .download_verified(&plan.source_url, checksum),
                    )
                    .await?;

                let parent = self.layout.cache_dir().join("sources");
                let source_url = plan.source_url.clone();
                let span_persist = tracing::info_span!(
                    "bd.phase",
                    operation,
                    phase = "persist-source-archive",
                    target = plan.formula_name.as_str(),
                );
                let span_extract = tracing::info_span!(
                    "bd.phase",
                    operation,
                    phase = "extract-source-archive",
                    target = plan.formula_name.as_str(),
                );

                let (source_root, tempdir) = tokio::task::spawn_blocking(move || {
                    std::fs::create_dir_all(&parent)?;
                    let tempdir = tempfile::tempdir_in(parent)?;
                    let archive_path = tempdir
                        .path()
                        .join(source_archive_filename(&source_url).unwrap_or("source.tar.gz"));
                    {
                        let _entered = span_persist.enter();
                        std::fs::write(&archive_path, &data)?;
                    }
                    let source_root = {
                        let _entered = span_extract.enter();
                        extract_source_archive(&archive_path, tempdir.path())?
                    };
                    Ok::<_, BrewdockError>((source_root, tempdir))
                })
                .await
                .map_err(|err| BrewdockError::from(std::io::Error::other(err)))??;

                Ok(AcquiredPayload::PendingSource(Box::new(
                    PendingSourcePayload {
                        source_root,
                        plan: plan.clone(),
                        _tempdir: tempdir,
                    },
                )))
            }
        }
    }

    async fn finalize_acquired(
        &self,
        formula: &Formula,
        cache: &FormulaCache,
        install_context: &InstallContext<'_, '_>,
        resolved: AcquiredFormula,
    ) -> Result<(), BrewdockError> {
        let name = formula.name.as_str();

        self.emit_formula_started(install_context.operation, name);
        tracing::info!(name, "installing formula");

        let keg_path = match (resolved.finalize, resolved.acquired) {
            (FinalizeStep::FinalizeBottle, AcquiredPayload::Bottle { keg_path }) => keg_path,
            (FinalizeStep::BuildFromSource, AcquiredPayload::PendingSource(pending)) => {
                self.build_source_to_keg(install_context.operation, name, *pending)?
            }
            (FinalizeStep::FinalizeBottle, AcquiredPayload::PendingSource(_))
            | (FinalizeStep::BuildFromSource, AcquiredPayload::Bottle { .. }) => {
                self.emit_formula_failed(
                    install_context.operation,
                    name,
                    "execution plan finalize step does not match acquired payload",
                );
                return Err(BrewdockError::Io(std::io::Error::other(
                    "execution plan finalize step does not match acquired payload",
                )));
            }
        };

        let result = self
            .finalize_with_keg(
                formula,
                cache,
                install_context,
                FinalizeContext {
                    method: &resolved.method,
                    keg_path: &keg_path,
                },
            )
            .await;

        match &result {
            Ok(()) => self.emit_formula_completed(install_context.operation, name),
            Err(error) => {
                self.emit_formula_failed(install_context.operation, name, &error.to_string());
            }
        }

        result
    }

    fn build_source_to_keg(
        &self,
        operation: &'static str,
        name: &str,
        pending: PendingSourcePayload,
    ) -> Result<PathBuf, BrewdockError> {
        let PendingSourcePayload {
            source_root,
            plan,
            _tempdir: tempdir_guard,
        } = pending;
        self.instrument_phase(operation, "build-from-source", name, || {
            run_source_build(&source_root, &plan, self.layout.prefix())
        })
        .inspect_err(|_| {
            let _ = cleanup_failed_install(
                &plan.cellar_path,
                self.layout.prefix(),
                &self.layout.opt_dir(),
                name,
            );
        })?;
        drop(tempdir_guard);
        self.instrument_phase(operation, "refresh-opt-link", name, || {
            refresh_opt_link(
                &plan.cellar_path,
                &self.layout.opt_dir(),
                &plan.formula_name,
            )
        })?;
        Ok(plan.cellar_path)
    }

    async fn finalize_with_keg(
        &self,
        formula: &Formula,
        cache: &FormulaCache,
        install_context: &InstallContext<'_, '_>,
        finalize_ctx: FinalizeContext<'_>,
    ) -> Result<(), BrewdockError> {
        let name = formula.name.as_str();
        let keg_path = finalize_ctx.keg_path;
        let bottle_prefix_transaction = self
            .instrument_phase(
                install_context.operation,
                "install-bottle-prefix",
                name,
                || install_bottle_etc_var(keg_path, self.layout.prefix()),
            )
            .inspect_err(|_| {
                let _ = cleanup_failed_install(
                    keg_path,
                    self.layout.prefix(),
                    &self.layout.opt_dir(),
                    &formula.name,
                );
            })?;

        let post_install_transaction = match self
            .instrument_async_phase(
                install_context.operation,
                "post-install",
                name,
                self.execute_post_install(install_context.operation, formula, keg_path),
            )
            .await
        {
            Ok(transaction) => transaction,
            Err(error) => {
                if let Err(rollback_err) = bottle_prefix_transaction.rollback() {
                    tracing::warn!(
                        ?rollback_err,
                        "failed to rollback bottle prefix transaction"
                    );
                }
                cleanup_failed_install(
                    keg_path,
                    self.layout.prefix(),
                    &self.layout.opt_dir(),
                    &formula.name,
                )?;
                return Err(error);
            }
        };

        let is_requested = install_context.requested.contains(formula.name.as_str());
        let receipt = build_receipt(
            finalize_ctx.method,
            if is_requested {
                InstallReason::OnRequest
            } else {
                InstallReason::AsDependency
            },
            Some(unix_timestamp_f64()),
            build_receipt_deps(formula, cache),
            build_receipt_source(formula),
        );
        if let Err(error) =
            self.instrument_phase(install_context.operation, "finalize-install", name, || {
                self.finalize_installed_formula(formula, keg_path, &receipt)
            })
        {
            if let Some(transaction) = post_install_transaction {
                transaction.rollback()?;
            }
            bottle_prefix_transaction.rollback()?;
            cleanup_failed_install(
                keg_path,
                self.layout.prefix(),
                &self.layout.opt_dir(),
                &formula.name,
            )?;
            return Err(error);
        }

        if let Some(transaction) = post_install_transaction {
            transaction.commit()?;
        }
        bottle_prefix_transaction.commit();

        tracing::info!(name, "installation complete");
        Ok(())
    }

    pub(super) async fn check_post_install_viability(
        &self,
        formula: &Formula,
    ) -> Result<(), BrewdockError> {
        if let Some(source) = self.fetch_post_install_source(formula).await? {
            validate_post_install(&source, &formula.versions.stable)
                .or_else(|_| {
                    lower_post_install_tier2(&source, &formula.versions.stable).map(|_| ())
                })
                .map_err(brewdock_cellar::CellarError::from)?;
        }
        Ok(())
    }

    async fn execute_post_install(
        &self,
        operation: &'static str,
        formula: &Formula,
        keg_path: &Path,
    ) -> Result<Option<PostInstallTransaction>, BrewdockError> {
        let Some(ruby_source) = self
            .instrument_async_phase(
                operation,
                "fetch-post-install-source",
                &formula.name,
                self.fetch_post_install_source(formula),
            )
            .await?
        else {
            return Ok(None);
        };
        if !formula.keg_only {
            self.instrument_phase(operation, "pre-link-post-install", &formula.name, || {
                link(keg_path, self.layout.prefix())
            })?;
        }
        let transaction =
            self.instrument_phase(operation, "run-post-install", &formula.name, || {
                run_post_install(
                    &ruby_source,
                    &mut PostInstallContext::new(
                        self.layout.prefix(),
                        keg_path,
                        &formula.versions.stable,
                        &self.platform,
                    ),
                )
            })?;
        Ok(Some(transaction))
    }

    async fn fetch_post_install_source(
        &self,
        formula: &Formula,
    ) -> Result<Option<String>, BrewdockError> {
        if !formula.post_install_defined {
            return Ok(None);
        }
        let ruby_source_path =
            formula
                .ruby_source_path
                .as_deref()
                .ok_or_else(|| FormulaError::Unsupported {
                    name: FormulaName::from(formula.name.clone()),
                    reason: UnsupportedReason::PostInstallDefined,
                })?;
        let source = self.repo.ruby_source(ruby_source_path).await?;
        Ok(Some(source))
    }

    fn finalize_installed_formula(
        &self,
        formula: &Formula,
        keg_path: &Path,
        receipt: &InstallReceipt,
    ) -> Result<(), BrewdockError> {
        if !formula.keg_only {
            link(keg_path, self.layout.prefix())?;
        }
        write_receipt(keg_path, receipt)?;
        Ok(())
    }

    pub(super) async fn execute_install_plan(
        &self,
        label: &str,
        to_install: &[String],
        cache: &FormulaCache,
        install_context: &InstallContext<'_, '_>,
    ) -> Result<(), BrewdockError> {
        let operation = install_context.operation;
        let execution_plan = self.instrument_phase(operation, "plan-execution", label, || {
            self.build_execution_plan(to_install, cache)
        })?;

        let acquired = self
            .run_acquire_stage(operation, &execution_plan, install_context.blob_store)
            .await
            .inspect_err(|_| self.cleanup_materialized_kegs(&execution_plan))?;

        for (entry, acquired_payload) in execution_plan.entries.into_iter().zip(acquired) {
            self.finalize_acquired(
                entry.formula,
                cache,
                install_context,
                AcquiredFormula {
                    method: entry.method,
                    finalize: entry.finalize,
                    acquired: acquired_payload,
                },
            )
            .await?;
        }

        Ok(())
    }

    async fn run_acquire_stage(
        &self,
        operation: &'static str,
        execution_plan: &ExecutionPlan<'_>,
        blob_store: &BlobStore,
    ) -> Result<Vec<AcquiredPayload>, BrewdockError> {
        let limit = execution_plan.acquire_concurrency.max(1);
        let mut results = stream::iter(execution_plan.entries.iter().enumerate().map(
            |(index, entry)| async move {
                self.acquire_entry(operation, entry, blob_store)
                    .await
                    .map(|payload| (index, payload))
            },
        ))
        .buffer_unordered(limit)
        .try_collect::<Vec<_>>()
        .await?;
        results.sort_by_key(|(index, _)| *index);
        Ok(results.into_iter().map(|(_, payload)| payload).collect())
    }

    async fn acquire_entry(
        &self,
        operation: &'static str,
        entry: &ExecutionPlanEntry<'_>,
        blob_store: &BlobStore,
    ) -> Result<AcquiredPayload, BrewdockError> {
        self.instrument_async_phase(
            operation,
            "acquire-payload",
            &entry.formula.name,
            self.acquire_payload(operation, entry, blob_store),
        )
        .await
    }

    fn cleanup_materialized_kegs(&self, execution_plan: &ExecutionPlan<'_>) {
        for entry in &execution_plan.entries {
            let version = pkg_version(&entry.formula.versions.stable, entry.formula.revision);
            let keg = self
                .layout
                .cellar()
                .join(&entry.formula.name)
                .join(&version);
            if keg.exists() {
                let _ = cleanup_failed_install(
                    &keg,
                    self.layout.prefix(),
                    &self.layout.opt_dir(),
                    &entry.formula.name,
                );
            }
        }
    }
}
