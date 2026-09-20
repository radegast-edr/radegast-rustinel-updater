use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize, Clone)]
pub struct ReleaseEntry {
    pub version: String,
    pub hash_sha256: String,
    pub sign_gpg: String,
}

pub fn fetch(url: &str) -> Result<Vec<ReleaseEntry>> {
    let client = Client::builder()
        .user_agent(concat!("radegast-rustinel-updater/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;
    let releases: Vec<ReleaseEntry> = client.get(url).send()?.error_for_status()?.json()
        .context("Failed to parse release manifest")?;
    Ok(releases)
}

pub fn find_latest_for_platform<'a>(
    releases: &'a [ReleaseEntry],
    platform: &crate::platform::Platform,
) -> Option<(crate::version::RadegastVersion, &'a ReleaseEntry)> {
    let archive_name = platform.archive_name();
    releases.iter()
        .filter_map(|entry| {
            // Only consider entries that have a checksum for our platform
            let has_platform = entry.hash_sha256.lines()
                .any(|line| line.trim().ends_with(&archive_name));
            if !has_platform { return None; }
            crate::version::RadegastVersion::parse(&entry.version)
                .ok()
                .map(|v| (v, entry))
        })
        .max_by(|(a, _), (b, _)| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_latest_for_platform() {
        let releases = vec![
            ReleaseEntry {
                version: "1.7.0r1".into(),
                hash_sha256: "hash1  linux-amd64.zip\nhash2  windows-amd64.zip\n".into(),
                sign_gpg: "sig1".into(),
            },
            ReleaseEntry {
                version: "1.7.0".into(),
                hash_sha256: "hash3  linux-amd64.zip\nhash4  mac-m5.zip\n".into(),
                sign_gpg: "sig2".into(),
            },
            ReleaseEntry {
                version: "1.6.0".into(),
                hash_sha256: "hash5  mac-m5.zip\n".into(),
                sign_gpg: "sig3".into(),
            },
        ];

        let linux = crate::platform::Platform {
            os: "linux",
            arch: "amd64",
        };
        let (ver, entry) = find_latest_for_platform(&releases, &linux).unwrap();
        assert_eq!(ver.to_manifest_string(), "1.7.0r1");
        assert_eq!(entry.sign_gpg, "sig1");

        let mac = crate::platform::Platform {
            os: "mac",
            arch: "m5",
        };
        let (ver, entry) = find_latest_for_platform(&releases, &mac).unwrap();
        assert_eq!(ver.to_manifest_string(), "1.7.0");
        assert_eq!(entry.sign_gpg, "sig2");
    }
}

