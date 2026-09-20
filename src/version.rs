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
        self.major.cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then(match (self.revision, other.revision) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Less,    // 1.7.0 < 1.7.0r1
                (Some(_), None) => Ordering::Greater,  // 1.7.0r1 > 1.7.0
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

/// Detect current installed rustinel version.
pub fn current_installed(rustinel_path: &str) -> Result<RadegastVersion> {
    let output = std::process::Command::new(rustinel_path)
        .arg("--version")
        .output()
        .context("Failed to execute rustinel --version")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let ver_part = stdout.split_whitespace()
        .find(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .context("Could not parse rustinel version from output")?;
    RadegastVersion::parse(ver_part)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_ordering() {
        let v170 = RadegastVersion::parse("1.7.0").unwrap();
        let v170r1 = RadegastVersion::parse("1.7.0r1").unwrap();
        let v160 = RadegastVersion::parse("1.6.0").unwrap();
        let v160r1 = RadegastVersion::parse("1.6.0r1").unwrap();
        assert!(v170r1 > v170);
        assert!(v170 > v160r1);
        assert!(v160r1 > v160);
        assert!(v170r1 > v160);
    }
    #[test]
    fn parse_with_v_prefix() {
        let v = RadegastVersion::parse("v1.7.0r1").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 7);
        assert_eq!(v.patch, 0);
        assert_eq!(v.revision, Some(1));
    }
    #[test]
    fn display_roundtrip() {
        assert_eq!(RadegastVersion::parse("1.7.0r1").unwrap().to_manifest_string(), "1.7.0r1");
        assert_eq!(RadegastVersion::parse("1.7.0").unwrap().to_manifest_string(), "1.7.0");
    }
}
