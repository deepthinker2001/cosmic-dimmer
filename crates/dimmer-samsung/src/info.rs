//! The TV's device info, `GET http://<tv>:8001/api/v2/`: passive, no prompt on
//! the TV, no pairing. It reports the power state, the model and the MAC used
//! for Wake-on-LAN. Treated as untrusted: the reply is size-capped and parsed
//! strictly, and the identifiers it carries are never printed by the tool.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::deadline::{Deadlined, Timeouts};
use crate::wol::MacAddr;
use crate::{Error, HTTP_PORT, Result, validate_host};

/// Largest reply accepted (the real one is about 1.5 KiB).
pub const MAX_REPLY: usize = 64 * 1024;

/// What the tool uses from the device info. Every field is optional: models
/// and firmware differ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    /// `"on"` or `"standby"` as the TV reports it. On the Frame, Art Mode is
    /// not distinguished here (see `docs/network-findings.md`).
    pub power_state: Option<String>,
    pub model_name: Option<String>,
    pub os: Option<String>,
    pub network_type: Option<String>,
    pub token_auth_support: bool,
    pub frame_tv_support: bool,
    pub mac: Option<MacAddr>,
}

impl DeviceInfo {
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.power_state.as_deref() == Some("on")
    }
}

#[derive(Deserialize)]
struct Raw {
    device: RawDevice,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawDevice {
    power_state: Option<String>,
    #[serde(rename = "modelName")]
    model_name: Option<String>,
    #[serde(rename = "OS")]
    os: Option<String>,
    #[serde(rename = "networkType")]
    network_type: Option<String>,
    token_auth_support: Option<serde_json::Value>,
    #[serde(rename = "FrameTVSupport")]
    frame_tv_support: Option<serde_json::Value>,
    #[serde(rename = "wifiMac")]
    wifi_mac: Option<String>,
}

/// The TV writes booleans as strings (`"true"`); accept either.
fn truthy(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// Short printable text, so odd values from the TV can't mess up a terminal.
fn clean(value: Option<String>) -> Option<String> {
    value.map(|s| {
        s.chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect::<String>()
    })
}

/// Parses the JSON body of `/api/v2/`.
///
/// # Errors
/// [`Error::Protocol`] when it isn't the expected shape.
pub fn parse(body: &[u8]) -> Result<DeviceInfo> {
    let raw: Raw = serde_json::from_slice(body)
        .map_err(|e| Error::Protocol(format!("device info isn't the expected JSON: {e}")))?;
    let d = raw.device;
    Ok(DeviceInfo {
        token_auth_support: truthy(d.token_auth_support.as_ref()),
        frame_tv_support: truthy(d.frame_tv_support.as_ref()),
        // A MAC that doesn't parse is dropped rather than trusted.
        mac: d.wifi_mac.as_deref().and_then(|m| m.parse().ok()),
        power_state: clean(d.power_state),
        model_name: clean(d.model_name),
        os: clean(d.os),
        network_type: clean(d.network_type),
    })
}

/// Fetches and parses the device info. `timeout` bounds the whole exchange
/// (connect, request and the complete reply), not each read.
///
/// # Errors
/// When the host is invalid, the TV doesn't answer within `timeout`, or the
/// reply isn't a 200 with the expected JSON.
pub fn fetch(host: &str, timeout: Duration) -> Result<DeviceInfo> {
    validate_host(host)?;
    let deadline = Instant::now() + timeout;
    let addr = (host, HTTP_PORT)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| Error::Protocol(format!("{host} doesn't resolve")))?;
    let tcp = TcpStream::connect_timeout(&addr, timeout).map_err(timeout_err)?;
    let reply = exchange(Deadlined::new(tcp, deadline), host)?;
    parse(&http_body(&reply)?)
}

/// Sends the request and reads the whole capped reply, within the stream's
/// deadline.
fn exchange<S: Read + Write + Timeouts>(mut stream: Deadlined<S>, host: &str) -> Result<Vec<u8>> {
    let request = format!(
        "GET /api/v2/ HTTP/1.1\r\nHost: {host}:{HTTP_PORT}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).map_err(timeout_err)?;
    let mut reply = Vec::new();
    stream
        .take(MAX_REPLY as u64 + 1)
        .read_to_end(&mut reply)
        .map_err(timeout_err)?;
    if reply.len() > MAX_REPLY {
        return Err(Error::Protocol("device info reply is too large".into()));
    }
    Ok(reply)
}

fn timeout_err(e: std::io::Error) -> Error {
    match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => Error::Timeout,
        _ => Error::Io(e),
    }
}

/// The body of a complete `HTTP/1.x 200` reply (with `Connection: close`),
/// de-chunked when needed.
///
/// # Errors
/// [`Error::Protocol`] for another status or a malformed reply.
pub fn http_body(reply: &[u8]) -> Result<Vec<u8>> {
    let split = reply
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| Error::Protocol("incomplete HTTP reply".into()))?;
    let head = std::str::from_utf8(&reply[..split])
        .map_err(|_| Error::Protocol("HTTP headers aren't text".into()))?;
    let body = &reply[split + 4..];
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let mut parts = status.split(' ');
    let (version, code) = (parts.next(), parts.next());
    if !version.is_some_and(|v| v.starts_with("HTTP/1.")) {
        return Err(Error::Protocol("not an HTTP/1 reply".into()));
    }
    if code != Some("200") {
        return Err(Error::Protocol(format!(
            "HTTP status {}",
            code.unwrap_or("?").chars().take(3).collect::<String>()
        )));
    }
    let chunked = lines.any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
        })
    });
    if chunked {
        dechunk(body)
    } else {
        Ok(body.to_vec())
    }
}

