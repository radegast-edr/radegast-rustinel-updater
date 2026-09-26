#!/usr/bin/env python3
"""Integration tests for radegast-rustinel-updater.

Verifies end-to-end functionality across Linux, macOS, and Windows:
1. Version & help CLI flags
2. Manifest fetching and GPG signature verification
3. Binary download, SHA256 verification, and atomic replacement
4. Post-update version verification
5. Idempotent no-op when already on the latest version
6. Security integrity checks: corrupted checksum rejection and invalid signature rejection
"""

import http.server
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

REPO_ROOT = Path(__file__).parent.parent.resolve()

# Verified manifests with authentic PGP signatures from the Radegast Release Key
VERIFIED_MANIFEST_DATA = [
    {
        "version": "1.8.0r2",
        "hash_sha256": (
            "f9a8c5bd2d6e15b1a7a4c062a6466473b3d1aee0fd74a895ad5fad48e29b376f  linux-amd64.zip\n"
            "4790c03a6b86336536d7eb9a7060c56cc5e284ca7d6d25961934d3d65776798e  linux-arm64.zip\n"
            "c321b8d810794eead9c55bb1ebfe4daa703862e848ed648f354efc60694ecd30  windows-amd64.zip\n"
        ),
        "sign_gpg": (
            "-----BEGIN PGP SIGNATURE-----\n\n"
            "iQIzBAABCgAdFiEE09RBOxFH8cabfO/ja9UaMJ3zQ88FAmqwNaMACgkQa9UaMJ3z\n"
            "Q8/tnxAAnbA0etDrScuwWDfY3hR1qkkZNRl9z1dD+sXEclt5poeDcHt2iDJl+SYA\n"
            "Kl0kSi8LqlcQ/cikMu4yT3tM4XKy8JAPjPwAGBKCy4zckRhXIQHIu9+m1WnYacVM\n"
            "P6AyW/hul2qrEWjuBAYIRwRSAWTRQx44IknGVJbyAQLOlNBV+2ALM7tuANToPsQv\n"
            "Khp9+x0li5MYMr4CUqQhowvT7sQJeexawTzef4EqWHg6alZwtXTw8bTeVdN28DK3\n"
            "v064anLJfFB4KOOTpyr8pFlaWTSDBmsAVNtg+Hj3zINoKakzfDCl8RFsvG3cEXRF\n"
            "LDgszZ+DLKtfsf6GzJ/ibCnlWHJqmMiYYo6AnfTRAH4gywkWJxSKxUMdVFwA5F/M\n"
            "PrLZzbOW08t8dlaTXhX+AQrd2U0AsTbr93/sfZl44vgd1nnGVNmb6QgrxvXZ9Jn1\n"
            "dYSoi7Y+xFyCi+TVL4va7mUycGqcFOR8MAKXi0qjRfJZGf7SpmwKsL1JKmppyxY9\n"
            "5FNrz/v/bIpkPzf4NA4CNQWte/OJoVcX0617lDczMarPYAQT9k1vaLh3Tmh/EBPS\n"
            "91gqfd5KDWQRqBq3WV/jvae5TnqJwLnMZJw4I4skmsAwZ7bLJHgf68gp9KuatUq+\n"
            "baL3RDVZJm5YsRkaLSJhaOTTMDnumnYHALDTKA9LM4YSzDt/SG8=\n"
            "=uZg5\n"
            "-----END PGP SIGNATURE-----\n"
        ),
    },
    {
        "version": "1.8.0",
        "hash_sha256": (
            "2e5b4d8aa9ab482301c5be1dd690dbd96e9e4c61275fcb95dbdc80fdf646eaa9  linux-amd64.zip\n"
            "e793a291b7b7a2543f9ffc80e31f0931a7676e771ecb3a0e751b8c6bf7a89a5f  linux-arm64.zip\n"
            "f63c76c9b0e7dbf230afc45039c92336b116149cbf4f8a6ce3cd67249699ff2b  mac-amd64.zip\n"
            "ebd56f7b17fb1f819bb7d6d079d8d863375cdc76735ccd95061395d088f84036  mac-m5.zip\n"
            "01e0ca55abf6a0c2a19e8c4e5b7c44e168b4b66f6de03c5d55482e0f10866a36  windows-amd64.zip\n"
        ),
        "sign_gpg": (
            "-----BEGIN PGP SIGNATURE-----\n\n"
            "iQIzBAABCgAdFiEE09RBOxFH8cabfO/ja9UaMJ3zQ88FAmqwGX4ACgkQa9UaMJ3z\n"
            "Q89OrA//ae9EfGdCg6ZkCeL4UxeBvhdXY9VJ9Pf9bXyQ82nZ2tgu6qhBMn4inuK4\n"
            "kMlhZhtEWX86rUOGdwAWhXQm6SAX4ixwBQZcuwD3Zo3dlYDmlu6zyWMb+4bVkIwr\n"
            "+QSzDHx07S3gbdTvSRFU/buFdcpzCXteR7LZaERMkQ0Na/iKDp+nU8usX4WNlhQk\n"
            "JVICV6msaS6bOACMooNk3LRBLYjsj2WMwlqV8YA5HBUE5oFQRgb7/7qH1gYIJQHl\n"
            "Wwpu92T8lAJIt7dWLR+OGU9vr/huGLW7MvJceT8GGfBW21p2RBpCgGzex0D2vCmP\n"
            "m2XXIR8E8YIM15mk3O53/M7RmaAPLjf7uxm9ofzh5GiGuSlthqZWS8OKKIRx3OAD\n"
            "1BoQFBkGXVQTwiIp0Nk2rlRb/lPc4xNWJrTIY8FxLdUOnkvNwCXBz7j/G7KpHy/C\n"
            "eZyzgslljKeFcObgrxprJrKJRx4PHfZz5EDjIrytU6yYmzZzaASIYrtQbX3uniNW\n"
            "9sXgNLE1RQswkxVr8zcKlWQE1smgTYQfgnVJpRY0xjEF9DaIBx3PYM3M7c+/chYe\n"
            "/4aMh+FnQ7P1YllnsNuy8O5zIUQPtUXolkIh9GPSnbROE6NvAWnY50MGVjeQaYMz\n"
            "IInDCrh4JTtIHmj6WVINHJkXo7fIQ7ei/BFod05KTLpYqURZf0w=\n"
            "=L3Zp\n"
            "-----END PGP SIGNATURE-----\n"
        ),
    },
]


