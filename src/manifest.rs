use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Deserialize, Clone)]
pub struct ReleaseEntry {
    pub version: String,
    pub hash_sha256: String,
    pub sign_gpg: String,
    #[serde(default = "default_autoupdate")]
    pub autoupdate: bool,
}

fn default_autoupdate() -> bool {
    true
}

/// Any release newer than the installed version can veto automatic updates,
/// even if it has no archive for this platform or a later release allows updates.
/// With an unknown installed version, a veto cannot safely be ruled out.
pub fn automatic_update_blocker<'a>(
    releases: &'a [ReleaseEntry],
    current: Option<&crate::version::RadegastVersion>,
) -> Result<Option<&'a ReleaseEntry>> {
    for entry in releases.iter().filter(|entry| !entry.autoupdate) {
        let version = crate::version::RadegastVersion::parse(&entry.version)
            .context("Invalid version on release with autoupdate=false")?;
        if current.is_none_or(|current| version > *current) {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

pub fn fetch(url: &str) -> Result<Vec<ReleaseEntry>> {
    let client = crate::http::client(url, Duration::from_secs(60))?;
    let response = client.get(url).send()?.error_for_status()?;
    let mut bytes = Vec::new();
    crate::http::copy_limited(response, &mut bytes, 8 * 1024 * 1024)?;
    let releases = serde_json::from_slice(&bytes).context("Failed to parse release manifest")?;
    Ok(releases)
}

/// Repeatedly attempts to fetch the release manifest every `retry_interval` until it succeeds or reaches `max_retries`.
/// Useful during initial boot when the network/Wi-Fi connection may not be established yet.
pub fn fetch_with_retry_limit(
    url: &str,
    retry_interval: Duration,
    max_retries: Option<usize>,
) -> Result<Vec<ReleaseEntry>> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        info!("Fetching manifest from {} (attempt {})...", url, attempt);
        match fetch(url) {
            Ok(releases) => {
                info!(
                    "Successfully fetched release manifest on attempt {}.",
                    attempt
                );
                return Ok(releases);
            }
            Err(e) => {
                if let Some(max) = max_retries {
                    if attempt >= max {
                        return Err(e);
                    }
                }
                warn!(
                    "Failed to fetch release manifest (e.g. Wi-Fi not connected yet): {}. Retrying in {} seconds...",
                    e,
                    retry_interval.as_secs()
                );
                std::thread::sleep(retry_interval);
            }
        }
    }
}

/// Repeatedly attempts to fetch the release manifest every `retry_interval` until it succeeds for the first time.
pub fn fetch_until_success(url: &str, retry_interval: Duration) -> Vec<ReleaseEntry> {
    fetch_with_retry_limit(url, retry_interval, None)
        .expect("fetch_with_retry_limit without retry limit never returns Err")
}

