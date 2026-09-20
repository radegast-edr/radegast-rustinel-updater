use anyhow::{Context, Result};
use reqwest::blocking::Client;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

pub fn fetch_binary(
    download_url: &str,
    version_str: &str,
    platform: &crate::platform::Platform,
) -> Result<PathBuf> {
    let client = Client::builder()
        .user_agent(concat!("radegast-rustinel-updater/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()?;
    let url = format!(
        "{}/public/releases/{}/{}/{}/download",
        download_url.trim_end_matches('/'),
        version_str,
        platform.os_name(),
        platform.arch_name(),
    );
    tracing::info!("Downloading from {url}");
    let response = client.get(&url).send()?.error_for_status()
        .context("Failed to download rustinel binary")?;
    let mut tmp = tempfile::NamedTempFile::new()?;
    let bytes = response.bytes()?;
    tmp.write_all(&bytes)?;
    tmp.flush()?;
    let path = tmp.into_temp_path().keep()?;
    Ok(path)
}
