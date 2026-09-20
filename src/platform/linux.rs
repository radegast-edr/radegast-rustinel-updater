use anyhow::{Context, Result, ensure};

pub fn stop_service() -> Result<()> {
    tracing::info!("Stopping rustinel service...");
    let stop = std::process::Command::new("/usr/bin/systemctl")
        .args(["stop", "rustinel"])
        .status()
        .context("Failed to stop rustinel via systemctl")?;
    if !stop.success() {
        tracing::warn!("systemctl stop rustinel returned non-zero (service may not be running)");
    }
    Ok(())
}

pub fn start_service() -> Result<()> {
    tracing::info!("Starting rustinel service...");
    let start = std::process::Command::new("/usr/bin/systemctl")
        .args(["start", "rustinel"])
        .status()
        .context("Failed to start rustinel via systemctl")?;
    ensure!(start.success(), "systemctl start rustinel failed");
    Ok(())
}
