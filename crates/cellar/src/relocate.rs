use std::path::Path;

use crate::{error::CellarError, fs};

/// Placeholder strings embedded in Homebrew bottles.
const PREFIX_PLACEHOLDER: &str = "@@HOMEBREW_PREFIX@@";
const CELLAR_PLACEHOLDER: &str = "@@HOMEBREW_CELLAR@@";
const REPOSITORY_PLACEHOLDER: &str = "@@HOMEBREW_REPOSITORY@@";

/// Mach-O 64-bit magic (little-endian).
const MH_MAGIC_64: [u8; 4] = [0xCF, 0xFA, 0xED, 0xFE];

/// Fat binary magic (big-endian).
const FAT_MAGIC: [u8; 4] = [0xCA, 0xFE, 0xBA, 0xBE];

/// Controls which relocation steps [`relocate_keg`] performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationScope {
    /// Replace `@@HOMEBREW_*@@` text placeholders only; skip Mach-O binary
    /// relocation via `install_name_tool`.
    ///
    /// Used for bottles with `cellar: :any_skip_relocation`.
    TextOnly,
    /// Replace text placeholders **and** patch Mach-O load commands.
    Full,
}

/// Rewrites `@@HOMEBREW_*@@` placeholders in all files under `keg_path`.
///
/// - When `scope` is [`RelocationScope::Full`], Mach-O binaries are patched
///   with `install_name_tool` and text/data files undergo byte replacement.
/// - When `scope` is [`RelocationScope::TextOnly`], Mach-O binaries are
///   skipped and only text/data files are processed.
///
/// # Errors
///
/// Returns [`CellarError::Io`] on filesystem or subprocess failure.
pub fn relocate_keg(
    keg_path: &Path,
    prefix: &Path,
    scope: RelocationScope,
) -> Result<(), CellarError> {
    let cellar = prefix.join("Cellar");
    let prefix_str = path_str(prefix)?.to_owned();
    let cellar_str = path_str(&cellar)?.to_owned();

    let replacements = [
        (CELLAR_PLACEHOLDER, cellar_str.as_str()),
        (PREFIX_PLACEHOLDER, prefix_str.as_str()),
        (REPOSITORY_PLACEHOLDER, prefix_str.as_str()),
    ];

    for file in fs::walk_files(keg_path)? {
        let data = match std::fs::read(&file) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => continue,
            Err(e) => return Err(e.into()),
        };

        if !has_placeholder(&data) {
            continue;
        }

        if is_macho(&data) {
            if scope == RelocationScope::Full {
                relocate_macho(&file, &replacements)?;
            }
        } else {
            relocate_text_file(&file, &data, &replacements)?;
        }
    }
    Ok(())
}

/// Checks whether the data contains any `@@HOMEBREW_` marker.
fn has_placeholder(data: &[u8]) -> bool {
    data.windows(b"@@HOMEBREW_".len())
        .any(|w| w == b"@@HOMEBREW_")
}

/// Checks whether the first bytes indicate a Mach-O or fat binary.
fn is_macho(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    let magic: [u8; 4] = [data[0], data[1], data[2], data[3]];
    magic == MH_MAGIC_64 || magic == FAT_MAGIC
}

/// Relocates a Mach-O binary using a single `otool -l` call and a single
/// batched `install_name_tool` invocation.
fn relocate_macho(path: &Path, replacements: &[(&str, &str)]) -> Result<(), CellarError> {
    let path_s = path_str(path)?;
    fs::make_writable(path)?;

    // Parse all load commands in one pass.
    let otool_output = run_cmd("otool", &["-l", path_s])?;
    let load_cmds = parse_load_commands(&otool_output);

    let mut args: Vec<String> = Vec::new();

    // Patch LC_ID_DYLIB.
    if let Some(ref old_id) = load_cmds.id
        && let Some(new_id) = replace_placeholders(old_id, replacements)
    {
        args.push("-id".to_owned());
        args.push(new_id);
    }

    // Patch LC_LOAD_DYLIB entries.
    for old_path in &load_cmds.dylibs {
        if let Some(new_path) = replace_placeholders(old_path, replacements) {
            args.push("-change".to_owned());
            args.push(old_path.clone());
            args.push(new_path);
        }
    }

    // Patch LC_RPATH entries.
    for old_rpath in &load_cmds.rpaths {
        if let Some(new_rpath) = replace_placeholders(old_rpath, replacements) {
            args.push("-rpath".to_owned());
            args.push(old_rpath.clone());
            args.push(new_rpath);
        }
    }

    // Single batched install_name_tool call.
    if !args.is_empty() {
        args.push(path_s.to_owned());
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run_cmd("install_name_tool", &arg_refs)?;
    }

    // Re-sign with ad-hoc signature. install_name_tool on modern macOS does
    // this automatically, but be explicit. Log on failure since a broken
    // signature makes the binary unusable.
    if let Err(e) = run_cmd("codesign", &["--force", "--sign", "-", path_s]) {
        tracing::warn!(path = path_s, error = %e, "ad-hoc codesign failed");
    }

    Ok(())
}