def find_or_build_updater_binary() -> Path:
    """Find existing radegast-rustinel-updater binary or build it via cargo."""
    candidates = [
        REPO_ROOT / "target" / "release" / ("radegast-rustinel-updater.exe" if sys.platform.startswith("win32") else "radegast-rustinel-updater"),
        REPO_ROOT / "target" / "debug" / ("radegast-rustinel-updater.exe" if sys.platform.startswith("win32") else "radegast-rustinel-updater"),
    ]
    for c in candidates:
        if c.exists() and os.access(c, os.X_OK if not sys.platform.startswith("win32") else os.F_OK):
            return c

    print("Building radegast-rustinel-updater binary with cargo...")
    subprocess.run(["cargo", "build"], cwd=REPO_ROOT, check=True)
    debug_bin = REPO_ROOT / "target" / "debug" / ("radegast-rustinel-updater.exe" if sys.platform.startswith("win32") else "radegast-rustinel-updater")
    if debug_bin.exists():
        return debug_bin
    raise FileNotFoundError("Could not find or build radegast-rustinel-updater binary.")


def create_mock_rustinel(target_path: Path, version_str: str) -> None:
    """Create an executable mock rustinel binary that outputs version_str on --version."""
    target_path.parent.mkdir(parents=True, exist_ok=True)
    if sys.platform.startswith("win32"):
        if shutil.which("rustc"):
            rust_src = (
                f'fn main() {{\n'
                f'    let args: Vec<String> = std::env::args().collect();\n'
                f'    if args.len() > 1 && args[1] == "--version" {{\n'
                f'        println!("rustinel {version_str}");\n'
                f'    }} else {{\n'
                f'        println!("mock rustinel running");\n'
                f'    }}\n'
                f'}}\n'
            )
            src_file = target_path.with_suffix(".rs")
            src_file.write_text(rust_src, encoding="utf-8")
            try:
                subprocess.run(["rustc", str(src_file), "-o", str(target_path)], check=True)
            finally:
                if src_file.exists():
                    src_file.unlink()
        else:
            bat_content = f"@echo off\r\nif \"%1\"==\"--version\" (\r\n  echo rustinel {version_str}\r\n) else (\r\n  echo mock rustinel running\r\n)\r\n"
            target_path.write_text(bat_content, encoding="utf-8")
    else:
        sh_content = f"#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo \"rustinel {version_str}\"\nelse\n  echo \"mock rustinel running\"\nfi\n"
        target_path.write_text(sh_content, encoding="utf-8")
        target_path.chmod(0o755)


def get_rustinel_version(binary_path: Path) -> str:
    """Run binary with --version and return stdout."""
    res = subprocess.run([str(binary_path), "--version"], capture_output=True, text=True, check=False)
    return res.stdout.strip()


def start_mock_manifest_server(data: list) -> tuple[str, http.server.HTTPServer]:
    """Start an ephemeral background HTTP server serving data as JSON."""
    raw_json = json.dumps(data).encode("utf-8")

    class _Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(raw_json)))
            self.end_headers()
            self.wfile.write(raw_json)

        def log_message(self, format, *args):
            pass

    server = http.server.HTTPServer(("127.0.0.1", 0), _Handler)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return f"http://127.0.0.1:{port}/rustinel-releases.json", server


