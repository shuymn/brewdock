use std::{fmt, str::FromStr};

use brewdock_cellar::{PlatformContext, canonical_homebrew_arch};
use strum::{Display, EnumString};

/// A platform identifier for Homebrew bottles (e.g., `arm64_sequoia`).
///
/// Format: `{arch}_{codename}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostTag(String);

impl HostTag {
    /// Returns the tag as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the Homebrew arch portion of the host tag.
    #[must_use]
    pub fn arch(&self) -> &str {
        self.0.split_once('_').map_or("", |(arch, _)| arch)
    }

    /// Returns the macOS codename portion of the host tag.
    #[must_use]
    pub fn macos_codename(&self) -> Option<&str> {
        self.0.split_once('_').map(|(_, codename)| codename)
    }

    /// Returns the macOS major version implied by the host tag codename.
    #[must_use]
    pub fn macos_major(&self) -> Option<u16> {
        self.macos_codename().and_then(macos_major_from_codename)
    }

    /// Detects the host tag for the current macOS system.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::Detection`] if system commands fail,
    /// or [`PlatformError::Unsupported`] if the architecture or OS version
    /// is not supported.
    #[cfg(target_os = "macos")]
    pub fn detect() -> Result<Self, PlatformError> {
        use std::process::Command;

        let version_output = Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .map_err(|e| PlatformError::Detection(format!("sw_vers failed: {e}")))?;
        if !version_output.status.success() {
            return Err(PlatformError::Detection(format!(
                "sw_vers exited with {}",
                version_output.status
            )));
        }

        let version_str = String::from_utf8_lossy(&version_output.stdout);
        let version: OsVersion = version_str.trim().parse()?;

        let arch_output = Command::new("uname")
            .arg("-m")
            .output()
            .map_err(|e| PlatformError::Detection(format!("uname failed: {e}")))?;
        if !arch_output.status.success() {
            return Err(PlatformError::Detection(format!(
                "uname exited with {}",
                arch_output.status
            )));
        }

        let arch_str = String::from_utf8_lossy(&arch_output.stdout);
        let arch: Arch = arch_str
            .trim()
            .parse()
            .map_err(|_parse_error| PlatformError::Unsupported)?;

        let codename = macos_codename(version.major)?;
        format!("{arch}_{codename}").parse()
    }

    /// Returns [`PlatformError::Unsupported`] on non-macOS platforms.
    ///
    /// # Errors
    ///
    /// Always returns [`PlatformError::Unsupported`].
    #[cfg(not(target_os = "macos"))]
    pub const fn detect() -> Result<Self, PlatformError> {
        Err(PlatformError::Unsupported)
    }
}

#[must_use]
pub(crate) fn detect_platform_context() -> PlatformContext {
    PlatformContext {
        kernel_version_major: detect_kernel_version_major(),
        macos_version: detect_macos_version(),
        cpu_arch: detect_cpu_arch(),
    }
}

impl FromStr for HostTag {
    type Err = PlatformError;

    /// Parses a host tag string.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::InvalidHostTag`] if the string is empty
    /// or does not contain an underscore.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || !s.contains('_') {
            return Err(PlatformError::InvalidHostTag(s.to_owned()));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for HostTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// CPU architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString)]
pub enum Arch {
    /// 64-bit ARM (Apple Silicon).
    #[strum(to_string = "arm64", serialize = "aarch64")]
    Arm64,
}

/// macOS version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OsVersion {
    /// Major version (e.g., 15 for Sequoia).
    pub major: u16,
    /// Minor version.
    pub minor: u16,
    /// Patch version.
    pub patch: u16,
}

