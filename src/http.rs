use anyhow::{ensure, Result};
use reqwest::{blocking::Client, redirect::Policy, Url};
use std::io::{Read, Write};
use std::time::Duration;

fn allowed_url(url: &Url) -> bool {
    let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
    url.scheme() == "https"
        || (url.scheme() == "http"
            && (host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())))
}

pub fn client(url: &str, timeout: Duration) -> Result<Client> {
    ensure!(
        allowed_url(&Url::parse(url)?),
        "HTTPS is required except on loopback hosts"
    );
    Ok(Client::builder()
        .user_agent(concat!(
            "radegast-rustinel-updater/",
            env!("CARGO_PKG_VERSION")
        ))
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("Too many redirects");
            }
            if !allowed_url(attempt.url())
                || (attempt.previous().iter().any(|url| url.scheme() == "https")
                    && attempt.url().scheme() != "https")
            {
                return attempt.error("Insecure redirect rejected");
            }
            attempt.follow()
        }))
        .connect_timeout(Duration::from_secs(30))
        .timeout(timeout)
        .build()?)
}

pub fn copy_limited(reader: impl Read, mut writer: impl Write, limit: u64) -> Result<()> {
    let copied = std::io::copy(&mut reader.take(limit + 1), &mut writer)?;
    ensure!(
        copied <= limit,
        "Response exceeds size limit of {limit} bytes"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_policy_checks_host_not_substrings() {
        for url in [
            "http://localhost:1234",
            "http://127.0.0.1",
            "http://[::1]",
            "https://example.com",
        ] {
            assert!(allowed_url(&Url::parse(url).unwrap()), "{url}");
        }
        for url in [
            "http://localhost.example.com",
            "http://example.com/localhost",
            "http://127.0.0.1@example.com",
            "http://example.com/?host=127.0.0.1",
            "ftp://localhost",
        ] {
            assert!(!allowed_url(&Url::parse(url).unwrap()), "{url}");
        }
    }

    #[test]
    fn bounded_copy_rejects_oversized_bodies() {
        let mut output = Vec::new();
        copy_limited(&b"1234"[..], &mut output, 4).unwrap();
        assert_eq!(output, b"1234");
        assert!(copy_limited(&b"12345"[..], Vec::new(), 4).is_err());
    }

    #[test]
    fn redirects_cannot_escape_transport_policy() {
        use std::net::TcpListener;
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            let mut buf = [0; 1024];
            assert!(stream.read(&mut buf).unwrap() > 0);
            stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://example.com/localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        assert!(client(&url, Duration::from_secs(5))
            .unwrap()
            .get(&url)
            .send()
            .is_err());
        worker.join().unwrap();
    }
}
