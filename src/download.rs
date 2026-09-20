use anyhow::{Context, Result};
use reqwest::blocking::Client;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

pub fn fetch_binary(
    download_url: &str,
    version_str: &str,
    platform: &crate::platform::Platform,
) -> Result<PathBuf> {
    let is_localhost = download_url.contains("127.0.0.1") || download_url.contains("localhost");
    let client = Client::builder()
        .user_agent(concat!(
            "radegast-rustinel-updater/",
            env!("CARGO_PKG_VERSION")
        ))
        .https_only(!is_localhost && !cfg!(test))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()?;
    let url = format!(
        "{}/public/releases/{}/{}/{}/download",
        download_url.trim_end_matches('/'),
        version_str,
        platform.os_name(),
        platform.arch_name(),
    );
    tracing::info!("Downloading from {url}");
    let response = client
        .get(&url)
        .send()?
        .error_for_status()
        .context("Failed to download rustinel binary")?;
    let mut tmp = tempfile::NamedTempFile::new()?;
    let bytes = response.bytes()?;
    tmp.write_all(&bytes)?;
    tmp.flush()?;
    let path = tmp.into_temp_path().keep()?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn test_fetch_binary_mock_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req_buf = [0u8; 1024];
                let _ = stream.read(&mut req_buf);

                let body = b"fake-binary-zip-stream";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/zip\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
                let _ = stream.flush();
            }
        });

        let platform = crate::platform::Platform {
            os: "linux",
            arch: "amd64",
        };
        let download_url = format!("http://127.0.0.1:{port}");
        let downloaded_path = fetch_binary(&download_url, "1.7.1r1", &platform).unwrap();

        assert!(downloaded_path.exists());
        let content = std::fs::read(&downloaded_path).unwrap();
        assert_eq!(content, b"fake-binary-zip-stream");

        let _ = std::fs::remove_file(downloaded_path);
    }

    #[test]
    fn test_fetch_binary_http_404_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req_buf = [0u8; 1024];
                let _ = stream.read(&mut req_buf);

                let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot Found";
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let platform = crate::platform::Platform {
            os: "linux",
            arch: "amd64",
        };
        let download_url = format!("http://127.0.0.1:{port}");
        assert!(fetch_binary(&download_url, "9.9.9", &platform).is_err());
    }
}
