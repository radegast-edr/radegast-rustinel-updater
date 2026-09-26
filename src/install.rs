use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

pub fn replace_binary(archive_path: &Path, rustinel_path: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // On macOS, if the archive contains a macOS .app bundle, replace the app bundle
        let file = std::fs::File::open(archive_path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let has_app_bundle = (0..zip.len()).any(|i| {
            zip.by_index(i)
                .map(|e| e.name().starts_with("Rustinel.app"))
                .unwrap_or(false)
        });

        if has_app_bundle {
            return crate::platform::macos::replace_app_bundle(archive_path, rustinel_path);
        }
    }

    let target = Path::new(rustinel_path);
    let binary_name = if cfg!(windows) {
        "rustinel.exe"
    } else {
        "rustinel"
    };

    let file = std::fs::File::open(archive_path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut entry = zip
        .by_name(binary_name)
        .context("Binary not found in archive")?;

    anyhow::ensure!(entry.is_file(), "Archive entry is not a regular file");

    // Write to temp file in same directory for atomic rename
    let parent = target.parent().context("Binary path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut entry, &mut tmp)?;
    tmp.flush()?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o755))?;
    }

    // Atomic rename with retries and Windows fallback for locked files
    let mut current_tmp = tmp.into_temp_path();
    let mut last_err = None;

    #[cfg(windows)]
    let backup_path = std::path::PathBuf::from(format!("{}.old", target.display()));

    for attempt in 1..=5 {
        #[cfg(windows)]
        {
            if target.exists() {
                let _ = std::fs::remove_file(&backup_path);
                if let Err(e) = std::fs::rename(target, &backup_path) {
                    tracing::warn!(
                        "Attempt {attempt}/5: Failed to move existing binary to {}: {e}",
                        backup_path.display()
                    );
                }
            }
        }

        match current_tmp.persist(target) {
            Ok(_) => {
                #[cfg(windows)]
                {
                    let _ = std::fs::remove_file(&backup_path);
                }
                last_err = None;
                break;
            }
            Err(persist_err) => {
                tracing::warn!(
                    "Attempt {attempt}/5: Failed to replace binary at {}: {}. Retrying...",
                    target.display(),
                    persist_err.error
                );
                last_err = Some(persist_err.error);
                current_tmp = persist_err.path;
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    }

    if let Some(err) = last_err {
        return Err(err).context("Failed to replace rustinel binary");
    }

    // Clean up archive
    let _ = std::fs::remove_file(archive_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    #[test]
    fn test_replace_binary_success() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_path = temp_dir.path().join("rustinel_bin");
        let archive_path = temp_dir.path().join("archive.zip");

        let binary_name = if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        };

        // Create a zip archive
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file(binary_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"fake-rustinel-executable-payload").unwrap();
            zip.finish().unwrap();
        }

        assert!(archive_path.exists());
        assert!(!target_path.exists());

        // Replace binary
        replace_binary(&archive_path, target_path.to_str().unwrap()).unwrap();

        // Target should now exist with correct payload
        assert!(target_path.exists());
        let content = std::fs::read(&target_path).unwrap();
        assert_eq!(content, b"fake-rustinel-executable-payload");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&target_path).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o755);
        }

        // Archive should be cleaned up
        assert!(!archive_path.exists());
    }

    #[test]
    fn test_replace_binary_missing_entry() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_path = temp_dir.path().join("rustinel_bin");
        let archive_path = temp_dir.path().join("archive.zip");

        // Create zip archive with wrong file name
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("other_file.txt", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"wrong file").unwrap();
            zip.finish().unwrap();
        }

        let res = replace_binary(&archive_path, target_path.to_str().unwrap());
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("Binary not found in archive"));
    }

    #[test]
    fn test_replace_binary_corrupt_archive() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_path = temp_dir.path().join("rustinel_bin");
        let archive_path = temp_dir.path().join("corrupt.zip");

        std::fs::write(&archive_path, b"not a zip file").unwrap();

        let res = replace_binary(&archive_path, target_path.to_str().unwrap());
        assert!(res.is_err());
    }
}
