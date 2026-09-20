use anyhow::Result;
use clap::Parser;
use std::time::Duration;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

mod checksum;
mod download;
mod gpg;
mod install;
mod manifest;
mod platform;
mod version;

#[derive(Parser, Debug, PartialEq)]
#[command(
    name = "radegast-rustinel-updater",
    version,
    about = "Secure auto-updater for Radegast Rustinel EDR"
)]
struct Cli {
    /// Run a single update check and exit
    #[arg(long)]
    once: bool,

    /// Same as --once
    #[arg(long)]
    check_now: bool,
}

struct Config {
    manifest_url: String,
    check_interval: Duration,
    download_url: String,
    rustinel_path: String,
    auto_restart: bool,
}

impl Config {
    fn from_env() -> Self {
        Self {
            manifest_url: std::env::var("UPDATER_MANIFEST_URL")
                .unwrap_or_else(|_| "https://radegast.app/api/rustinel-releases.json".into()),
            check_interval: Duration::from_secs(
                std::env::var("UPDATER_CHECK_INTERVAL")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(86400),
            ),
            download_url: std::env::var("UPDATER_DOWNLOAD_URL")
                .unwrap_or_else(|_| "https://console-api.radegast.app/api/v1".into()),
            rustinel_path: std::env::var("UPDATER_RUSTINEL_PATH")
                .unwrap_or_else(|_| platform::default_rustinel_path().into()),
            auto_restart: std::env::var("UPDATER_AUTO_RESTART")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
        }
    }
}

fn update_cycle(config: &Config) -> Result<()> {
    info!("Fetching manifest from {}", config.manifest_url);
    let releases = manifest::fetch(&config.manifest_url)?;

    let current_version_res = version::current_installed(&config.rustinel_path);
    let current_version = match current_version_res {
        Ok(v) => {
            info!("Current installed version: {}", v);
            Some(v)
        }
        Err(e) => {
            warn!(
                "Failed to get current version (perhaps not installed): {}",
                e
            );
            None
        }
    };

    let current_platform = platform::current();
    let latest = manifest::find_latest_for_platform(&releases, &current_platform);

    let (latest_version, latest_entry) = match latest {
        Some(v) => v,
        None => {
            info!("No suitable release found for current platform");
            return Ok(());
        }
    };

    if let Some(ref current) = current_version {
        if &latest_version <= current {
            info!("Already on latest version ({}), no update needed.", current);
            return Ok(());
        }
    }

    info!("New version available: {}", latest_version);

    info!("Verifying GPG signature...");
    gpg::verify_signature(&latest_entry.hash_sha256, &latest_entry.sign_gpg)?;
    info!("GPG signature verified successfully.");

    let archive_path = download::fetch_binary(
        &config.download_url,
        &latest_version.to_manifest_string(),
        &current_platform,
    )?;
    info!("Downloaded archive to {:?}", archive_path);

    info!("Verifying SHA256 checksum...");
    checksum::verify(&archive_path, &latest_entry.hash_sha256, &current_platform)?;
    info!("Checksum verified successfully.");

    // Stop service BEFORE binary replacement to release file locks (critical on Windows)
    if config.auto_restart {
        info!("Stopping Rustinel service before binary replacement...");
        platform::stop_rustinel()?;
    }

    info!("Installing new binary...");
    install::replace_binary(&archive_path, &config.rustinel_path)?;
    info!("Installation successful.");

    // Start service AFTER binary replacement
    if config.auto_restart {
        info!("Starting Rustinel service...");
        platform::start_rustinel()?;
    }

    Ok(())
}

