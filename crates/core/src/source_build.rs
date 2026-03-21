use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

use brewdock_bottle::extract_tar_gz;
use brewdock_formula::{Formula, FormulaName, Requirement};

use crate::{
    BrewdockError, Layout,
    error::SourceBuildError,
    orchestrate::{SourceBuildPlan, pkg_version},
};

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
    if source_archive_kind(&stable.url).is_none() {
        return Err(SourceBuildError::UnsupportedSourceArchive(
            stable.url.clone(),
        ));
    }
    let version = pkg_version(&formula.versions.stable, formula.revision);
    Ok(SourceBuildPlan {
        formula_name: formula.name.clone(),
        version: version.clone(),
        source_url: stable.url.clone(),
        source_checksum: Some(source_checksum),
        build_dependencies: formula.build_dependencies.clone(),
        runtime_dependencies: formula.dependencies.clone(),
        prefix: layout.prefix().to_path_buf(),
        cellar_path: layout.cellar().join(&formula.name).join(version),
    })
}

fn requirement_name(requirement: &Requirement) -> &str {
    match requirement {
        Requirement::Name(name) => name,
        Requirement::Detailed(detail) => &detail.name,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceArchiveKind {
    TarGz,
}

pub fn source_archive_filename(url: &str) -> Option<&str> {
    let trimmed = url.split('?').next().unwrap_or(url);
    trimmed.rsplit('/').next()
}

fn source_archive_kind(url: &str) -> Option<SourceArchiveKind> {
    let filename = source_archive_filename(url)?.to_ascii_lowercase();
    if filename.ends_with(".tar.gz")
        || Path::new(&filename)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("tgz"))
    {
        Some(SourceArchiveKind::TarGz)
    } else {
        None
    }
}

pub fn extract_source_archive(
    archive_path: &Path,
    tempdir_root: &Path,
) -> Result<PathBuf, BrewdockError> {
    let kind = source_archive_kind(
        archive_path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default(),
    )
    .ok_or_else(|| {
        SourceBuildError::UnsupportedSourceArchive(archive_path.display().to_string())
    })?;

    let extract_dir = tempdir_root.join("extract");
    match kind {
        SourceArchiveKind::TarGz => extract_tar_gz(archive_path, &extract_dir)?,
    }
    discover_source_root(&extract_dir)
}

fn discover_source_root(extract_dir: &Path) -> Result<PathBuf, BrewdockError> {
    let entries: Vec<PathBuf> = std::fs::read_dir(extract_dir)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|entry| entry.path())
        .collect();

    if entries.len() == 1 && entries[0].is_dir() {
        Ok(entries[0].clone())
    } else if entries.is_empty() {
        Err(SourceBuildError::MissingSourceRoot(extract_dir.display().to_string()).into())
    } else {
        Ok(extract_dir.to_path_buf())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceBuildSystem {
    Configure,
    Cmake,
    Meson,
    PerlMakeMaker,
    Make,
}

pub fn run_source_build(
    source_root: &Path,
    plan: &SourceBuildPlan,
    prefix: &Path,
) -> Result<(), BrewdockError> {
    std::fs::create_dir_all(&plan.cellar_path)?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let joined_path = std::env::join_paths(
        std::iter::once(prefix.join("bin")).chain(std::env::split_paths(&path)),
    )
    .map_err(|error| std::io::Error::other(error.to_string()))?;
    let build_system = detect_build_system(source_root)?;
    let prefix_arg = format!("--prefix={}", plan.cellar_path.display());

    match build_system {
        SourceBuildSystem::Configure => {
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "./configure",
                &[&prefix_arg],
            )?;
            run_build_command(source_root, prefix, &joined_path, "make", &[])?;
            run_build_command(source_root, prefix, &joined_path, "make", &["install"])?;
        }
        SourceBuildSystem::Cmake => {
            let build_dir = source_root.join("build");
            let configure_args = cmake_configure_args(source_root, &build_dir, &plan.cellar_path);
            let configure_arg_refs = configure_args
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "cmake",
                &configure_arg_refs,
            )?;
            let build_arg = build_dir.display().to_string();
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "cmake",
                &["--build", &build_arg],
            )?;
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "cmake",
                &["--install", &build_arg],
            )?;
        }
        SourceBuildSystem::Meson => {
            let build_dir = source_root.join("build");
            let build_arg = build_dir.display().to_string();
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "meson",
                &["setup", &build_arg, &prefix_arg],
            )?;
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "ninja",
                &["-C", &build_arg],
            )?;
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "ninja",
                &["-C", &build_arg, "install"],
            )?;
        }
        SourceBuildSystem::PerlMakeMaker => {
            let install_base = format!("INSTALL_BASE={}", plan.cellar_path.display());
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "perl",
                &["Makefile.PL", &install_base],
            )?;
            run_build_command(source_root, prefix, &joined_path, "make", &[])?;
            run_build_command(source_root, prefix, &joined_path, "make", &["install"])?;
        }
        SourceBuildSystem::Make => {
            let prefix_value = plan.cellar_path.display().to_string();
            run_build_command(source_root, prefix, &joined_path, "make", &[])?;
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "make",
                &[
                    "install",
                    &format!("PREFIX={prefix_value}"),
                    &format!("prefix={prefix_value}"),
                ],
            )?;
        }
    }

    Ok(())
}

