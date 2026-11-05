//! Verify real integration fixtures using the updater's production trust code.
use anyhow::{Context, Result};

#[path = "../src/checksum.rs"]
mod checksum;
#[path = "../src/gpg.rs"]
mod gpg;

mod platform {
    pub struct Platform {
        pub os: &'static str,
        pub arch: &'static str,
    }
    impl Platform {
        pub fn archive_name(&self) -> String {
            format!("{}-{}.zip", self.os, self.arch)
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 5,
        "Usage: verify_release ENTRY.json ARCHIVE.zip OS ARCH"
    );
    let entry: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let checksums = entry["hash_sha256"].as_str().context("Missing checksums")?;
    let signature = entry["sign_gpg"].as_str().context("Missing signature")?;
    gpg::verify_signature(checksums, signature)?;
    let os = match args[3].as_str() {
        "linux" => "linux",
        "windows" => "windows",
        "mac" => "mac",
        _ => anyhow::bail!("Unsupported OS"),
    };
    let arch = match args[4].as_str() {
        "amd64" => "amd64",
        "arm64" => "arm64",
        "m5" => "m5",
        _ => anyhow::bail!("Unsupported architecture"),
    };
    checksum::verify(
        std::path::Path::new(&args[2]),
        checksums,
        &platform::Platform { os, arch },
    )
}
