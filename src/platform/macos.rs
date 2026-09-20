use anyhow::{Context, Result, ensure};
use std::path::Path;
use std::io::Read;

const LAUNCHD_LABEL: &str = "com.rustinel.sensor";
const APP_BUNDLE_PATH: &str = "/Applications/Rustinel.app";

/// Replace the entire Rustinel.app bundle to preserve code signature.
pub fn replace_app_bundle(archive_path: &Path) -> Result<()> {
    let staging = tempfile::tempdir_in("/Applications")
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
                    std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode))?;
                }
            }
        }
    }
    
    let extracted_app = staging.path().join("Rustinel.app");
    ensure!(extracted_app.exists(), "Extracted archive does not contain Rustinel.app");
    
    // Verify code signature of the extracted bundle
    let verify = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&extracted_app)
        .status()
        .context("Failed to run codesign verification")?;
    ensure!(verify.success(), "Code signature verification failed on extracted bundle");
    
    let installed = Path::new(APP_BUNDLE_PATH);
    
    // Backup current bundle
    let backup = staging.path().join("previous.app");
    if installed.exists() {
        std::fs::rename(installed, &backup)
            .context("Failed to move current bundle to backup")?;
    }
    
    // Use ditto for atomic replacement (preserves extended attributes)
    let ditto = std::process::Command::new("/usr/bin/ditto")
        .arg(&extracted_app)
        .arg(installed)
        .status()
        .context("Failed to copy new bundle via ditto")?;
    
    if !ditto.success() {
        // Restore backup
        if backup.exists() {
            let _ = std::fs::rename(&backup, installed);
        }
        anyhow::bail!("ditto failed to install new app bundle");
    }
    
    Ok(())
}

pub fn restart_service() -> Result<()> {
    tracing::info!("Stopping rustinel service...");
    let _ = std::process::Command::new("/bin/launchctl")
        .args(["stop", LAUNCHD_LABEL])
        .status();
    
    // Give it a moment to stop
    std::thread::sleep(std::time::Duration::from_secs(2));
    
    tracing::info!("Starting rustinel service...");
    let start = std::process::Command::new("/bin/launchctl")
        .args(["start", LAUNCHD_LABEL])
        .status()
        .context("Failed to start rustinel via launchctl")?;
    ensure!(start.success(), "launchctl start failed");
    Ok(())
}