pub fn find_latest_for_platform<'a>(
    releases: &'a [ReleaseEntry],
    platform: &crate::platform::Platform,
) -> Option<(crate::version::RadegastVersion, &'a ReleaseEntry)> {
    let archive_name = platform.archive_name();
    releases
        .iter()
        .filter_map(|entry| {
            // Only consider entries that have a checksum for our platform
            let has_platform = entry
                .hash_sha256
                .lines()
                .any(|line| line.split_whitespace().nth(1) == Some(archive_name.as_str()));
            if !has_platform {
                return None;
            }
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
    fn test_deserialize_manifest_json() {
        let json_data = r#"[
            {
                "version": "1.7.0r1",
                "hash_sha256": "abc123hash  linux-amd64.zip\n",
                "sign_gpg": "-----BEGIN PGP SIGNATURE-----\ntest\n-----END PGP SIGNATURE-----"
            }
        ]"#;

        let releases: Vec<ReleaseEntry> = serde_json::from_str(json_data).unwrap();
        assert_eq!(releases.len(), 1);
        assert!(releases[0].autoupdate);
        assert_eq!(releases[0].version, "1.7.0r1");
        assert!(releases[0].hash_sha256.contains("linux-amd64.zip"));
        assert!(releases[0].sign_gpg.contains("BEGIN PGP SIGNATURE"));
    }

    fn release(version: &str, autoupdate: bool) -> ReleaseEntry {
        ReleaseEntry {
            version: version.into(),
            hash_sha256: "hash  windows-amd64.zip\n".into(),
            sign_gpg: "sig".into(),
            autoupdate,
        }
    }

    #[test]
    fn test_deserialize_autoupdate_boolean() {
        for allowed in [false, true] {
            let data = serde_json::json!({
                "version": "1.8.0",
                "hash_sha256": "hash  linux-amd64.zip",
                "sign_gpg": "sig",
                "autoupdate": allowed,
            });
            let entry: ReleaseEntry = serde_json::from_value(data).unwrap();
            assert_eq!(entry.autoupdate, allowed);
        }
        for invalid in [serde_json::json!("false"), serde_json::Value::Null] {
            let data = serde_json::json!({
                "version": "1.8.0", "hash_sha256": "hash", "sign_gpg": "sig",
                "autoupdate": invalid,
            });
            assert!(serde_json::from_value::<ReleaseEntry>(data).is_err());
        }
    }

    #[test]
    fn test_intermediate_release_blocks_later_allowed_release() {
        // Deliberately unsorted, mixing explicit true, omitted, and false.
        // The veto has no Linux archive.
        let mut releases: Vec<ReleaseEntry> = serde_json::from_str(r#"[
            {"version":"1.9.0","hash_sha256":"hash  linux-amd64.zip", "sign_gpg":"sig", "autoupdate":true},
            {"version":"1.7.0","hash_sha256":"hash  linux-amd64.zip", "sign_gpg":"sig"},
            {"version":"v1.8.0r1","hash_sha256":"hash  windows-amd64.zip", "sign_gpg":"sig", "autoupdate":false}
        ]"#).unwrap();
        assert!(releases[0].autoupdate);
        assert!(releases[1].autoupdate);
        assert!(!releases[2].autoupdate);
        let current = crate::version::RadegastVersion::parse("1.8.0").unwrap();
        let blocker = automatic_update_blocker(&releases, Some(&current))
            .unwrap()
            .unwrap();
        assert_eq!(blocker.version, "v1.8.0r1");
        releases.reverse();
        assert!(automatic_update_blocker(&releases, Some(&current))
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_omitted_and_true_autoupdate_allow_newer_releases() {
        let releases: Vec<ReleaseEntry> = serde_json::from_str(r#"[
            {"version":"1.8.0","hash_sha256":"hash  linux-amd64.zip", "sign_gpg":"sig"},
            {"version":"1.9.0","hash_sha256":"hash  linux-amd64.zip", "sign_gpg":"sig", "autoupdate":true}
        ]"#).unwrap();
        let current = crate::version::RadegastVersion::parse("1.7.0").unwrap();
        assert!(automatic_update_blocker(&releases, Some(&current))
            .unwrap()
            .is_none());
        let linux = crate::platform::Platform {
            os: "linux",
            arch: "amd64",
        };
        let (latest, _) = find_latest_for_platform(&releases, &linux).unwrap();
        assert_eq!(latest.to_manifest_string(), "1.9.0");
    }

    #[test]
    fn test_blocker_at_or_below_current_allows_updates() {
        let releases = vec![release("1.8.0r1", false), release("1.9.0", true)];
        for version in ["1.8.0r1", "1.8.0r2", "1.9.0"] {
            let current = crate::version::RadegastVersion::parse(version).unwrap();
            assert!(automatic_update_blocker(&releases, Some(&current))
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn test_unknown_current_version_with_blocker_is_blocked() {
        assert!(automatic_update_blocker(&[release("1.8.0", false)], None)
            .unwrap()
            .is_some());
        assert!(automatic_update_blocker(&[release("1.8.0", true)], None)
            .unwrap()
            .is_none());
        assert!(automatic_update_blocker(&[], None).unwrap().is_none());
    }

    #[test]
    fn test_invalid_blocker_version_aborts_update() {
        let current = crate::version::RadegastVersion::parse("1.7.0").unwrap();
        assert!(automatic_update_blocker(&[release("invalid", false)], Some(&current)).is_err());
    }

    #[test]
    fn test_find_latest_for_platform() {
        let releases = vec![
            ReleaseEntry {
                version: "1.7.0r1".into(),
                hash_sha256: "hash1  linux-amd64.zip\nhash2  windows-amd64.zip\n".into(),
                sign_gpg: "sig1".into(),
                autoupdate: true,
            },
            ReleaseEntry {
                version: "1.7.0".into(),
                hash_sha256: "hash3  linux-amd64.zip\nhash4  mac-m5.zip\n".into(),
                sign_gpg: "sig2".into(),
                autoupdate: true,
            },
            ReleaseEntry {
                version: "1.6.0".into(),
                hash_sha256: "hash5  mac-m5.zip\n".into(),
                sign_gpg: "sig3".into(),
                autoupdate: true,
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

        let windows = crate::platform::Platform {
            os: "windows",
            arch: "amd64",
        };
        let (ver, entry) = find_latest_for_platform(&releases, &windows).unwrap();
        assert_eq!(ver.to_manifest_string(), "1.7.0r1");
        assert_eq!(entry.sign_gpg, "sig1");
    }

    #[test]
    fn test_find_latest_handles_empty_and_missing() {
        let empty_releases: Vec<ReleaseEntry> = vec![];
        let linux = crate::platform::Platform {
            os: "linux",
            arch: "amd64",
        };
        assert!(find_latest_for_platform(&empty_releases, &linux).is_none());

        let releases_without_mac = vec![ReleaseEntry {
            version: "1.7.0r1".into(),
            hash_sha256: "hash1  linux-amd64.zip\n".into(),
            sign_gpg: "sig1".into(),
            autoupdate: true,
        }];
        let mac = crate::platform::Platform {
            os: "mac",
            arch: "m5",
        };
        assert!(find_latest_for_platform(&releases_without_mac, &mac).is_none());
    }

    #[test]
    fn test_find_latest_skips_invalid_semver() {
        let releases = vec![
            ReleaseEntry {
                version: "invalid-semver".into(),
                hash_sha256: "hash0  linux-amd64.zip\n".into(),
                sign_gpg: "sig0".into(),
                autoupdate: true,
            },
            ReleaseEntry {
                version: "1.5.0".into(),
                hash_sha256: "hash1  linux-amd64.zip\n".into(),
                sign_gpg: "sig1".into(),
                autoupdate: true,
            },
        ];

        let linux = crate::platform::Platform {
            os: "linux",
            arch: "amd64",
        };
        let (ver, entry) = find_latest_for_platform(&releases, &linux).unwrap();
        assert_eq!(ver.to_manifest_string(), "1.5.0");
        assert_eq!(entry.sign_gpg, "sig1");
    }

    #[test]
    fn test_fetch_manifest_mock_server() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req_buf = [0u8; 1024];
                let _ = stream.read(&mut req_buf);

                let body = r#"[{"version":"1.7.1r1","hash_sha256":"h1 linux-amd64.zip\n","sign_gpg":"s1"}]"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let url = format!("http://127.0.0.1:{port}/manifest.json");
        let releases = fetch(&url).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version, "1.7.1r1");
    }

    #[test]
    fn test_fetch_until_success_retries_on_failure() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            let mut count = 0;
            while let Ok((mut stream, _)) = listener.accept() {
                count += 1;
                let mut req_buf = [0u8; 1024];
                let _ = stream.read(&mut req_buf);

                if count < 3 {
                    // First 2 requests fail with 503 Service Unavailable
                    let response = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                } else {
                    // 3rd request succeeds
                    let body = r#"[{"version":"1.8.0","hash_sha256":"h2 linux-amd64.zip\n","sign_gpg":"s2"}]"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    break;
                }
            }
        });

        let url = format!("http://127.0.0.1:{port}/manifest.json");
        let releases = fetch_until_success(&url, Duration::from_millis(10));
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version, "1.8.0");
    }

    #[test]
    fn test_fetch_with_retry_limit_exceeded() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                let mut req_buf = [0u8; 1024];
                let _ = stream.read(&mut req_buf);
                let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let url = format!("http://127.0.0.1:{port}/manifest.json");
        let res = fetch_with_retry_limit(&url, Duration::from_millis(10), Some(2));
        assert!(res.is_err());
    }
}