/// Relocates a text/data file by byte replacement.
fn relocate_text_file(
    path: &Path,
    data: &[u8],
    replacements: &[(&str, &str)],
) -> Result<(), CellarError> {
    let mut content = data.to_vec();
    let mut changed = false;

    for &(placeholder, actual) in replacements {
        if replace_bytes(&mut content, placeholder.as_bytes(), actual.as_bytes()) {
            changed = true;
        }
    }

    if changed {
        fs::make_writable(path)?;
        std::fs::write(path, &content)?;
    }
    Ok(())
}

/// Replaces all occurrences of `from` with `to` in a byte buffer.
///
/// Returns `true` if any replacement was made.
fn replace_bytes(data: &mut Vec<u8>, from: &[u8], to: &[u8]) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + from.len() <= data.len() {
        if &data[i..i + from.len()] == from {
            data.splice(i..i + from.len(), to.iter().copied());
            i += to.len();
            changed = true;
        } else {
            i += 1;
        }
    }
    changed
}

/// Applies placeholder replacement to a string, returning `Some(new)` if changed.
fn replace_placeholders(s: &str, replacements: &[(&str, &str)]) -> Option<String> {
    let mut result = s.to_owned();
    let mut changed = false;
    for &(placeholder, actual) in replacements {
        if result.contains(placeholder) {
            result = result.replace(placeholder, actual);
            changed = true;
        }
    }
    if changed { Some(result) } else { None }
}

/// Parsed Mach-O load commands relevant for relocation.
struct MachOLoadCommands {
    /// `LC_ID_DYLIB` value (if present).
    id: Option<String>,
    /// `LC_LOAD_DYLIB` paths.
    dylibs: Vec<String>,
    /// `LC_RPATH` paths.
    rpaths: Vec<String>,
}

/// Parses `LC_ID_DYLIB`, `LC_LOAD_DYLIB`, and `LC_RPATH` from `otool -l` output.
fn parse_load_commands(output: &str) -> MachOLoadCommands {
    let mut id = None;
    let mut dylibs = Vec::new();
    let mut rpaths = Vec::new();
    let mut lines = output.lines();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        let cmd = match trimmed.strip_prefix("cmd ") {
            Some(c) => c.trim(),
            None => continue,
        };

        match cmd {
            "LC_ID_DYLIB" | "LC_LOAD_DYLIB" | "LC_REEXPORT_DYLIB" => {
                if let Some(name) = skip_cmdsize_and_extract(&mut lines, "name ") {
                    if cmd == "LC_ID_DYLIB" {
                        id = Some(name);
                    } else {
                        dylibs.push(name);
                    }
                }
            }
            "LC_RPATH" => {
                if let Some(path) = skip_cmdsize_and_extract(&mut lines, "path ") {
                    rpaths.push(path);
                }
            }
            _ => {}
        }
    }

    MachOLoadCommands { id, dylibs, rpaths }
}

/// Skips the cmdsize line, reads the next line, and extracts the value after
/// `prefix` (stripping the trailing `(offset ...)` annotation).
fn skip_cmdsize_and_extract<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    prefix: &str,
) -> Option<String> {
    lines.next(); // cmdsize
    let value_line = lines.next()?;
    let rest = value_line.trim().strip_prefix(prefix)?;
    let value = rest.split(" (offset").next().unwrap_or("").trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

/// Runs a command and returns its stdout as a string.
fn run_cmd(program: &str, args: &[&str]) -> Result<String, CellarError> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| {
            CellarError::Io(std::io::Error::other(format!(
                "{program} failed to start: {e}"
            )))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CellarError::Io(std::io::Error::other(format!(
            "{program} failed: {stderr}"
        ))));
    }

    String::from_utf8(output.stdout).map_err(|e| {
        CellarError::Io(std::io::Error::other(format!(
            "{program} produced invalid UTF-8: {e}"
        )))
    })
}

