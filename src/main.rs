use anyhow::Result;
use clap::Parser;
use std::time::Duration;
use tracing::{info, error, warn};
use tracing_subscriber::EnvFilter;

mod manifest;
mod gpg;
mod checksum;
mod download;
mod install;
mod platform;
mod version;

#[derive(Parser)]
#[command(name = "radegast-rustinel-updater", about = "Secure auto-updater for Radegast Rustinel EDR")]
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
            warn!("Failed to get current version (perhaps not installed): {}", e);
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
    
    let archive_path = download::fetch_binary(&config.download_url, &latest_version.to_manifest_string(), &current_platform)?;
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

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

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
