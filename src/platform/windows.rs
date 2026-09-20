use anyhow::{Context, Result};

pub const CANDIDATE_SERVICES: &[&str] = &["RadegastRustinel", "Rustinel"];

pub fn parse_sc_query_is_stopped(output: &str) -> bool {
    // 1060: The specified service does not exist as an installed service.
    // 1062: The service has not been started.
    // Or STATE indicates STOPPED.
    output.contains("1060")
        || output.contains("does not exist")
        || output.contains("1062")
        || output.contains("has not been started")
        || output.contains("STOPPED")
}

pub fn parse_sc_query_exists(output: &str, status_success: bool) -> bool {
    if output.contains("1060") || output.contains("does not exist") {
        return false;
    }
    status_success || output.contains("STATE")
}

fn service_exists(name: &str) -> bool {
    match std::process::Command::new("sc.exe")
        .args(["query", name])
        .output()
    {
        Ok(out) => {
            let combined = format!(
                "{} {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            parse_sc_query_exists(&combined, out.status.success())
        }
        Err(e) => {
            tracing::warn!("Failed to invoke sc.exe query {name}: {e}");
            false
        }
    }
}

pub fn find_service_name() -> Option<&'static str> {
    CANDIDATE_SERVICES
        .iter()
        .find(|&&name| service_exists(name))
        .copied()
}

fn is_service_stopped(name: &str) -> bool {
    let output = match std::process::Command::new("sc.exe")
        .args(["query", name])
        .output()
    {
        Ok(out) => out,
        Err(_) => return true,
    };
    let combined = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_sc_query_is_stopped(&combined)
}

fn kill_lingering_processes() {
    // Defense-in-depth: Ensure any orphaned rustinel.exe process that may still hold
    // a file lock is terminated before binary replacement.
    let _ = std::process::Command::new("taskkill.exe")
        .args(["/F", "/IM", "rustinel.exe"])
        .output();
}

pub fn stop_service() -> Result<()> {
    let service_name = match find_service_name() {
        Some(name) => name,
        None => {
            tracing::warn!(
                "No Rustinel service found on system (checked {:?}), skipping service stop",
                CANDIDATE_SERVICES
            );
            kill_lingering_processes();
            return Ok(());
        }
    };

    if is_service_stopped(service_name) {
        tracing::info!("{service_name} service is already stopped");
        kill_lingering_processes();
        return Ok(());
    }

    tracing::info!("Stopping {service_name} service...");
    match std::process::Command::new("sc.exe")
        .args(["stop", service_name])
        .output()
    {
        Ok(output) => {
            let combined = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if !output.status.success() {
                if combined.contains("1062") || combined.contains("has not been started") {
                    tracing::info!("{service_name} service was not running");
                    kill_lingering_processes();
                    return Ok(());
                }
                tracing::warn!("sc.exe stop {service_name} returned non-zero: {combined}");
            }
        }
        Err(e) => {
            tracing::warn!("Failed to execute sc.exe stop {service_name}: {e}");
        }
    }

    // Wait for service to fully stop (important on Windows - binary is locked while running)
    tracing::info!("Waiting for {service_name} service to stop...");
    for _ in 0..60 {
        if is_service_stopped(service_name) {
            tracing::info!("{service_name} service has stopped");
            kill_lingering_processes();
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    tracing::warn!("Timed out waiting for {service_name} service to stop");
    kill_lingering_processes();
    Ok(())
}

pub fn start_service() -> Result<()> {
    let service_name = match find_service_name() {
        Some(name) => name,
        None => {
            tracing::warn!(
                "No Rustinel service found on system (checked {:?}), skipping service start",
                CANDIDATE_SERVICES
            );
            return Ok(());
        }
    };

    tracing::info!("Starting {service_name} service...");
    let start_output = std::process::Command::new("sc.exe")
        .args(["start", service_name])
        .output()
        .context(format!("Failed to execute sc.exe start {service_name}"))?;

    let combined = format!(
        "{} {}",
        String::from_utf8_lossy(&start_output.stdout),
        String::from_utf8_lossy(&start_output.stderr)
    );

    if !start_output.status.success() {
        if combined.contains("1056") || combined.contains("already running") {
            tracing::info!("{service_name} service is already running");
            return Ok(());
        }
        anyhow::bail!("sc.exe start {service_name} failed: {combined}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sc_query_is_stopped() {
        let not_found = "[SC] OpenService FAILED 1060:\n\nThe specified service does not exist as an installed service.";
        assert!(parse_sc_query_is_stopped(not_found));

        let not_started = "[SC] ControlService FAILED 1062:\n\nThe service has not been started.";
        assert!(parse_sc_query_is_stopped(not_started));

        let stopped_status = "SERVICE_NAME: RadegastRustinel\n        TYPE               : 10  WIN32_OWN_PROCESS\n        STATE              : 1  STOPPED\n        WIN32_EXIT_CODE    : 0  (0x0)";
        assert!(parse_sc_query_is_stopped(stopped_status));

        let running_status = "SERVICE_NAME: RadegastRustinel\n        TYPE               : 10  WIN32_OWN_PROCESS\n        STATE              : 4  RUNNING\n        WIN32_EXIT_CODE    : 0  (0x0)";
        assert!(!parse_sc_query_is_stopped(running_status));

        let stop_pending = "SERVICE_NAME: RadegastRustinel\n        TYPE               : 10  WIN32_OWN_PROCESS\n        STATE              : 3  STOP_PENDING\n        WIN32_EXIT_CODE    : 0  (0x0)";
        assert!(!parse_sc_query_is_stopped(stop_pending));
    }

    #[test]
    fn test_parse_sc_query_exists() {
        let not_found = "[SC] OpenService FAILED 1060:\n\nThe specified service does not exist as an installed service.";
        assert!(!parse_sc_query_exists(not_found, false));

        let stopped_status = "SERVICE_NAME: RadegastRustinel\n        TYPE               : 10  WIN32_OWN_PROCESS\n        STATE              : 1  STOPPED";
        assert!(parse_sc_query_exists(stopped_status, true));

        let running_status = "SERVICE_NAME: RadegastRustinel\n        TYPE               : 10  WIN32_OWN_PROCESS\n        STATE              : 4  RUNNING";
        assert!(parse_sc_query_exists(running_status, true));
    }
}