fn dechunk(mut body: &[u8]) -> Result<Vec<u8>> {
    let bad = || Error::Protocol("malformed chunked body".into());
    let mut out = Vec::new();
    loop {
        let line_end = body.windows(2).position(|w| w == b"\r\n").ok_or_else(bad)?;
        let size_text = std::str::from_utf8(&body[..line_end]).map_err(|_| bad())?;
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        // The size comes from the TV: bound it before any arithmetic.
        if size_text.is_empty() || size_text.len() > 8 {
            return Err(bad());
        }
        let size = usize::from_str_radix(size_text, 16).map_err(|_| bad())?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        let end = size
            .checked_add(2)
            .filter(|_| size <= MAX_REPLY && out.len() + size <= MAX_REPLY)
            .ok_or_else(bad)?;
        if body.len() < end || &body[size..end] != b"\r\n" {
            return Err(bad());
        }
        out.extend_from_slice(&body[..size]);
        body = &body[end..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape of the QN55LS03H's reply (identifiers replaced).
    const SAMPLE: &str = r#"{"device":{"FrameTVSupport":"true","OS":"Tizen","PowerState":"on",
        "TokenAuthSupport":"true","modelName":"QN55LS03HEFXZA","networkType":"wireless",
        "wifiMac":"02:00:00:12:34:56","id":"uuid:x","name":"x"},"type":"Samsung SmartTV",
        "version":"2.0.25"}"#;

    #[test]
    fn parses_sample() {
        let info = parse(SAMPLE.as_bytes()).unwrap();
        assert!(info.is_on());
        assert!(info.token_auth_support);
        assert!(info.frame_tv_support);
        assert_eq!(info.model_name.as_deref(), Some("QN55LS03HEFXZA"));
        assert_eq!(info.network_type.as_deref(), Some("wireless"));
        assert_eq!(info.mac, Some("02:00:00:12:34:56".parse().unwrap()));
    }

    #[test]
    fn standby_and_missing_fields() {
        let info = parse(br#"{"device":{"PowerState":"standby","wifiMac":"nonsense"}}"#).unwrap();
        assert!(!info.is_on());
        assert_eq!(info.mac, None);
        assert!(!info.token_auth_support);
        assert!(parse(b"{}").is_err());
        assert!(parse(b"not json").is_err());
    }

    #[test]
    fn strips_control_characters() {
        let info = parse(b"{\"device\":{\"modelName\":\"QN\\u001b[2Jx\"}}").unwrap();
        assert_eq!(info.model_name.as_deref(), Some("QN[2Jx"));
    }

    #[test]
    fn http_plain_and_chunked() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"a\":1}";
        assert_eq!(http_body(plain).unwrap(), b"{\"a\":1}");
        let chunked =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3;x=y\r\n:1}\r\n0\r\n\r\n";
        assert_eq!(http_body(chunked).unwrap(), b"{\"a\":1}");
        assert!(http_body(b"HTTP/1.1 404 Not Found\r\n\r\n").is_err());
        assert!(http_body(b"HTTP/1.1 200 OK\r\n").is_err());
        let truncated = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nff\r\nabc";
        assert!(http_body(truncated).is_err());
    }

    /// Regression: a huge chunk size used to overflow `size + 2` (a panic in
    /// debug builds, an out-of-range slice in release).
    #[test]
    fn huge_chunk_sizes_are_rejected() {
        for size in [
            "ffffffffffffffff",
            "fffffffffffffffe",
            "ffffffff",
            "10001",
            "",
            "-1",
        ] {
            let reply = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{size}\r\nabc\r\n0\r\n\r\n"
            );
            assert!(http_body(reply.as_bytes()).is_err(), "{size:?}");
        }
        // Many small chunks can't add up past the cap either.
        let mut reply = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for _ in 0..=(MAX_REPLY / 4096) {
            reply.extend_from_slice(b"1000\r\n");
            reply.extend_from_slice(&[b'a'; 4096]);
            reply.extend_from_slice(b"\r\n");
        }
        reply.extend_from_slice(b"0\r\n\r\n");
        assert!(http_body(&reply).is_err());
    }

    /// `fetch()` always dials port 8001, so drive its exchange over a local
    /// socket instead.
    fn local_exchange(
        serve: impl FnOnce(TcpStream) + Send + 'static,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            serve(s);
        });
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let result = exchange(Deadlined::new(tcp, Instant::now() + timeout), "127.0.0.1");
        server.join().unwrap();
        result
    }

    #[test]
    fn fetch_against_local_server() {
        let reply = local_exchange(
            |mut s| {
                let mut buf = [0u8; 512];
                let n = s.read(&mut buf).unwrap();
                assert!(buf[..n].starts_with(b"GET /api/v2/ HTTP/1.1\r\n"));
                let body = SAMPLE.as_bytes();
                write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .unwrap();
                s.write_all(body).unwrap();
            },
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(parse(&http_body(&reply).unwrap()).unwrap().is_on());
    }

    #[test]
    fn trickled_reply_hits_the_overall_deadline() {
        let started = Instant::now();
        let result = local_exchange(
            |mut s| {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                // One byte every 100 ms: no single read ever times out.
                for _ in 0..30 {
                    if s.write_all(b"H").is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            },
            Duration::from_millis(500),
        );
        assert!(matches!(result, Err(Error::Timeout)), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn oversized_reply_is_refused() {
        let result = local_exchange(
            |mut s| {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                let _ = s.write_all(&vec![b'a'; MAX_REPLY + 10]);
            },
            Duration::from_secs(5),
        );
        assert!(matches!(result, Err(Error::Protocol(_))), "{result:?}");
    }

    #[test]
    fn fetch_rejects_bad_host() {
        assert!(matches!(
            fetch("tv\r\nX: 1", Duration::from_millis(10)),
            Err(Error::InvalidHost(_))
        ));
    }
}
