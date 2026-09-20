use anyhow::{Context, Result};
use std::path::Path;
use std::io::Write;

pub fn replace_binary(archive_path: &Path, rustinel_path: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // On macOS, replace the entire .app bundle
        crate::platform::macos::replace_app_bundle(archive_path)?;
        return Ok(());
    }
    
    #[cfg(not(target_os = "macos"))]
    {
        let target = Path::new(rustinel_path);
        let binary_name = if cfg!(windows) { "rustinel.exe" } else { "rustinel" };
        
        let file = std::fs::File::open(archive_path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut entry = zip.by_name(binary_name)
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
            tmp.as_file().set_permissions(std::fs::Permissions::from_mode(0o755))?;
        }
        
        // Atomic rename
        let tmp_path = tmp.into_temp_path();
        tmp_path.persist(target)
            .context("Failed to replace rustinel binary")?;
        
        // Clean up archive
        let _ = std::fs::remove_file(archive_path);
        
        Ok(())
    }
}
