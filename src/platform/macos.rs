use anyhow::{ensure, Context, Result};
use std::path::Path;

const LAUNCHD_LABEL: &str = "io.rustinel.daemon";

/// Replace the entire Rustinel.app bundle to preserve code signature,
/// and ensure rustinel_path points to the app bundle's binary.
pub fn replace_app_bundle(archive_path: &Path, rustinel_path: &str) -> Result<()> {
    let target = Path::new(rustinel_path);
    // Find the directory containing or destined to contain Rustinel.app
    let bundle_dir = if target.to_string_lossy().contains("Rustinel.app") {
        let mut cur = target;
        while let Some(p) = cur.parent() {
            if cur
                .file_name()
                .map(|n| n == "Rustinel.app")
                .unwrap_or(false)
            {
                break;
            }
            cur = p;
        }
        cur.parent().unwrap_or(cur)
    } else {
        target
            .parent()
            .unwrap_or(Path::new("/Library/Radegast/rustinel"))
    };
    std::fs::create_dir_all(bundle_dir)?;

    let staging = tempfile::Builder::new()
        .prefix(".tmp_staging")
        .tempdir_in(bundle_dir)
        .or_else(|_| tempfile::tempdir())
        .context("Failed to create staging directory")?;

    // Extract zip to staging
    let file = std::fs::File::open(archive_path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let out_path = staging.path().join(entry.name());
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut outfile = std::fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut outfile)?;
            // Preserve executable permissions
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = entry.unix_mode() {
                    let _ =
                        std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode));
                }
            }
        }
    }

    let extracted_app = staging.path().join("Rustinel.app");
    ensure!(
        extracted_app.exists(),
        "Extracted archive does not contain Rustinel.app"
    );

    // Verify code signature of the extracted bundle if codesign tool is present
    if Path::new("/usr/bin/codesign").exists() {
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

    // Backup current bundle
    let backup = staging.path().join("previous.app");
    if installed.exists() {
        let _ = std::fs::rename(&installed, &backup);
    }

    // Use ditto for atomic replacement (preserves extended attributes)
    let ditto_success = if Path::new("/usr/bin/ditto").exists() {
        std::process::Command::new("/usr/bin/ditto")
            .arg(&extracted_app)
            .arg(&installed)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        false
    };

    if !ditto_success {
        // Fallback to rename
        if let Err(e) = std::fs::rename(&extracted_app, &installed) {
            if backup.exists() {
                let _ = std::fs::rename(&backup, &installed);
            }
            anyhow::bail!(
                "Failed to copy new app bundle to {}: {e}",
                installed.display()
            );
        }
    }

    // Ensure rustinel_path points to the app binary inside the installed bundle
    let app_bin = installed.join("Contents/MacOS/rustinel");
    if app_bin.exists() && target != app_bin {
        let _ = std::fs::remove_file(target);
        let symlink_created = {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&app_bin, target).is_ok()
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        if !symlink_created {
            std::fs::copy(&app_bin, target)
                .context("Failed to copy app binary to rustinel path")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755));
            }
        }
    }

    // Clean up archive
    let _ = std::fs::remove_file(archive_path);

    Ok(())
}

pub fn stop_service() -> Result<()> {
    tracing::info!("Stopping rustinel service...");
    let _ = std::process::Command::new("/bin/launchctl")
        .args(["stop", LAUNCHD_LABEL])
        .status();
    // Give it a moment to stop
    std::thread::sleep(std::time::Duration::from_secs(2));
    Ok(())
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