impl OsVersion {
    /// Creates a new `OsVersion`.
    #[must_use]
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl FromStr for OsVersion {
    type Err = PlatformError;

    /// Parses a version string (e.g., `15.0.1` or `15.0`).
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::Detection`] if the format is invalid.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return Err(PlatformError::Detection(format!(
                "invalid version format: {s}"
            )));
        }
        let major = parts[0]
            .parse()
            .map_err(|e| PlatformError::Detection(format!("invalid major version: {e}")))?;
        let minor = parts[1]
            .parse()
            .map_err(|e| PlatformError::Detection(format!("invalid minor version: {e}")))?;
        let patch = if parts.len() == 3 {
            parts[2]
                .parse()
                .map_err(|e| PlatformError::Detection(format!("invalid patch version: {e}")))?
        } else {
            0
        };
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for OsVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Platform-related errors.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// The host tag string is invalid.
    #[error("invalid host tag: {0}")]
    InvalidHostTag(String),

    /// The platform is not supported by brewdock.
    #[error("unsupported platform")]
    Unsupported,

    /// Platform detection failed.
    #[error("detection failed: {0}")]
    Detection(String),
}

const MACOS_VERSIONS: &[(u16, &str)] = &[
    (26, "tahoe"),
    (15, "sequoia"),
    (14, "sonoma"),
    (13, "ventura"),
    (12, "monterey"),
    (11, "big_sur"),
];

#[cfg(target_os = "macos")]
fn macos_codename(major: u16) -> Result<&'static str, PlatformError> {
    MACOS_VERSIONS
        .iter()
        .find(|(m, _)| *m == major)
        .map(|(_, name)| *name)
        .ok_or(PlatformError::Unsupported)
}

fn macos_major_from_codename(codename: &str) -> Option<u16> {
    MACOS_VERSIONS
        .iter()
        .find(|(_, name)| *name == codename)
        .map(|(major, _)| *major)
}

fn detect_kernel_version_major() -> String {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            let version = String::from_utf8_lossy(&output.stdout);
            version.trim().split('.').next().map(ToOwned::to_owned)
        })
        .unwrap_or_default()
}

fn detect_macos_version() -> String {
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn detect_cpu_arch() -> String {
    canonical_homebrew_arch(std::env::consts::ARCH).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_tag_parse_valid() -> Result<(), PlatformError> {
        let tag: HostTag = "arm64_sequoia".parse()?;
        assert_eq!(tag.as_str(), "arm64_sequoia");
        Ok(())
    }

    #[test]
    fn test_host_tag_parse_empty() {
        let result: Result<HostTag, _> = "".parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_host_tag_round_trip_cases() -> Result<(), PlatformError> {
        for original in ["arm64_sequoia", "arm64_sonoma", "arm64_tahoe"] {
            let tag: HostTag = original.parse()?;
            assert_eq!(tag.as_str(), original, "as_str mismatch for {original}");
            assert_eq!(
                tag.to_string(),
                original,
                "to_string mismatch for {original}"
            );
        }
        Ok(())
    }

    #[test]
    fn test_host_tag_accessors() -> Result<(), PlatformError> {
        let tag: HostTag = "arm64_sequoia".parse()?;
        assert_eq!(tag.arch(), "arm64");
        assert_eq!(tag.macos_codename(), Some("sequoia"));
        assert_eq!(tag.macos_major(), Some(15));
        Ok(())
    }

    #[test]
    fn test_arch_arm64() -> Result<(), Box<dyn std::error::Error>> {
        let arch: Arch = "arm64".parse()?;
        assert_eq!(arch, Arch::Arm64);
        assert_eq!(arch.to_string(), "arm64");
        Ok(())
    }

    #[test]
    fn test_arch_aarch64_alias() -> Result<(), Box<dyn std::error::Error>> {
        let arch: Arch = "aarch64".parse()?;
        assert_eq!(arch, Arch::Arm64);
        Ok(())
    }

    #[test]
    fn test_arch_unsupported() {
        let result: Result<Arch, _> = "x86_64".parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_os_version_parse_three_parts() -> Result<(), PlatformError> {
        let ver: OsVersion = "15.0.1".parse()?;
        assert_eq!(ver, OsVersion::new(15, 0, 1));
        Ok(())
    }

    #[test]
    fn test_os_version_parse_two_parts() -> Result<(), PlatformError> {
        let ver: OsVersion = "15.0".parse()?;
        assert_eq!(ver, OsVersion::new(15, 0, 0));
        Ok(())
    }

    #[test]
    fn test_os_version_display() {
        let ver = OsVersion::new(15, 0, 1);
        assert_eq!(ver.to_string(), "15.0.1");
    }

    #[test]
    fn test_os_version_parse_invalid_inputs() {
        for input in ["15", "15.0.1.2", "abc.0.1", "", "15.0."] {
            assert!(
                input.parse::<OsVersion>().is_err(),
                "expected invalid os version: {input:?}"
            );
        }
    }
}
