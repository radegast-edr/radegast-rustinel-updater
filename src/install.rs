use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// Apply an update from a downloaded release archive.
///
/// If a migration script is present in the archive (`migrate.sh` on Linux/macOS,
/// `migrate.bat` on Windows), it is executed BEFORE the rustinel binary is replaced.
/// If the migration script fails (non-zero exit code or timeout), the update is aborted
/// and the existing binary is left untouched.
pub fn apply_update(
    archive_path: &Path,
    rustinel_path: &str,
    current_version: Option<&str>,
    new_version: &str,
    migration_timeout: Duration,
) -> Result<()> {
    crate::migrate::run_if_present(
        archive_path,
        current_version,
        new_version,
        migration_timeout,
    )?;
    replace_binary(archive_path, rustinel_path)?;
    Ok(())
}

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

    #[test]
    fn test_apply_update_executes_migration_script_and_replaces_binary_native() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_path = temp_dir.path().join(if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        });
        let archive_path = temp_dir.path().join("release-update.zip");
        let marker_path = temp_dir.path().join("migration_applied.txt");

        // Pre-create existing binary with old content
        std::fs::write(&target_path, b"old-rustinel-v1.0.0").unwrap();

        let binary_name = if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        };
        let script_name = if cfg!(windows) {
            "migrate.bat"
        } else {
            "migrate.sh"
        };

        // Create mock zip containing target binary and platform migration script
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);

            // 1. New binary entry
            zip.start_file(binary_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"new-rustinel-v2.0.0").unwrap();

            // 2. Migration script entry
            let script_content = if cfg!(windows) {
                format!(
                    "@echo off\r\necho %1 %2 > \"{}\"\r\nexit /b 0\r\n",
                    marker_path.to_string_lossy()
                )
            } else {
                format!(
                    "#!/bin/bash\nset -e\nif [ \"$1\" != \"1.0.0\" ] || [ \"$2\" != \"2.0.0\" ]; then\n  exit 1\nfi\nif [ \"$RUSTINEL_CURRENT_VERSION\" != \"1.0.0\" ] || [ \"$RUSTINEL_NEW_VERSION\" != \"2.0.0\" ]; then\n  exit 2\nfi\necho \"migrated $1 -> $2\" > \"{}\"\nexit 0\n",
                    marker_path.display()
                )
            };
            zip.start_file(script_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(script_content.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        assert!(archive_path.exists());
        assert!(!marker_path.exists());

        // Apply update
        apply_update(
            &archive_path,
            target_path.to_str().unwrap(),
            Some("1.0.0"),
            "2.0.0",
            Duration::from_secs(10),
        )
        .unwrap();

        // 1. Verify migration script ran
        assert!(marker_path.exists(), "Migration script marker must exist");
        let marker_content = std::fs::read_to_string(&marker_path).unwrap();
        assert!(marker_content.contains("1.0.0"));
        assert!(marker_content.contains("2.0.0"));

        // 2. Verify binary was replaced
        assert!(target_path.exists());
        let updated_binary = std::fs::read(&target_path).unwrap();
        assert_eq!(updated_binary, b"new-rustinel-v2.0.0");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&target_path).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o755);
        }

        // 3. Verify archive was cleaned up
        assert!(!archive_path.exists());
    }

    #[test]
    fn test_apply_update_migration_failure_aborts_and_preserves_binary() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_path = temp_dir.path().join(if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        });
        let archive_path = temp_dir.path().join("failing-update.zip");

        // Target has original safe binary
        std::fs::write(&target_path, b"original-safe-binary-v1").unwrap();

        let binary_name = if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        };
        let script_name = if cfg!(windows) {
            "migrate.bat"
        } else {
            "migrate.sh"
        };

        // Create mock zip with failing migration script
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);

            zip.start_file(binary_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"broken-untested-binary-v2").unwrap();

            let script_content = if cfg!(windows) {
                "@echo off\r\nexit /b 1\r\n"
            } else {
                "#!/bin/bash\nexit 1\n"
            };
            zip.start_file(script_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(script_content.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        // Apply update must fail
        let res = apply_update(
            &archive_path,
            target_path.to_str().unwrap(),
            Some("1.0.0"),
            "2.0.0",
            Duration::from_secs(10),
        );
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("Migration script failed"));

        // Binary MUST be untouched and preserved!
        let current_binary = std::fs::read(&target_path).unwrap();
        assert_eq!(
            current_binary, b"original-safe-binary-v1",
            "Target binary must remain untouched when migration fails"
        );
    }

    #[test]
    fn test_apply_update_migration_script_strictly_executes_before_binary_replacement() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_path = temp_dir.path().join(if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        });
        let archive_path = temp_dir.path().join("timing-update.zip");
        let timing_marker = temp_dir.path().join("timing_verified.txt");

        std::fs::write(&target_path, b"pre-update-payload-12345").unwrap();

        let binary_name = if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        };
        let script_name = if cfg!(windows) {
            "migrate.bat"
        } else {
            "migrate.sh"
        };

        // Migration script asserts target file has the PRE-UPDATE payload
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);

            zip.start_file(binary_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"post-update-payload-67890").unwrap();

            let script_content = if cfg!(windows) {
                format!(
                    "@echo off\r\nfindstr /m \"pre-update-payload-12345\" \"{}\" > nul\r\nif errorlevel 1 exit /b 42\r\necho verified > \"{}\"\r\nexit /b 0\r\n",
                    target_path.to_string_lossy(),
                    timing_marker.to_string_lossy()
                )
            } else {
                format!(
                    "#!/bin/bash\nset -e\nif ! grep -q \"pre-update-payload-12345\" \"{}\"; then\n  echo \"Target already replaced!\" >&2\n  exit 42\nfi\necho \"verified\" > \"{}\"\nexit 0\n",
                    target_path.display(),
                    timing_marker.display()
                )
            };
            zip.start_file(script_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(script_content.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        apply_update(
            &archive_path,
            target_path.to_str().unwrap(),
            Some("1.0.0"),
            "2.0.0",
            Duration::from_secs(10),
        )
        .unwrap();

        // 1. Timing marker proves migration ran while old binary was present
        assert!(
            timing_marker.exists(),
            "Timing marker must exist, proving script ran before replacement"
        );

        // 2. Target binary has now been replaced with post-update payload
        let current_binary = std::fs::read(&target_path).unwrap();
        assert_eq!(current_binary, b"post-update-payload-67890");
    }

    #[test]
    fn test_apply_update_without_migration_script_succeeds() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_path = temp_dir.path().join(if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        });
        let archive_path = temp_dir.path().join("standard-update.zip");

        std::fs::write(&target_path, b"old-binary-without-migration").unwrap();

        let binary_name = if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        };

        // Mock zip without migration script
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);

            zip.start_file(binary_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"new-binary-without-migration").unwrap();

            zip.finish().unwrap();
        }

        apply_update(
            &archive_path,
            target_path.to_str().unwrap(),
            Some("1.0.0"),
            "2.0.0",
            Duration::from_secs(10),
        )
        .unwrap();

        let current_binary = std::fs::read(&target_path).unwrap();
        assert_eq!(current_binary, b"new-binary-without-migration");
        assert!(!archive_path.exists());
    }

    #[test]
    fn test_apply_update_migration_timeout_aborts_and_preserves_binary() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_path = temp_dir.path().join(if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        });
        let archive_path = temp_dir.path().join("timeout-update.zip");

        std::fs::write(&target_path, b"safe-binary-pre-timeout").unwrap();

        let binary_name = if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        };
        let script_name = if cfg!(windows) {
            "migrate.bat"
        } else {
            "migrate.sh"
        };

        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);

            zip.start_file(binary_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"new-binary-should-not-exist").unwrap();

            let script_content = if cfg!(windows) {
                "@echo off\r\nping -n 30 127.0.0.1 > nul\r\nexit /b 0\r\n"
            } else {
                "#!/bin/bash\nsleep 30\nexit 0\n"
            };
            zip.start_file(script_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(script_content.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        let res = apply_update(
            &archive_path,
            target_path.to_str().unwrap(),
            Some("1.0.0"),
            "2.0.0",
            Duration::from_millis(300),
        );
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("timed out"));

        // Binary must be preserved!
        let current_binary = std::fs::read(&target_path).unwrap();
        assert_eq!(current_binary, b"safe-binary-pre-timeout");
    }

    #[test]
    fn test_apply_update_with_none_current_version() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_path = temp_dir.path().join(if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        });
        let archive_path = temp_dir.path().join("none-version-update.zip");
        let marker_path = temp_dir.path().join("none_verified.txt");

        let binary_name = if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        };
        let script_name = if cfg!(windows) {
            "migrate.bat"
        } else {
            "migrate.sh"
        };

        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);

            zip.start_file(binary_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"fresh-install-binary").unwrap();

            let script_content = if cfg!(windows) {
                format!(
                    "@echo off\r\necho %1 > \"{}\"\r\nexit /b 0\r\n",
                    marker_path.to_string_lossy()
                )
            } else {
                format!(
                    "#!/bin/bash\nif [ \"$1\" != \"none\" ] || [ \"$RUSTINEL_CURRENT_VERSION\" != \"none\" ]; then\n  exit 1\nfi\necho \"ok\" > \"{}\"\nexit 0\n",
                    marker_path.display()
                )
            };
            zip.start_file(script_name, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(script_content.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        apply_update(
            &archive_path,
            target_path.to_str().unwrap(),
            None,
            "2.0.0",
            Duration::from_secs(10),
        )
        .unwrap();

        assert!(marker_path.exists());
        let current_binary = std::fs::read(&target_path).unwrap();
        assert_eq!(current_binary, b"fresh-install-binary");
    }

    #[cfg(not(windows))]
    #[test]
    fn test_apply_update_windows_script_ignored_on_unix() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_path = temp_dir.path().join("rustinel");
        let archive_path = temp_dir.path().join("windows-in-unix.zip");

        std::fs::write(&target_path, b"pre-windows-ignored").unwrap();

        // Archive contains only migrate.bat (Windows script) and rustinel binary
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);

            zip.start_file("rustinel", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"post-windows-ignored").unwrap();

            zip.start_file("migrate.bat", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"@echo off\r\nexit /b 1\r\n").unwrap(); // Even if it would fail, it shouldn't run on Unix

            zip.finish().unwrap();
        }

        // On Unix, migrate.sh is looked for. Since only migrate.bat is present,
        // it is ignored, and binary replacement succeeds without running the batch file.
        apply_update(
            &archive_path,
            target_path.to_str().unwrap(),
            Some("1.0.0"),
            "2.0.0",
            Duration::from_secs(10),
        )
        .unwrap();

        let current_binary = std::fs::read(&target_path).unwrap();
        assert_eq!(current_binary, b"post-windows-ignored");
    }

    #[test]
    fn test_apply_update_mock_zip_macos_bundle_structure() {
        let temp_dir = tempfile::tempdir().unwrap();
        let archive_path = temp_dir.path().join("macos-mock.zip");
        let marker_path = temp_dir.path().join("macos_migrated.txt");

        // Create a zip with macOS bundle layout + migrate.sh
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);

            zip.start_file(
                "Rustinel.app/Contents/MacOS/rustinel",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"macos-bundle-binary-payload").unwrap();

            let script_content = if cfg!(windows) {
                format!(
                    "@echo off\r\necho macos-migrated > \"{}\"\r\nexit /b 0\r\n",
                    marker_path.to_string_lossy()
                )
            } else {
                format!(
                    "#!/bin/bash\necho \"macos-migrated\" > \"{}\"\nexit 0\n",
                    marker_path.display()
                )
            };
            zip.start_file(
                if cfg!(windows) {
                    "migrate.bat"
                } else {
                    "migrate.sh"
                },
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(script_content.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        #[cfg(target_os = "macos")]
        {
            let target_bundle_path = temp_dir.path().join("Rustinel.app/Contents/MacOS/rustinel");
            std::fs::create_dir_all(target_bundle_path.parent().unwrap()).unwrap();
            std::fs::write(&target_bundle_path, b"old-bundle-payload").unwrap();

            apply_update(
                &archive_path,
                target_bundle_path.to_str().unwrap(),
                Some("1.0.0"),
                "2.0.0",
                Duration::from_secs(10),
            )
            .unwrap();

            assert!(marker_path.exists());
        }

        #[cfg(not(target_os = "macos"))]
        {
            // On non-macOS, test that migrate::run_if_present extracts and runs the script from this zip
            let ran = crate::migrate::run_if_present(
                &archive_path,
                Some("1.0.0"),
                "2.0.0",
                Duration::from_secs(10),
            )
            .unwrap();
            assert!(ran);
            assert!(marker_path.exists());
        }
    }
}
