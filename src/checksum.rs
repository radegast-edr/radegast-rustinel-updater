use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

pub fn verify(
    archive_path: &Path,
    checksums: &str,
    platform: &crate::platform::Platform,
) -> Result<()> {
    let expected_filename = platform.archive_name();
    let expected_hash = checksums
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let filename = parts.next()?;
            (filename == expected_filename).then_some(hash)
        })
        .next()
        .with_context(|| format!("No checksum found for {expected_filename} in signed manifest"))?;

    let mut file = std::fs::File::open(archive_path)
        .with_context(|| format!("Failed to open archive at {:?}", archive_path))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_hash) {
        bail!("SHA256 mismatch for {expected_filename}: expected {expected_hash}, got {actual}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_checksum_verify_match_and_mismatch() {
        let platform = crate::platform::Platform {
            os: "linux",
            arch: "amd64",
        };
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        temp_file.write_all(b"test binary content").unwrap();

        let correct_hash = hex::encode(Sha256::digest(b"test binary content"));
        let checksums = format!("{correct_hash}  linux-amd64.zip\n");

        assert!(verify(temp_file.path(), &checksums, &platform).is_ok());

        let wrong_checksums =
            "1111111111111111111111111111111111111111111111111111111111111111  linux-amd64.zip\n";
        assert!(verify(temp_file.path(), wrong_checksums, &platform).is_err());

        let missing_platform =
            "1111111111111111111111111111111111111111111111111111111111111111  windows-amd64.zip\n";
        assert!(verify(temp_file.path(), missing_platform, &platform).is_err());
    }

    #[test]
    fn test_checksum_case_insensitivity_and_multiline() {
        let platform = crate::platform::Platform {
            os: "windows",
            arch: "amd64",
        };
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        temp_file.write_all(b"windows payload").unwrap();

        let hash_lower = hex::encode(Sha256::digest(b"windows payload"));
        let hash_upper = hash_lower.to_ascii_uppercase();

        let multiline_manifest = format!(
            "0000000000000000000000000000000000000000000000000000000000000000  linux-amd64.zip\n\
             {hash_upper}   windows-amd64.zip\n\
             1111111111111111111111111111111111111111111111111111111111111111  mac-m5.zip\n"
        );

        assert!(verify(temp_file.path(), &multiline_manifest, &platform).is_ok());
    }

    #[test]
    fn test_checksum_nonexistent_file() {
        let platform = crate::platform::Platform {
            os: "linux",
            arch: "amd64",
        };
        let path = Path::new("/tmp/nonexistent-archive-file-12345.zip");
        let checksums =
            "0000000000000000000000000000000000000000000000000000000000000000  linux-amd64.zip\n";
        assert!(verify(path, checksums, &platform).is_err());
    }

    #[test]
    fn test_checksum_empty_manifest() {
        let platform = crate::platform::Platform {
            os: "linux",
            arch: "amd64",
        };
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        assert!(verify(temp_file.path(), "", &platform).is_err());
        assert!(verify(temp_file.path(), "\n   \n", &platform).is_err());
    }
}