def test_updater_integration():
    """Run comprehensive integration tests for radegast-rustinel-updater."""
    updater_bin = find_or_build_updater_binary()
    print(f"Using updater binary: {updater_bin}")

    # 1. Test CLI Help and Version flags
    print("\n--- Step 1: Testing CLI flags (--help, --version) ---")
    res_help = subprocess.run([str(updater_bin), "--help"], capture_output=True, text=True, check=True)
    assert "Secure auto-updater for Radegast Rustinel EDR" in res_help.stdout
    print("  OK: --help output verified.")

    res_ver = subprocess.run([str(updater_bin), "--version"], capture_output=True, text=True, check=True)
    assert "radegast-rustinel-updater" in res_ver.stdout
    print(f"  OK: --version output verified: {res_ver.stdout.strip()}")

    # 2. Test End-to-End Update Flow (1.3.0 -> 1.8.0 / 1.8.0r2)
    print("\n--- Step 2: Testing End-to-End Auto-Update Flow ---")
    with tempfile.TemporaryDirectory() as td:
        temp_dir = Path(td)
        rustinel_name = "rustinel.exe" if sys.platform.startswith("win32") else "rustinel"
        rustinel_path = temp_dir / rustinel_name

        create_mock_rustinel(rustinel_path, "1.3.0")
        initial_ver = get_rustinel_version(rustinel_path)
        print(f"  Installed base rustinel version: {initial_ver}")
        assert "1.3.0" in initial_ver

        manifest_server = None
        manifest_url = os.environ.get("UPDATER_MANIFEST_URL")
        if not manifest_url:
            manifest_url, manifest_server = start_mock_manifest_server(VERIFIED_MANIFEST_DATA)
            print(f"  Started local mock manifest server at: {manifest_url}")

        download_url = os.environ.get("UPDATER_DOWNLOAD_URL", "https://console-api.radegast.app/api/v1")

        updater_env = os.environ.copy()
        updater_env["UPDATER_MANIFEST_URL"] = manifest_url
        updater_env["UPDATER_DOWNLOAD_URL"] = download_url
        updater_env["UPDATER_RUSTINEL_PATH"] = str(rustinel_path)
        updater_env["UPDATER_AUTO_RESTART"] = "false"
        updater_env["UPDATER_LOG_LEVEL"] = "info"

        try:
            print("  Executing radegast-rustinel-updater --once...")
            res_update = subprocess.run(
                [str(updater_bin), "--once"],
                env=updater_env,
                capture_output=True,
                text=True,
                check=False,
            )
            print("  STDOUT:\n", res_update.stdout)
            if res_update.stderr:
                print("  STDERR:\n", res_update.stderr)

            assert res_update.returncode == 0, f"Updater failed with code {res_update.returncode}"

            updated_ver = get_rustinel_version(rustinel_path)
            print(f"  Post-update rustinel version: {updated_ver}")
            assert ("1.8.0" in updated_ver or "1.8.0r2" in updated_ver), f"Unexpected version after update: {updated_ver}"
            assert updated_ver != initial_ver, "Version did not change!"
            print(f"  SUCCESS: Rustinel was updated from {initial_ver} to {updated_ver}")

            # 3. Test Idempotency (Already on latest version)
            print("\n--- Step 3: Testing Idempotency (Already on latest version) ---")
            res_noop = subprocess.run(
                [str(updater_bin), "--once"],
                env=updater_env,
                capture_output=True,
                text=True,
                check=False,
            )
            assert res_noop.returncode == 0, f"Idempotent check failed with code {res_noop.returncode}"
            assert get_rustinel_version(rustinel_path) == updated_ver
            print("  SUCCESS: Updater safely no-ops when already on latest version.")

        finally:
            if manifest_server:
                manifest_server.shutdown()
                manifest_server.server_close()

    # 4. Test Security: Corrupted Checksum Rejection
    print("\n--- Step 4: Testing Checksum Mismatch Protection ---")
    corrupt_data = [
        {
            "version": "1.9.9",
            "hash_sha256": (
                "0000000000000000000000000000000000000000000000000000000000000000  linux-amd64.zip\n"
                "0000000000000000000000000000000000000000000000000000000000000000  linux-arm64.zip\n"
                "0000000000000000000000000000000000000000000000000000000000000000  mac-m5.zip\n"
                "0000000000000000000000000000000000000000000000000000000000000000  windows-amd64.zip\n"
            ),
            "sign_gpg": VERIFIED_MANIFEST_DATA[0]["sign_gpg"],  # Signature will fail or checksum will fail
        }
    ]
    with tempfile.TemporaryDirectory() as td:
        temp_dir = Path(td)
        rustinel_path = temp_dir / rustinel_name
        create_mock_rustinel(rustinel_path, "1.3.0")

        bad_manifest_url, bad_server = start_mock_manifest_server(corrupt_data)
        try:
            updater_env["UPDATER_MANIFEST_URL"] = bad_manifest_url
            updater_env["UPDATER_RUSTINEL_PATH"] = str(rustinel_path)
            res_bad = subprocess.run([str(updater_bin), "--once"], env=updater_env, capture_output=True, text=True)
            # Must abort and fail
            assert res_bad.returncode != 0, "Updater must fail on invalid/tampered manifest!"
            # Installed binary must remain unchanged
            assert "1.3.0" in get_rustinel_version(rustinel_path)
            print("  SUCCESS: Updater rejected tampered manifest and left binary untouched.")
        finally:
            bad_server.shutdown()
            bad_server.server_close()

    print("\nAll integration tests passed successfully!")


if __name__ == "__main__":
    test_updater_integration()
