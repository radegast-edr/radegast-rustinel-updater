use anyhow::{Result, ensure};

pub fn stop_service() -> Result<()> {
    tracing::info!("Stopping rustinel service...");
    match std::process::Command::new("systemctl").args(["stop", "rustinel"]).status() {
        Ok(status) => {
            if !status.success() {
                tracing::warn!("systemctl stop rustinel returned non-zero (service may not be running)");
            }
        }
        Err(e) => {
            tracing::warn!("Failed to execute systemctl stop rustinel: {e}");
        }
    }
    Ok(())
}

pub fn start_service() -> Result<()> {
    tracing::info!("Starting rustinel service...");
    match std::process::Command::new("systemctl").args(["start", "rustinel"]).status() {
        Ok(status) => {
            ensure!(status.success(), "systemctl start rustinel failed with exit code: {:?}", status.code());
        }
        Err(e) => {
            tracing::warn!("Failed to execute systemctl start rustinel: {e}");
        }
    }
    Ok(())
}