fn path_str(path: &Path) -> Result<&str, CellarError> {
    path.to_str().ok_or_else(|| {
        CellarError::Io(std::io::Error::other(format!(
            "non-UTF-8 path: {}",
            path.display()
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_placeholder_present() {
        let data = b"path=@@HOMEBREW_PREFIX@@/bin/tool";
        assert!(has_placeholder(data));
    }

    #[test]
    fn test_has_placeholder_absent() {
        let data = b"path=/opt/homebrew/bin/tool";
        assert!(!has_placeholder(data));
    }

    #[test]
    fn test_is_macho_64() {
        let data = [0xCF, 0xFA, 0xED, 0xFE, 0x00];
        assert!(is_macho(&data));
    }

    #[test]
    fn test_is_macho_fat() {
        let data = [0xCA, 0xFE, 0xBA, 0xBE, 0x00];
        assert!(is_macho(&data));
    }

    #[test]
    fn test_is_macho_not() {
        let data = b"#!/bin/sh\necho hello";
        assert!(!is_macho(data));
    }

    #[test]
    fn test_is_macho_too_short() {
        let data = [0xCF, 0xFA];
        assert!(!is_macho(&data));
    }

    #[test]
    fn test_replace_placeholders_prefix() {
        let replacements = [
            ("@@HOMEBREW_CELLAR@@", "/opt/homebrew/Cellar"),
            ("@@HOMEBREW_PREFIX@@", "/opt/homebrew"),
        ];
        let result = replace_placeholders(
            "@@HOMEBREW_PREFIX@@/opt/oniguruma/lib/libonig.5.dylib",
            &replacements,
        );
        assert_eq!(
            result,
            Some("/opt/homebrew/opt/oniguruma/lib/libonig.5.dylib".to_owned())
        );
    }

    #[test]
    fn test_replace_placeholders_cellar() {
        let replacements = [
            ("@@HOMEBREW_CELLAR@@", "/opt/homebrew/Cellar"),
            ("@@HOMEBREW_PREFIX@@", "/opt/homebrew"),
        ];
        let result = replace_placeholders(
            "@@HOMEBREW_CELLAR@@/jq/1.8.1/lib/libjq.1.dylib",
            &replacements,
        );
        assert_eq!(
            result,
            Some("/opt/homebrew/Cellar/jq/1.8.1/lib/libjq.1.dylib".to_owned())
        );
    }

    #[test]
    fn test_replace_placeholders_no_change() {
        let replacements = [
            ("@@HOMEBREW_CELLAR@@", "/opt/homebrew/Cellar"),
            ("@@HOMEBREW_PREFIX@@", "/opt/homebrew"),
        ];
        assert!(replace_placeholders("/usr/lib/libSystem.B.dylib", &replacements).is_none());
    }

    #[test]
    fn test_replace_bytes_basic() {
        let mut data = b"@@HOMEBREW_PREFIX@@/bin/tool".to_vec();
        let changed = replace_bytes(&mut data, b"@@HOMEBREW_PREFIX@@", b"/opt/homebrew");
        assert!(changed);
        assert_eq!(data, b"/opt/homebrew/bin/tool");
    }

    #[test]
    fn test_replace_bytes_no_match() {
        let mut data = b"/opt/homebrew/bin/tool".to_vec();
        let changed = replace_bytes(&mut data, b"@@HOMEBREW_PREFIX@@", b"/opt/homebrew");
        assert!(!changed);
        assert_eq!(data, b"/opt/homebrew/bin/tool");
    }

    #[test]
    fn test_replace_bytes_multiple() {
        let mut data = b"@@HOMEBREW_PREFIX@@/a:@@HOMEBREW_PREFIX@@/b".to_vec();
        let changed = replace_bytes(&mut data, b"@@HOMEBREW_PREFIX@@", b"/opt/homebrew");
        assert!(changed);
        assert_eq!(data, b"/opt/homebrew/a:/opt/homebrew/b");
    }

    #[test]
    fn test_parse_load_commands_dylibs_and_rpaths() {
        let output = "\
Load command 10
          cmd LC_ID_DYLIB
      cmdsize 64
         name @@HOMEBREW_CELLAR@@/jq/1.8.1/lib/libjq.1.dylib (offset 24)
Load command 11
          cmd LC_LOAD_DYLIB
      cmdsize 56
         name /usr/lib/libSystem.B.dylib (offset 24)
Load command 12
          cmd LC_LOAD_DYLIB
      cmdsize 80
         name @@HOMEBREW_PREFIX@@/opt/oniguruma/lib/libonig.5.dylib (offset 24)
Load command 15
          cmd LC_RPATH
      cmdsize 48
         path @@HOMEBREW_PREFIX@@/lib (offset 12)
Load command 16
          cmd LC_SEGMENT_64
      cmdsize 64
";
        let cmds = parse_load_commands(output);
        assert_eq!(
            cmds.id.as_deref(),
            Some("@@HOMEBREW_CELLAR@@/jq/1.8.1/lib/libjq.1.dylib")
        );
        assert_eq!(cmds.dylibs.len(), 2);
        assert_eq!(cmds.dylibs[0], "/usr/lib/libSystem.B.dylib");
        assert_eq!(
            cmds.dylibs[1],
            "@@HOMEBREW_PREFIX@@/opt/oniguruma/lib/libonig.5.dylib"
        );
        assert_eq!(cmds.rpaths, vec!["@@HOMEBREW_PREFIX@@/lib"]);
    }

    #[test]
    fn test_parse_load_commands_empty() {
        let cmds = parse_load_commands("Load command 0\n          cmd LC_SEGMENT_64\n");
        assert!(cmds.id.is_none());
        assert!(cmds.dylibs.is_empty());
        assert!(cmds.rpaths.is_empty());
    }

    #[test]
    fn test_relocate_text_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.pc");
        std::fs::write(
            &path,
            "prefix=@@HOMEBREW_PREFIX@@\nlibdir=@@HOMEBREW_CELLAR@@/pkg/1.0/lib\n",
        )?;

        let data = std::fs::read(&path)?;
        let replacements = [
            ("@@HOMEBREW_CELLAR@@", "/opt/homebrew/Cellar"),
            ("@@HOMEBREW_PREFIX@@", "/opt/homebrew"),
            ("@@HOMEBREW_REPOSITORY@@", "/opt/homebrew"),
        ];
        relocate_text_file(&path, &data, &replacements)?;

        let result = std::fs::read_to_string(&path)?;
        assert_eq!(
            result,
            "prefix=/opt/homebrew\nlibdir=/opt/homebrew/Cellar/pkg/1.0/lib\n"
        );
        Ok(())
    }

    #[test]
    fn test_relocate_text_file_readonly() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir()?;
        let path = dir.path().join("readonly.pc");
        std::fs::write(&path, "prefix=@@HOMEBREW_PREFIX@@\n")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444))?;

        let data = std::fs::read(&path)?;
        let replacements = [
            ("@@HOMEBREW_CELLAR@@", "/opt/homebrew/Cellar"),
            ("@@HOMEBREW_PREFIX@@", "/opt/homebrew"),
            ("@@HOMEBREW_REPOSITORY@@", "/opt/homebrew"),
        ];
        relocate_text_file(&path, &data, &replacements)?;

        let result = std::fs::read_to_string(&path)?;
        assert_eq!(result, "prefix=/opt/homebrew\n");
        Ok(())
    }

    #[test]
    fn test_relocate_keg_skips_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");
        std::fs::create_dir_all(keg_path.join("lib"))?;

        // Create a regular file with a placeholder.
        std::fs::write(
            keg_path.join("lib/config.pc"),
            "prefix=@@HOMEBREW_PREFIX@@\n",
        )?;
        // Create a symlink. It should be untouched by relocation.
        std::os::unix::fs::symlink("config.pc", keg_path.join("lib/config-link.pc"))?;

        relocate_keg(&keg_path, &prefix, RelocationScope::Full)?;

        // The regular file should be relocated.
        let relocated = std::fs::read_to_string(keg_path.join("lib/config.pc"))?;
        assert_eq!(relocated, format!("prefix={}\n", prefix.display()));

        // The symlink should still be a symlink with original target.
        let link = keg_path.join("lib/config-link.pc");
        assert!(link.is_symlink(), "symlink should still be a symlink");
        assert_eq!(
            std::fs::read_link(&link)?,
            std::path::PathBuf::from("config.pc")
        );
        Ok(())
    }

    #[test]
    fn test_relocate_keg_text_only_replaces_text_but_skips_macho()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");
        std::fs::create_dir_all(keg_path.join("bin"))?;
        std::fs::create_dir_all(keg_path.join("lib"))?;

        // Text file with placeholder — should be relocated.
        std::fs::write(
            keg_path.join("bin/tool"),
            "#!@@HOMEBREW_PREFIX@@/bin/python3\n",
        )?;

        // Fake Mach-O file (magic bytes + placeholder in "load command").
        let mut macho_data = MH_MAGIC_64.to_vec();
        macho_data.extend_from_slice(b"@@HOMEBREW_PREFIX@@/lib/libfoo.dylib");
        std::fs::write(keg_path.join("lib/libfoo.dylib"), &macho_data)?;

        relocate_keg(&keg_path, &prefix, RelocationScope::TextOnly)?;

        // Text file should have placeholders replaced.
        let text = std::fs::read_to_string(keg_path.join("bin/tool"))?;
        assert_eq!(text, format!("#!{}/bin/python3\n", prefix.display()));

        // Mach-O file should be untouched (no install_name_tool called).
        let macho = std::fs::read(keg_path.join("lib/libfoo.dylib"))?;
        assert_eq!(
            macho, macho_data,
            "Mach-O file must not be modified in TextOnly mode"
        );

        Ok(())
    }
}
