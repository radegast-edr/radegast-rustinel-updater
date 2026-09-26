use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

const MOCK_MANIFEST: &str = r#"[{"version":"1.8.0r2","hash_sha256":"f9a8c5bd2d6e15b1a7a4c062a6466473b3d1aee0fd74a895ad5fad48e29b376f  linux-amd64.zip\n4790c03a6b86336536d7eb9a7060c56cc5e284ca7d6d25961934d3d65776798e  linux-arm64.zip\nc321b8d810794eead9c55bb1ebfe4daa703862e848ed648f354efc60694ecd30  windows-amd64.zip\n","sign_gpg":"-----BEGIN PGP SIGNATURE-----\n\niQIzBAABCgAdFiEE09RBOxFH8cabfO/ja9UaMJ3zQ88FAmqwNaMACgkQa9UaMJ3z\nQ8/tnxAAnbA0etDrScuwWDfY3hR1qkkZNRl9z1dD+sXEclt5poeDcHt2iDJl+SYA\nKl0kSi8LqlcQ/cikMu4yT3tM4XKy8JAPjPwAGBKCy4zckRhXIQHIu9+m1WnYacVM\nP6AyW/hul2qrEWjuBAYIRwRSAWTRQx44IknGVJbyAQLOlNBV+2ALM7tuANToPsQv\nKhp9+x0li5MYMr4CUqQhowvT7sQJeexawTzef4EqWHg6alZwtXTw8bTeVdN28DK3\nv064anLJfFB4KOOTpyr8pFlaWTSDBmsAVNtg+Hj3zINoKakzfDCl8RFsvG3cEXRF\nLDgszZ+DLKtfsf6GzJ/ibCnlWHJqmMiYYo6AnfTRAH4gywkWJxSKxUMdVFwA5F/M\nPrLZzbOW08t8dlaTXhX+AQrd2U0AsTbr93/sfZl44vgd1nnGVNmb6QgrxvXZ9Jn1\ndYSoi7Y+xFyCi+TVL4va7mUycGqcFOR8MAKXi0qjRfJZGf7SpmwKsL1JKmppyxY9\n5FNrz/v/bIpkPzf4NA4CNQWte/OJoVcX0617lDczMarPYAQT9k1vaLh3Tmh/EBPS\n91gqfd5KDWQRqBq3WV/jvae5TnqJwLnMZJw4I4skmsAwZ7bLJHgf68gp9KuatUq+\nbaL3RDVZJm5YsRkaLSJhaOTTMDnumnYHALDTKA9LM4YSzDt/SG8=\n=uZg5\n-----END PGP SIGNATURE-----\n"},{"version":"1.8.0","hash_sha256":"2e5b4d8aa9ab482301c5be1dd690dbd96e9e4c61275fcb95dbdc80fdf646eaa9  linux-amd64.zip\ne793a291b7b7a2543f9ffc80e31f0931a7676e771ecb3a0e751b8c6bf7a89a5f  linux-arm64.zip\nf63c76c9b0e7dbf230afc45039c92336b116149cbf4f8a6ce3cd67249699ff2b  mac-amd64.zip\nebd56f7b17fb1f819bb7d6d079d8d863375cdc76735ccd95061395d088f84036  mac-m5.zip\n01e0ca55abf6a0c2a19e8c4e5b7c44e168b4b66f6de03c5d55482e0f10866a36  windows-amd64.zip\n","sign_gpg":"-----BEGIN PGP SIGNATURE-----\n\niQIzBAABCgAdFiEE09RBOxFH8cabfO/ja9UaMJ3zQ88FAmqwGX4ACgkQa9UaMJ3z\nQ89OrA//ae9EfGdCg6ZkCeL4UxeBvhdXY9VJ9Pf9bXyQ82nZ2tgu6qhBMn4inuK4\nkMlhZhtEWX86rUOGdwAWhXQm6SAX4ixwBQZcuwD3Zo3dlYDmlu6zyWMb+4bVkIwr\n+QSzDHx07S3gbdTvSRFU/buFdcpzCXteR7LZaERMkQ0Na/iKDp+nU8usX4WNlhQk\nJVICV6msaS6bOACMooNk3LRBLYjsj2WMwlqV8YA5HBUE5oFQRgb7/7qH1gYIJQHl\nWwpu92T8lAJIt7dWLR+OGU9vr/huGLW7MvJceT8GGfBW21p2RBpCgGzex0D2vCmP\nm2XXIR8E8YIM15mk3O53/M7RmaAPLjf7uxm9ofzh5GiGuSlthqZWS8OKKIRx3OAD\n1BoQFBkGXVQTwiIp0Nk2rlRb/lPc4xNWJrTIY8FxLdUOnkvNwCXBz7j/G7KpHy/C\neZyzgslljKeFcObgrxprJrKJRx4PHfZz5EDjIrytU6yYmzZzaASIYrtQbX3uniNW\n9sXgNLE1RQswkxVr8zcKlWQE1smgTYQfgnVJpRY0xjEF9DaIBx3PYM3M7c+/chYe\n/4aMh+FnQ7P1YllnsNuy8O5zIUQPtUXolkIh9GPSnbROE6NvAWnY50MGVjeQaYMz\nIInDCrh4JTtIHmj6WVINHJkXo7fIQ7ei/BFod05KTLpYqURZf0w=\n=L3Zp\n-----END PGP SIGNATURE-----\n"}]"#;

