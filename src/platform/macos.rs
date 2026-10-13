use anyhow::{ensure, Context, Result};
use std::path::Path;

const LAUNCHD_LABEL: &str = "io.rustinel.daemon";

/// Replace the entire Rustinel.app bundle to preserve code signature,
/// and ensure rustinel_path points to the app bundle's binary.
pub fn replace_app_bundle(archive_path: &Path, rustinel_path: &str) -> Result<()> {
    let target = Path::new(rustinel_path);
    // Find the directory containing or destined to contain Rustinel.app
    let bundle_dir = target
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "Rustinel.app"))
        .and_then(Path::parent)
        .or_else(|| target.parent())
        .context("Rustinel path has no parent directory")?;
    std::fs::create_dir_all(bundle_dir)?;

    let staging = tempfile::Builder::new()
        .prefix(".tmp_staging")
        .tempdir_in(bundle_dir)
        .context("Failed to create staging directory")?;

    crate::install::extract_bundle(archive_path, staging.path())?;

    let extracted_app = staging.path().join("Rustinel.app");
    ensure!(
        extracted_app.exists(),
        "Extracted archive does not contain Rustinel.app"
    );

    // Require verification of the extracted bundle in production
    // (skipped in test builds to allow mock test bundles)
    if !cfg!(test) {
        let verify = std::process::Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&extracted_app)
            .status()
            .context("Failed to run codesign verification")?;
        ensure!(
            verify.success(),
            "Code signature verification failed on extracted bundle"
        );
    }

    let installed = bundle_dir.join("Rustinel.app");

    ensure!(
        extracted_app.join("Contents/MacOS/rustinel").is_file(),
        "Bundle is missing rustinel executable"
    );
    // Keep the backup outside the disposable staging directory so failed
    // rollback cannot delete the last working installation.
    let backup = bundle_dir.join("Rustinel.app.old");
    ensure!(
        !backup.exists(),
        "Previous backup exists at {}; recover it before updating",
        backup.display()
    );
    let had_previous = installed.exists();
    if had_previous {
        std::fs::rename(&installed, &backup).context("Failed to back up installed bundle")?;
    }
    if let Err(error) = std::fs::rename(&extracted_app, &installed) {
        if had_previous {
            std::fs::rename(&backup, &installed).with_context(|| {
                format!(
                    "Bundle replacement failed ({error}); backup retained at {}",
                    backup.display()
                )
            })?;
        }
        return Err(error).context("Failed to install new bundle");
    }

    // Stage the launcher before replacing it, preserving the old launcher on failure.
    let app_bin = installed.join("Contents/MacOS/rustinel");
    if target != app_bin {
        let launcher = staging.path().join("launcher");
        let result = (|| -> Result<()> {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::canonicalize(&app_bin)?, &launcher)?;
            #[cfg(not(unix))]
            std::fs::copy(&app_bin, &launcher)?;
            std::fs::rename(&launcher, target).context("Failed to install bundle launcher")?;
            Ok(())
        })();
        if let Err(error) = result {
            std::fs::rename(&installed, &extracted_app).context(
                "Failed to remove new bundle during rollback; previous bundle retained in backup",
            )?;
            if had_previous {
                std::fs::rename(&backup, &installed)
                    .context("Failed to restore previous bundle; retained in backup")?;
            }
            return Err(error);
        }
    }

    if had_previous {
        if let Err(error) = std::fs::remove_dir_all(&backup) {
            tracing::warn!("Updated bundle installed, but backup cleanup failed: {error}");
        }
    }

    // Clean up archive
    let _ = std::fs::remove_file(archive_path);

    Ok(())
}

pub fn stop_service() -> Result<()> {
    tracing::info!("Stopping rustinel service...");
    let status = std::process::Command::new("/bin/launchctl")
        .args(["stop", LAUNCHD_LABEL])
        .status()
        .context("Failed to stop rustinel via launchctl")?;
    ensure!(status.success(), "launchctl stop failed");
    for _ in 0..60 {
        let output = std::process::Command::new("/bin/launchctl")
            .arg("list")
            .output()?;
        ensure!(output.status.success(), "Failed to query launchd services");
        if service_has_stopped(&String::from_utf8_lossy(&output.stdout)) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    anyhow::bail!("Timed out waiting for Rustinel to stop")
}

pub(crate) fn service_has_stopped(output: &str) -> bool {
    output.lines().any(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        fields.len() == 3 && fields[2] == LAUNCHD_LABEL && fields[0] == "-"
    })
}

pub fn start_service() -> Result<()> {
    tracing::info!("Starting rustinel service...");
    let start = std::process::Command::new("/bin/launchctl")
        .args(["start", LAUNCHD_LABEL])
        .status()
        .context("Failed to start rustinel via launchctl")?;
    ensure!(start.success(), "launchctl start failed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn stopped_state_requires_exact_label_and_no_pid() {
        assert!(service_has_stopped("-\t0\tio.rustinel.daemon"));
        assert!(!service_has_stopped("123\t0\tio.rustinel.daemon"));
        assert!(!service_has_stopped("-\t0\tio.rustinel.daemon.other"));
        assert!(!service_has_stopped(""));
    }

    #[test]
    fn launcher_failure_restores_previous_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("Rustinel.app");
        std::fs::create_dir(&installed).unwrap();
        std::fs::write(installed.join("old-marker"), b"old").unwrap();
        let target = dir.path().join("launcher");
        // A directory cannot be overwritten by the staged launcher.
        std::fs::create_dir(&target).unwrap();
        let archive = dir.path().join("bundle.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        zip.start_file(
            "Rustinel.app/Contents/MacOS/rustinel",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"new").unwrap();
        zip.finish().unwrap();
        assert!(replace_app_bundle(&archive, target.to_str().unwrap()).is_err());
        assert_eq!(std::fs::read(installed.join("old-marker")).unwrap(), b"old");
        assert!(target.is_dir());
    }
}
