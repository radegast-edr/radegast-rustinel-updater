use anyhow::{Context, Result, ensure};

const SERVICE_NAME: &str = "Rustinel";

pub fn stop_service() -> Result<()> {
    tracing::info!("Stopping Rustinel service...");
    let stop = std::process::Command::new("sc.exe")
        .args(["stop", SERVICE_NAME])
        .status()
        .context("Failed to stop Rustinel service")?;
    if !stop.success() {
        tracing::warn!("sc.exe stop returned non-zero (service may not be running)");
    }
    
    // Wait for service to fully stop (important on Windows - binary is locked while running)
    tracing::info!("Waiting for service to stop...");
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let output = std::process::Command::new("sc.exe")
            .args(["query", SERVICE_NAME])
            .output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("STOPPED") {
                break;
            }
        }
    }
    Ok(())
}

pub fn start_service() -> Result<()> {
    tracing::info!("Starting Rustinel service...");
    let start = std::process::Command::new("sc.exe")
        .args(["start", SERVICE_NAME])
        .status()
        .context("Failed to start Rustinel service")?;
    ensure!(start.success(), "sc.exe start Rustinel failed");
    Ok(())
}
