# Radegast Rustinel Updater

Secure auto-updater service for the Radegast Rustinel EDR sensor.

## How it works

1. **Fetch manifest**: Downloads the release manifest containing version, SHA256 hashes, and GPG signatures.
2. **Verify GPG**: Verifies the signature of the release entry with the embedded public key.
3. **Download**: Fetches the binary for the detected platform.
4. **Verify SHA256**: Ensures the downloaded archive matches the expected hash.
5. **Stop service**: Stops the Rustinel service before replacing the binary (critical on Windows where the .exe is locked while running).
6. **Replace**: Extracts and replaces the existing binary (or full `.app` bundle on macOS).
7. **Start service**: Starts the Rustinel service after successful replacement.

## Configuration (Environment Variables)

| Variable | Default | Description |
|----------|---------|-------------|
| `UPDATER_MANIFEST_URL` | `https://radegast.app/api/rustinel-releases.json` | URL to fetch the release manifest from. |
| `UPDATER_CHECK_INTERVAL` | `86400` | Interval in seconds between update checks (default: 24 hours). |
| `UPDATER_INITIAL_RETRY_INTERVAL` | `20` | Interval in seconds to re-try fetching the manifest on startup until first success (e.g. waiting for Wi-Fi). |
| `UPDATER_DOWNLOAD_URL` | `https://console-api.radegast.app/api/v1` | Base URL for binary downloads. |
| `UPDATER_RUSTINEL_PATH` | Platform-dependent | Path to the Rustinel binary or application bundle. |
| `UPDATER_AUTO_RESTART` | `true` | Whether to stop/start the service around binary replacement. |
| `UPDATER_LOG_LEVEL` | `info` | Logging level (via `RUST_LOG`). |

## CLI Options

| Flag | Description |
|------|-------------|
| `--version`, `-V` | Print binary name and version, then exit. |
| `--once`, `--check-now` | Run a single update check and exit instead of running daemon loop. |
| `--help`, `-h` | Print help information. |

### Default Paths

| Platform | Default `UPDATER_RUSTINEL_PATH` |
|----------|--------------------------------|
| Linux | `/opt/radegast/rustinel/rustinel` |
| macOS | `/Library/Radegast/rustinel/rustinel` |
| Windows | `C:\Program Files\Radegast\rustinel\rustinel\rustinel.exe` |

## Installation

The updater binary is bundled inside `rustinel.zip` alongside the sensor binary. When deployed via the Radegast install scripts, a dedicated service is automatically created:

- **Linux**: `rustinel-updater.service` (systemd)
- **macOS**: `app.radegast.rustinel-updater.plist` (launchd)
- **Windows**: `RadegastUpdater` (WinSW service)

The service can be disabled during installation by passing `rustinel-autoupdate=false` to the install endpoint.

## Security

- GPG public key embedded at compile time (`pub.pgp.asc`)
- Hardened systemd unit with `ProtectSystem=strict`, `NoNewPrivileges=true`, etc.
- All downloads via HTTPS only (`reqwest` with `rustls`)
- Binary replacement uses atomic rename (temp file in same directory)
- macOS: code signature verification (`codesign --verify --deep --strict`) before bundle replacement
