use anyhow::{Context, Result};
use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RadegastVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub revision: Option<u64>, // The 'rN' part; None means base release
}

impl RadegastVersion {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim().trim_start_matches('v');
        if let Some((base, rev_str)) = s.split_once('r') {
            let parts: Vec<&str> = base.split('.').collect();
            anyhow::ensure!(parts.len() == 3, "Invalid version format: {s}");
            Ok(Self {
                major: parts[0].parse().context("invalid major")?,
                minor: parts[1].parse().context("invalid minor")?,
                patch: parts[2].parse().context("invalid patch")?,
                revision: Some(rev_str.parse().context("invalid revision")?),
            })
        } else {
            let parts: Vec<&str> = s.split('.').collect();
            anyhow::ensure!(parts.len() == 3, "Invalid version format: {s}");
            Ok(Self {
                major: parts[0].parse().context("invalid major")?,
                minor: parts[1].parse().context("invalid minor")?,
                patch: parts[2].parse().context("invalid patch")?,
                revision: None,
            })
        }
    }

    /// Convert back to display string (e.g., "1.7.0r1" or "1.7.0")
    pub fn to_manifest_string(&self) -> String {
        match self.revision {
            Some(r) => format!("{}.{}.{}r{}", self.major, self.minor, self.patch, r),
            None => format!("{}.{}.{}", self.major, self.minor, self.patch),
        }
    }
}

impl Ord for RadegastVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then(match (self.revision, other.revision) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Less, // 1.7.0 < 1.7.0r1
                (Some(_), None) => Ordering::Greater, // 1.7.0r1 > 1.7.0
                (Some(a), Some(b)) => a.cmp(&b),
            })
    }
}

impl PartialOrd for RadegastVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for RadegastVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_manifest_string())
    }
}

/// Parse version from standard command line output (e.g. "rustinel 1.7.1r1")
pub fn parse_version_output(stdout: &str) -> Result<RadegastVersion> {
    let ver_part = stdout
        .split_whitespace()
        .find(|s| {
            s.trim_start_matches('v')
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        })
        .context("Could not parse rustinel version from output")?;
    RadegastVersion::parse(ver_part)
}

/// Detect current installed rustinel version.
pub fn current_installed(rustinel_path: &str) -> Result<RadegastVersion> {
    let output = std::process::Command::new(rustinel_path)
        .arg("--version")
        .output()
        .context("Failed to execute rustinel --version")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_version_output(&stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        let v170 = RadegastVersion::parse("1.7.0").unwrap();
        let v170r1 = RadegastVersion::parse("1.7.0r1").unwrap();
        let v170r2 = RadegastVersion::parse("1.7.0r2").unwrap();
        let v171 = RadegastVersion::parse("1.7.1").unwrap();
        let v160 = RadegastVersion::parse("1.6.0").unwrap();
        let v160r1 = RadegastVersion::parse("1.6.0r1").unwrap();

        assert!(v170r1 > v170);
        assert!(v170r2 > v170r1);
        assert!(v171 > v170r2);
        assert!(v170 > v160r1);
        assert!(v160r1 > v160);
        assert!(v170r1 > v160);
        assert_eq!(v170r1, v170r1.clone());
        assert_eq!(v170, v170.clone());
    }

    #[test]
    fn parse_with_v_prefix() {
        let v = RadegastVersion::parse("v1.7.0r1").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 7);
        assert_eq!(v.patch, 0);
        assert_eq!(v.revision, Some(1));

        let v_base = RadegastVersion::parse("v2.3.4").unwrap();
        assert_eq!(v_base.major, 2);
        assert_eq!(v_base.minor, 3);
        assert_eq!(v_base.patch, 4);
        assert_eq!(v_base.revision, None);
    }

    #[test]
    fn display_roundtrip() {
        let v1 = RadegastVersion::parse("1.7.0r1").unwrap();
        assert_eq!(v1.to_manifest_string(), "1.7.0r1");
        assert_eq!(format!("{v1}"), "1.7.0r1");

        let v2 = RadegastVersion::parse("1.7.0").unwrap();
        assert_eq!(v2.to_manifest_string(), "1.7.0");
        assert_eq!(format!("{v2}"), "1.7.0");
    }

    #[test]
    fn parse_invalid_versions() {
        assert!(RadegastVersion::parse("").is_err());
        assert!(RadegastVersion::parse("invalid").is_err());
        assert!(RadegastVersion::parse("1").is_err());
        assert!(RadegastVersion::parse("1.2").is_err());
        assert!(RadegastVersion::parse("1.2.3.4").is_err());
        assert!(RadegastVersion::parse("1.2.3rx").is_err());
        assert!(RadegastVersion::parse("1.2.3r").is_err());
        assert!(RadegastVersion::parse("1.a.3").is_err());
    }

    #[test]
    fn test_parse_version_output() {
        let out1 = "rustinel 1.7.1r1";
        assert_eq!(
            parse_version_output(out1).unwrap().to_manifest_string(),
            "1.7.1r1"
        );

        let out2 = "Rustinel EDR Sensor v1.5.0 (built 2026-09-01)";
        assert_eq!(
            parse_version_output(out2).unwrap().to_manifest_string(),
            "1.5.0"
        );

        let out3 = "no numbers here";
        assert!(parse_version_output(out3).is_err());
    }
}
