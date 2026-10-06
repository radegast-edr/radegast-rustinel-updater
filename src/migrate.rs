use anyhow::{Context, Result};
use std::path::Path;
use std::time::Duration;

/// Migration script file names by platform.
fn migration_script_name() -> &'static str {
    if cfg!(windows) {
        "migrate.bat"
    } else {
        "migrate.sh"
    }
}

/// Check if the downloaded archive contains a migration script for the current platform.
/// If found, extract it to a temporary directory, execute it, and return the result.
/// Returns `Ok(true)` if a migration ran successfully, `Ok(false)` if no script was found.
pub fn run_if_present(
    archive_path: &Path,
    current_version: Option<&str>,
    new_version: &str,
    timeout: Duration,
) -> Result<bool> {
    let script_name = migration_script_name();

    let file = std::fs::File::open(archive_path)?;
    let mut zip = zip::ZipArchive::new(file)?;

    // Check if migration script exists in archive
    if zip.by_name(script_name).is_err() {
        tracing::info!(
            "No migration script ({}) found in archive, skipping.",
            script_name
        );
        return Ok(false);
    }

    // Extract to a temporary directory
    let tmp_dir = tempfile::tempdir().context("Failed to create temp dir for migration")?;
    let script_path = tmp_dir.path().join(script_name);

    {
        let mut entry = zip.by_name(script_name)?;
        let mut out_file = std::fs::File::create(&script_path)?;
        std::io::copy(&mut entry, &mut out_file)?;
    }

    // Set executable permission on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;
    }

    let current_str = current_version.unwrap_or("none");
    tracing::info!(
        "Running migration script: {} (current={}, new={})",
        script_path.display(),
        current_str,
        new_version,
    );

    // Build the command
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd.exe");
        c.args(["/c", &script_path.to_string_lossy()]);
        c
    } else {
        let mut c = std::process::Command::new("bash");
        c.arg(&script_path);
        c
    };

    // Pass version info as arguments and env vars
    cmd.arg(current_str);
    cmd.arg(new_version);
    cmd.env("RUSTINEL_CURRENT_VERSION", current_str);
    cmd.env("RUSTINEL_NEW_VERSION", new_version);

    let mut child = cmd.spawn().context("Failed to start migration script")?;

    // Wait with timeout using a separate thread
    let timeout_ms = timeout.as_millis() as u64;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    anyhow::bail!("Migration script failed with exit code {:?}", status.code(),);
                }
                tracing::info!("Migration script completed successfully.");
                return Ok(true);
            }
            Ok(None) => {
                if start.elapsed().as_millis() as u64 >= timeout_ms {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!(
                        "Migration script timed out after {} seconds",
                        timeout.as_secs()
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(e).context("Failed to wait for migration script");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn test_no_migration_script_returns_false() {
        let temp_dir = tempfile::tempdir().unwrap();
        let archive_path = temp_dir.path().join("test.zip");

        // Create zip without migration script
        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let binary_name = if cfg!(windows) {
            "rustinel.exe"
        } else {
            "rustinel"
        };
        zip.start_file(binary_name, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"fake-binary").unwrap();
        zip.finish().unwrap();

        let result = run_if_present(
            &archive_path,
            Some("1.0.0"),
            "2.0.0",
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(!result);
    }

    #[test]
    fn test_migration_script_success() {
        let temp_dir = tempfile::tempdir().unwrap();
        let archive_path = temp_dir.path().join("test.zip");

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let script_name = migration_script_name();
        let script_content = if cfg!(windows) {
            "@echo off\r\nexit /b 0\r\n"
        } else {
            "#!/bin/bash\nexit 0\n"
        };
        zip.start_file(script_name, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(script_content.as_bytes()).unwrap();
        zip.finish().unwrap();

        let result = run_if_present(
            &archive_path,
            Some("1.0.0"),
            "2.0.0",
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(result);
    }

    #[test]
    fn test_migration_script_failure_returns_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let archive_path = temp_dir.path().join("test.zip");

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let script_name = migration_script_name();
        let script_content = if cfg!(windows) {
            "@echo off\r\nexit /b 1\r\n"
        } else {
            "#!/bin/bash\nexit 1\n"
        };
        zip.start_file(script_name, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(script_content.as_bytes()).unwrap();
        zip.finish().unwrap();

        let result = run_if_present(
            &archive_path,
            Some("1.0.0"),
            "2.0.0",
            Duration::from_secs(10),
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Migration script failed"));
    }

    #[test]
    fn test_migration_script_receives_version_args() {
        let temp_dir = tempfile::tempdir().unwrap();
        let archive_path = temp_dir.path().join("test.zip");
        let marker_path = temp_dir.path().join("versions.txt");

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let script_name = migration_script_name();
        let script_content = if cfg!(windows) {
            format!(
                "@echo off\r\necho %1 %2 > \"{}\"\r\nexit /b 0\r\n",
                marker_path.to_string_lossy()
            )
        } else {
            format!(
                "#!/bin/bash\necho \"$1 $2\" > \"{}\"\nexit 0\n",
                marker_path.display()
            )
        };
        zip.start_file(script_name, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(script_content.as_bytes()).unwrap();
        zip.finish().unwrap();

        let result = run_if_present(
            &archive_path,
            Some("1.5.0"),
            "2.0.0",
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(result);

        let contents = std::fs::read_to_string(&marker_path).unwrap();
        assert!(contents.contains("1.5.0"));
        assert!(contents.contains("2.0.0"));
    }

    #[test]
    fn test_migration_script_timeout() {
        let temp_dir = tempfile::tempdir().unwrap();
        let archive_path = temp_dir.path().join("test.zip");

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let script_name = migration_script_name();
        let script_content = if cfg!(windows) {
            "@echo off\r\nping -n 30 127.0.0.1 > nul\r\nexit /b 0\r\n"
        } else {
            "#!/bin/bash\nsleep 30\nexit 0\n"
        };
        zip.start_file(script_name, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(script_content.as_bytes()).unwrap();
        zip.finish().unwrap();

        let result = run_if_present(
            &archive_path,
            Some("1.0.0"),
            "2.0.0",
            Duration::from_millis(500),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[test]
    fn test_migration_script_name_platform() {
        let name = migration_script_name();
        if cfg!(windows) {
            assert_eq!(name, "migrate.bat");
        } else {
            assert_eq!(name, "migrate.sh");
        }
    }

    #[test]
    fn test_migration_script_with_none_current_version() {
        let temp_dir = tempfile::tempdir().unwrap();
        let archive_path = temp_dir.path().join("test.zip");
        let marker_path = temp_dir.path().join("versions_none.txt");

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let script_name = migration_script_name();
        let script_content = if cfg!(windows) {
            format!(
                "@echo off\r\necho %1 %2 > \"{}\"\r\nexit /b 0\r\n",
                marker_path.to_string_lossy()
            )
        } else {
            format!(
                "#!/bin/bash\necho \"$1 $2\" > \"{}\"\nexit 0\n",
                marker_path.display()
            )
        };
        zip.start_file(script_name, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(script_content.as_bytes()).unwrap();
        zip.finish().unwrap();

        let result = run_if_present(&archive_path, None, "2.0.0", Duration::from_secs(10)).unwrap();
        assert!(result);

        let contents = std::fs::read_to_string(&marker_path).unwrap();
        assert!(contents.contains("none"));
        assert!(contents.contains("2.0.0"));
    }

    #[test]
    fn test_mock_zip_containing_both_linux_and_windows_scripts() {
        let temp_dir = tempfile::tempdir().unwrap();
        let archive_path = temp_dir.path().join("dual_platform.zip");
        let unix_marker = temp_dir.path().join("unix_marker.txt");
        let win_marker = temp_dir.path().join("win_marker.txt");

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        // Add migrate.sh
        let sh_content = format!(
            "#!/bin/bash\necho unix > \"{}\"\nexit 0\n",
            unix_marker.display()
        );
        zip.start_file("migrate.sh", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(sh_content.as_bytes()).unwrap();

        // Add migrate.bat
        let bat_content = format!(
            "@echo off\r\necho win > \"{}\"\r\nexit /b 0\r\n",
            win_marker.to_string_lossy()
        );
        zip.start_file("migrate.bat", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bat_content.as_bytes()).unwrap();

        zip.finish().unwrap();

        let result = run_if_present(
            &archive_path,
            Some("1.0.0"),
            "2.0.0",
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(result);

        if cfg!(windows) {
            assert!(win_marker.exists(), "On Windows, migrate.bat must run");
            assert!(!unix_marker.exists(), "On Windows, migrate.sh must NOT run");
        } else {
            assert!(unix_marker.exists(), "On Unix, migrate.sh must run");
            assert!(!win_marker.exists(), "On Unix, migrate.bat must NOT run");
        }
    }

    #[test]
    fn test_migration_script_custom_exit_code_captured() {
        let temp_dir = tempfile::tempdir().unwrap();
        let archive_path = temp_dir.path().join("custom_exit.zip");

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let script_name = migration_script_name();
        let script_content = if cfg!(windows) {
            "@echo off\r\nexit /b 42\r\n"
        } else {
            "#!/bin/bash\nexit 42\n"
        };
        zip.start_file(script_name, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(script_content.as_bytes()).unwrap();
        zip.finish().unwrap();

        let result = run_if_present(
            &archive_path,
            Some("1.0.0"),
            "2.0.0",
            Duration::from_secs(10),
        );
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("42"),
            "Expected exit code 42 in error message: {err_msg}"
        );
    }
}
