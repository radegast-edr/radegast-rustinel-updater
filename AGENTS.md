# Agent Instructions for radegast-rustinel-updater (AGENTS.md)

Welcome, AI Agent! This document outlines the architecture, toolchain conventions, testing workflows, CI/CD pipeline, and safety boundaries for [`radegast-rustinel-updater`](file:///home/adam/Projekty/radegast/radegast-rustinel-updater).

---

## 1. Project Overview

`radegast-rustinel-updater` is the official auto-updater daemon for the [Radegast Rustinel EDR](https://github.com/radegast-edr/rustinel) endpoint sensor.

Its primary responsibilities are:
1. **Manifest Retrieval**: Fetches release metadata (`version`, `hash_sha256`, and `sign_gpg`) from the release server.
2. **Cryptographic Verification**: Verifies the manifest's GPG signature against the embedded compile-time public key ([`pub.pgp.asc`](file:///home/adam/Projekty/radegast/radegast-rustinel-updater/pub.pgp.asc)) using Sequoia OpenPGP.
3. **Archive Download & Integrity**: Downloads the platform-specific archive and verifies its SHA256 checksum.
4. **Service Coordination**: Stops the Rustinel sensor service *before* binary replacement (crucial on Windows to release `.exe` file locks), replaces the binary or application bundle via atomic rename, and restarts the service.
5. **Daemon & Scheduling**: Performs an immediate startup update check, followed by a periodic 24-hour daemon check interval (configurable).

---

## 2. Tech Stack

- **Language**: Rust 2021 Edition (Rust 1.82+)
- **CLI Parsing**: `clap` with `derive` and `version` features
- **HTTP Client**: `reqwest` with pure-Rust `rustls-tls-native-roots` (blocking API)
- **GPG Signature Verification**: `sequoia-openpgp` (pure-Rust crypto, no host `gpg` dependency required at runtime)
- **Hashing & Crypto**: `sha2`, `hex`
- **Archiving & Filesystem**: `zip`, `tempfile`
- **Logging**: `tracing` and `tracing-subscriber` with `EnvFilter` (default level: `info`)
- **Version Handling**: `semver` (custom `RadegastVersion` parsing upstream `X.Y.Z` and patched `X.Y.ZrN`)

---

## 3. Environment & Commands

Always run these commands from the repository root ([`/home/adam/Projekty/radegast/radegast-rustinel-updater`](file:///home/adam/Projekty/radegast/radegast-rustinel-updater)):

### Run Tests
```bash
# Run all unit and integration tests
cargo test

# Run tests with verbose output (prints stdout)
cargo test -- --nocapture
```

### Code Quality & Linter
```bash
# Check code with Clippy (zero warnings policy)
cargo clippy -- -D warnings

# Check code formatting
cargo fmt --check

# Format code automatically
cargo fmt
```

### Build Binaries
```bash
# Debug build (fast compilation)
cargo build

# Optimized release build (Fat LTO + stripped binary)
cargo build --release
```

### Run Updater Locally
```bash
# Print updater version
./target/release/radegast-rustinel-updater --version
./target/release/radegast-rustinel-updater -V

# Run a single update check without starting daemon loop
./target/release/radegast-rustinel-updater --once
```

---

## 4. Required Post-Task Validation Checklist

After making any changes to this codebase, **always** execute the following validations in order:

1. **Format Check**:
   ```bash
   cargo fmt --check
   ```
2. **Clippy Linter**:
   ```bash
   cargo clippy -- -D warnings
   ```
   Fix any linter errors or warnings.
3. **Unit Tests**:
   ```bash
   cargo test
   ```
   Ensure all 28+ unit tests pass cleanly. Never disable or delete tests without explicit approval.
4. **Coverage Maintenance**:
   If a new feature, CLI flag, or platform handler is added, add corresponding unit tests in the module's `tests` submodule.

---

## 5. Repository Architecture

```
radegast-rustinel-updater/
├── .github/
│   └── workflows/
│       └── release.yml          # Automated CI/CD release pipeline
├── src/
│   ├── main.rs                  # Entry point, CLI args, Config, daemon loop, update cycle
│   ├── manifest.rs              # Fetches & parses release manifest JSON, version resolution
│   ├── gpg.rs                   # Pure-Rust PGP signature verification with pub.pgp.asc
│   ├── checksum.rs              # SHA256 checksum parsing and file hash validation
│   ├── download.rs              # HTTPS archive download into temporary storage
│   ├── install.rs               # Atomic binary replacement, archive extraction, macOS codesign verify
│   ├── version.rs               # RadegastVersion struct & ordering (e.g. 1.7.0 vs 1.7.0r1)
│   └── platform/
│       ├── mod.rs               # Platform struct, OS/Arch detection & archive naming
│       ├── linux.rs             # Linux systemctl service controls (rustinel.service)
│       ├── macos.rs             # macOS launchctl service controls (io.rustinel.daemon)
│       └── windows.rs           # Windows sc.exe service controls with polling loop
├── pub.pgp.asc                  # Embedded ASCII-armored PGP public key
├── rustinel-updater.service     # Systemd unit template for Linux
├── app.radegast.rustinel-updater.plist # Launchd plist template for macOS
├── Cargo.toml                   # Project dependencies and release profile (LTO fat, strip)
└── README.md                    # User documentation and environment variable reference
```

---

## 6. Important Design Constraints & Rules

1. **Service Stop Before Replace**:
   Never attempt to replace the Rustinel executable while its service is actively running. On Windows, executing binaries are locked by the OS kernel, and on Unix systems, in-flight replacement can crash running sensors. Always call `platform::stop_rustinel()` before `install::replace_binary()`, and `platform::start_rustinel()` afterward.

2. **Supply Chain Security**:
   - In GitHub Actions ([`.github/workflows/release.yml`](file:///home/adam/Projekty/radegast/radegast-rustinel-updater/.github/workflows/release.yml)), **100% of actions must be pinned to immutable 40-character commit SHA hashes** (no `@v4` or branch tags).
   - Detached PGP signatures (`checksums-sha256.txt.asc`) are mandatory on releases and must be verified locally against [`pub.pgp.asc`](file:///home/adam/Projekty/radegast/radegast-rustinel-updater/pub.pgp.asc) before publishing.
   - Build provenance attestations must be generated for all release assets via `actions/attest-build-provenance`.

3. **Release Tagging on `main`**:
   The release workflow triggers on push to `main` and checks if `package.version` in `Cargo.toml` has been bumped. If so, it creates git tag `v<version>`, pushes it, and publishes the multi-platform release. When releasing a new version, simply update `version = "X.Y.Z"` in `Cargo.toml`.

4. **Environment Variables**:
   Any new configuration setting must be:
   - Added to `Config` in [`src/main.rs`](file:///home/adam/Projekty/radegast/radegast-rustinel-updater/src/main.rs).
   - Given a sensible, secure default value.
   - Covered by unit tests in `tests::test_config_from_env_defaults` and `tests::test_config_from_env_overrides`.
   - Documented in [`README.md`](file:///home/adam/Projekty/radegast/radegast-rustinel-updater/README.md).