fn main() -> Result<()> {
    let default_level = std::env::var("UPDATER_LOG_LEVEL")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| "info".into());

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();
    let config = Config::from_env();

    if cli.once || cli.check_now {
        if let Err(e) = update_cycle(&config) {
            error!("Update failed: {:?}", e);
            std::process::exit(1);
        }
        return Ok(());
    }

    info!(
        "Starting auto-updater daemon (check interval: {}s / {:.1}h)...",
        config.check_interval.as_secs(),
        config.check_interval.as_secs_f64() / 3600.0
    );

    // 1. Run immediate update check during startup
    info!("Running initial update check on startup...");
    if let Err(e) = update_cycle(&config) {
        error!("Initial startup update check failed: {:?}", e);
    } else {
        info!("Initial startup update check completed successfully.");
    }
    info!(
        "Next update check scheduled in {} seconds ({:.1} hours).",
        config.check_interval.as_secs(),
        config.check_interval.as_secs_f64() / 3600.0
    );

    // 2. Periodic loop: wait 1 day (check_interval), then run update check
    loop {
        std::thread::sleep(config.check_interval);

        info!("Running scheduled periodic update check...");
        if let Err(e) = update_cycle(&config) {
            error!("Scheduled update check failed: {:?}", e);
        } else {
            info!("Scheduled update check completed successfully.");
        }
        info!(
            "Next update check scheduled in {} seconds ({:.1} hours).",
            config.check_interval.as_secs(),
            config.check_interval.as_secs_f64() / 3600.0
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        let cli_default = Cli::try_parse_from(["radegast-rustinel-updater"]).unwrap();
        assert!(!cli_default.once);
        assert!(!cli_default.check_now);

        let cli_once = Cli::try_parse_from(["radegast-rustinel-updater", "--once"]).unwrap();
        assert!(cli_once.once);
        assert!(!cli_once.check_now);

        let cli_check_now =
            Cli::try_parse_from(["radegast-rustinel-updater", "--check-now"]).unwrap();
        assert!(!cli_check_now.once);
        assert!(cli_check_now.check_now);

        let err_version =
            Cli::try_parse_from(["radegast-rustinel-updater", "--version"]).unwrap_err();
        assert_eq!(err_version.kind(), clap::error::ErrorKind::DisplayVersion);
        assert!(err_version.to_string().contains(env!("CARGO_PKG_VERSION")));

        let err_v = Cli::try_parse_from(["radegast-rustinel-updater", "-V"]).unwrap_err();
        assert_eq!(err_v.kind(), clap::error::ErrorKind::DisplayVersion);
        assert!(err_v.to_string().contains(env!("CARGO_PKG_VERSION")));

        assert!(Cli::try_parse_from(["radegast-rustinel-updater", "--nonexistent"]).is_err());
    }

    #[test]
    fn test_config_from_env_defaults() {
        std::env::remove_var("UPDATER_MANIFEST_URL");
        std::env::remove_var("UPDATER_CHECK_INTERVAL");
        std::env::remove_var("UPDATER_DOWNLOAD_URL");
        std::env::remove_var("UPDATER_RUSTINEL_PATH");
        std::env::remove_var("UPDATER_AUTO_RESTART");

        let cfg = Config::from_env();
        assert_eq!(
            cfg.manifest_url,
            "https://radegast.app/api/rustinel-releases.json"
        );
        assert_eq!(cfg.check_interval, Duration::from_secs(86400));
        assert_eq!(cfg.download_url, "https://console-api.radegast.app/api/v1");
        assert!(cfg.auto_restart);
        assert!(!cfg.rustinel_path.is_empty());
    }

    #[test]
    fn test_config_from_env_overrides() {
        std::env::set_var(
            "UPDATER_MANIFEST_URL",
            "https://custom.example.com/releases.json",
        );
        std::env::set_var("UPDATER_CHECK_INTERVAL", "3600");
        std::env::set_var("UPDATER_DOWNLOAD_URL", "https://download.example.com");
        std::env::set_var("UPDATER_RUSTINEL_PATH", "/custom/path/to/rustinel");
        std::env::set_var("UPDATER_AUTO_RESTART", "false");

        let cfg = Config::from_env();
        assert_eq!(cfg.manifest_url, "https://custom.example.com/releases.json");
        assert_eq!(cfg.check_interval, Duration::from_secs(3600));
        assert_eq!(cfg.download_url, "https://download.example.com");
        assert_eq!(cfg.rustinel_path, "/custom/path/to/rustinel");
        assert!(!cfg.auto_restart);

        std::env::set_var("UPDATER_AUTO_RESTART", "0");
        let cfg2 = Config::from_env();
        assert!(!cfg2.auto_restart);

        std::env::set_var("UPDATER_AUTO_RESTART", "true");
        assert!(Config::from_env().auto_restart);
        std::env::set_var("UPDATER_AUTO_RESTART", "1");
        assert!(Config::from_env().auto_restart);

        std::env::remove_var("UPDATER_MANIFEST_URL");
        std::env::remove_var("UPDATER_CHECK_INTERVAL");
        std::env::remove_var("UPDATER_DOWNLOAD_URL");
        std::env::remove_var("UPDATER_RUSTINEL_PATH");
        std::env::remove_var("UPDATER_AUTO_RESTART");
    }
}
