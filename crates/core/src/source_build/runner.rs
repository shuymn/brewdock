use std::{ffi::OsStr, path::Path, process::Command};

use super::SourceBuildPlan;
use crate::{BrewdockError, error::SourceBuildError};

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
    let span = tracing::info_span!("bd.child_process", program, argv = args.join(" "),);
    let _entered = span.enter();
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

    let detail = if output.stderr.iter().any(|&b| !b.is_ascii_whitespace()) {
        String::from_utf8_lossy(&output.stderr).trim().to_owned()
    } else {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
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