fn cmake_configure_args(source_root: &Path, build_dir: &Path, cellar_path: &Path) -> Vec<String> {
    vec![
        "-S".to_owned(),
        source_root.display().to_string(),
        "-B".to_owned(),
        build_dir.display().to_string(),
        format!("-DCMAKE_INSTALL_PREFIX={}", cellar_path.display()),
    ]
}

fn detect_build_system(source_root: &Path) -> Result<SourceBuildSystem, BrewdockError> {
    let candidates = [
        ("Makefile.PL", SourceBuildSystem::PerlMakeMaker),
        ("CMakeLists.txt", SourceBuildSystem::Cmake),
        ("meson.build", SourceBuildSystem::Meson),
        ("configure", SourceBuildSystem::Configure),
        ("Makefile", SourceBuildSystem::Make),
        ("makefile", SourceBuildSystem::Make),
    ];

    candidates
        .iter()
        .find(|(name, _)| source_root.join(name).exists())
        .map(|(_, build_system)| *build_system)
        .ok_or_else(|| {
            SourceBuildError::UnsupportedBuildSystem(source_root.display().to_string()).into()
        })
}

fn run_build_command(
    source_root: &Path,
    prefix: &Path,
    path: &OsStr,
    program: &str,
    args: &[&str],
) -> Result<(), BrewdockError> {
    let output = Command::new(program)
        .current_dir(source_root)
        .env("PATH", path)
        .env("HOMEBREW_PREFIX", prefix)
        .args(args)
        .output()
        .map_err(|error| SourceBuildError::CommandFailed {
            command: format_command(program, args),
            stderr: error.to_string(),
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    Err(SourceBuildError::CommandFailed {
        command: format_command(program, args),
        stderr: if detail.is_empty() {
            output.status.code().map_or_else(
                || "terminated by signal".to_owned(),
                |code| code.to_string(),
            )
        } else {
            detail
        },
    }
    .into())
}

fn format_command(program: &str, args: &[&str]) -> String {
    if args.is_empty() {
        program.to_owned()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::cmake_configure_args;

    #[test]
    fn test_cmake_configure_args_use_cmake_install_prefix() {
        let source_root = Path::new("/tmp/source");
        let build_dir = Path::new("/tmp/source/build");
        let cellar_path = Path::new("/opt/homebrew/Cellar/demo/1.0");

        let args = cmake_configure_args(source_root, build_dir, cellar_path);

        assert!(args.iter().any(|arg| arg == "-S"));
        assert!(args.iter().any(|arg| arg == "-B"));
        assert!(
            args.iter()
                .any(|arg| arg == "-DCMAKE_INSTALL_PREFIX=/opt/homebrew/Cellar/demo/1.0")
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg == "--prefix=/opt/homebrew/Cellar/demo/1.0")
        );
    }
}