fn create_mock_rustinel(dir: &Path, version: &str) -> PathBuf {
    let file_name = if cfg!(windows) {
        "rustinel.exe"
    } else {
        "rustinel"
    };
    let path = dir.join(file_name);

    if cfg!(windows) {
        let rust_src = format!(
            "fn main() {{ let args: Vec<String> = std::env::args().collect(); if args.len() > 1 && args[1] == \"--version\" {{ println!(\"rustinel {version}\"); }} else {{ println!(\"mock rustinel running\"); }} }}"
        );
        let src_file = dir.join("mock_rustinel.rs");
        let _ = std::fs::write(&src_file, &rust_src);
        let status = Command::new("rustc")
            .arg(&src_file)
            .arg("-o")
            .arg(&path)
            .status();
        let _ = std::fs::remove_file(src_file);
        if status.is_err() || !status.unwrap().success() {
            let content = format!("@echo off\r\necho rustinel {version}\r\n");
            std::fs::write(&path, content).unwrap();
        }
    } else {
        use std::os::unix::fs::PermissionsExt;
        let content = format!("#!/bin/sh\necho \"rustinel {version}\"\n");
        std::fs::write(&path, content).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }
    path
}

fn get_version(binary: &Path) -> String {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .expect("Failed to execute mock rustinel");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn spawn_http_server(body: &'static str) -> (String, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let running = Arc::new(AtomicBool::new(true));
    let r_clone = running.clone();

    listener.set_nonblocking(true).unwrap();

    thread::spawn(move || {
        while r_clone.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://127.0.0.1:{port}/manifest.json"), running)
}

#[test]
fn test_cli_flags() {
    let updater_bin = env!("CARGO_BIN_EXE_radegast-rustinel-updater");

    let help_out = Command::new(updater_bin)
        .arg("--help")
        .output()
        .expect("Failed to run --help");
    assert!(help_out.status.success());
    let stdout = String::from_utf8_lossy(&help_out.stdout);
    assert!(stdout.contains("radegast-rustinel-updater"));

    let ver_out = Command::new(updater_bin)
        .arg("--version")
        .output()
        .expect("Failed to run --version");
    assert!(ver_out.status.success());
    let ver_stdout = String::from_utf8_lossy(&ver_out.stdout);
    assert!(ver_stdout.contains("radegast-rustinel-updater"));
}

#[test]
fn test_updater_end_to_end_flow() {
    let updater_bin = env!("CARGO_BIN_EXE_radegast-rustinel-updater");
    let temp_dir = tempfile::tempdir().unwrap();
    let rustinel_path = create_mock_rustinel(temp_dir.path(), "1.3.0");

    assert_eq!(get_version(&rustinel_path), "rustinel 1.3.0");

    let (manifest_url, stop_server) = spawn_http_server(MOCK_MANIFEST);

    let status = Command::new(updater_bin)
        .arg("--once")
        .env("UPDATER_MANIFEST_URL", &manifest_url)
        .env(
            "UPDATER_DOWNLOAD_URL",
            "https://console-api.radegast.app/api/v1",
        )
        .env("UPDATER_RUSTINEL_PATH", rustinel_path.to_str().unwrap())
        .env("UPDATER_AUTO_RESTART", "false")
        .env("UPDATER_LOG_LEVEL", "info")
        .status()
        .expect("Failed to execute updater binary");

    stop_server.store(false, Ordering::Relaxed);

    assert!(
        status.success(),
        "Updater execution failed with status: {status:?}"
    );

    let updated_ver = get_version(&rustinel_path);
    assert!(
        updated_ver.contains("1.8.0"),
        "Expected version 1.8.0 after update, got: {updated_ver}"
    );
}

#[test]
fn test_updater_tampered_manifest_fails_and_preserves_binary() {
    let updater_bin = env!("CARGO_BIN_EXE_radegast-rustinel-updater");
    let temp_dir = tempfile::tempdir().unwrap();
    let rustinel_path = create_mock_rustinel(temp_dir.path(), "1.3.0");

    // Tampered checksums
    let tampered_manifest = r#"[{"version":"1.9.9","hash_sha256":"0000000000000000000000000000000000000000000000000000000000000000  linux-amd64.zip\n","sign_gpg":"-----BEGIN PGP SIGNATURE-----\ncorrupted\n-----END PGP SIGNATURE-----\n"}]"#;
    let (manifest_url, stop_server) = spawn_http_server(tampered_manifest);

    let status = Command::new(updater_bin)
        .arg("--once")
        .env("UPDATER_MANIFEST_URL", &manifest_url)
        .env(
            "UPDATER_DOWNLOAD_URL",
            "https://console-api.radegast.app/api/v1",
        )
        .env("UPDATER_RUSTINEL_PATH", rustinel_path.to_str().unwrap())
        .env("UPDATER_AUTO_RESTART", "false")
        .env("UPDATER_LOG_LEVEL", "info")
        .status()
        .expect("Failed to execute updater binary");

    stop_server.store(false, Ordering::Relaxed);

    assert!(!status.success(), "Updater must fail on tampered manifest");
    assert_eq!(
        get_version(&rustinel_path),
        "rustinel 1.3.0",
        "Binary must not be replaced"
    );
}
