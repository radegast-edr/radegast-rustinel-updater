# Radegast Rustinel Updater

Secure auto-updater service for the Radegast Rustinel EDR sensor.

## How it works

1. **Fetch manifest**: Downloads the release manifest containing version, SHA256 hashes, and GPG signatures.
2. **Verify GPG**: Verifies the signature of the release entry with the embedded public key.
3. **Download**: Fetches the binary for the detected platform.
4. **Verify SHA256**: Ensures the downloaded archive matches the expected hash.
5. **Replace**: Extracts and replaces the existing binary (or full `.app` bundle on macOS).
6. **Restart**: Automatically restarts the Rustinel service depending on the platform.

## Configuration (Environment Variables)

| Variable | Default | Description |
|----------|---------|-------------|
| `UPDATER_MANIFEST_URL` | `https://radegast.app/api/rustinel-releases.json` | URL to fetch the release manifest from. |
| `UPDATER_CHECK_INTERVAL` | `3600` | Interval in seconds to wait between checks. |
| `UPDATER_DOWNLOAD_URL` | `https://console-api.radegast.app/api/v1` | Base URL for binary downloads. |
| `UPDATER_RUSTINEL_PATH` | Platform-dependent | Path to the Rustinel binary or application bundle. |
| `UPDATER_AUTO_RESTART` | `true` | Whether to automatically restart the service after update. |
| `UPDATER_LOG_LEVEL` | `info` | Logging level. |

