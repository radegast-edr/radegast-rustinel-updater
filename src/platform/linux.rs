use anyhow::{ensure, Context, Result};
use std::process::Command;

fn run_service_command(mut command: Command) -> Result<()> {
    let status = command.status().context("Failed to execute systemctl")?;
    ensure!(status.success(), "systemctl failed with status {status}");
    Ok(())
}

fn service_command(action: &str) -> Command {
    let mut command = Command::new("systemctl");
    command.args([action, "rustinel"]);
    command
}

pub fn stop_service() -> Result<()> {
    tracing::info!("Stopping rustinel service...");
    run_service_command(service_command("stop"))
}

pub fn start_service() -> Result<()> {
    tracing::info!("Starting rustinel service...");
    run_service_command(service_command("start"))
}

#[cfg(test)]
pub fn test_service_commands() {
    // Exercise status handling without touching services on the test host.
    assert!(run_service_command(Command::new("/bin/true")).is_ok());
    assert!(run_service_command(Command::new("/bin/false")).is_err());
    assert!(run_service_command(Command::new("/nonexistent/systemctl")).is_err());
    for action in ["start", "stop"] {
        let command = service_command(action);
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, [action, "rustinel"]);
    }
}
