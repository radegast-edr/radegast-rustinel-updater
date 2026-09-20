use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

pub fn verify(archive_path: &Path, checksums: &str, platform: &crate::platform::Platform) -> Result<()> {
    let expected_filename = platform.archive_name();
    let expected_hash = checksums.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() { return None; }
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let filename = parts.next()?;
            (filename == expected_filename).then_some(hash)
        })
        .next()
        .with_context(|| format!("No checksum found for {expected_filename} in signed manifest"))?;
    
    let mut file = std::fs::File::open(archive_path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    if actual != expected_hash {
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

        let wrong_checksums = "1111111111111111111111111111111111111111111111111111111111111111  linux-amd64.zip\n";
        assert!(verify(temp_file.path(), wrong_checksums, &platform).is_err());

        let missing_platform = "1111111111111111111111111111111111111111111111111111111111111111  windows-amd64.zip\n";
        assert!(verify(temp_file.path(), missing_platform, &platform).is_err());
    }
}

