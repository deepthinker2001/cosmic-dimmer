//! The TV's WebSocket remote API, `wss://<tv>:8002/api/v2/channels/
//! samsung.remote.control?name=<base64 name>&token=<token>`.
//!
//! The first connection without a valid token makes the TV show an
//! Allow/Deny prompt; on Allow, the TV's `ms.channel.connect` event carries a
//! token to use from then on. Key presses are `ms.remote.control` messages.
//!
//! The URL holds the token, so it is never formatted into an error or a log.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};
use serde_json::{Value, json};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{HandshakeError, Message, WebSocket};
use zeroize::Zeroizing;

use crate::IO_TIMEOUT;
use crate::config::Secret;
use crate::deadline::{Deadlined, Timeouts};
use crate::pin::{Fingerprint, PinVerifier, Policy};
use crate::{Error, Result, TLS_PORT, validate_host};

/// Largest WebSocket message accepted from the TV.
pub const MAX_MESSAGE: usize = 64 * 1024;
/// How long `KEY_POWER` is held for a full power-off on the Frame (a short
/// press only toggles Art Mode).
pub const POWER_OFF_HOLD: Duration = Duration::from_secs(3);

/// What the TV said on the channel.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// Connected; `token` is set when the TV issued (or re-issued) one.
    Connect { token: Option<Secret> },
    /// The user chose Deny, or the token was rejected.
    Unauthorized,
    /// Nobody answered the prompt.
    PairingTimedOut,
    /// Anything else (`ms.channel.ready`, `ed.*` notifications ...).
    Other,
}

/// A token we are willing to put in a URL: the TV issues short digit strings.
fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= 64 && token.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Parses one text message from the TV.
///
/// # Errors
/// [`Error::Protocol`] when it isn't JSON, or the token in it isn't plausible
/// (the value itself is never echoed).
pub fn parse_event(text: &str) -> Result<Event> {
    let mut value: Value = serde_json::from_str(text)
        .map_err(|_| Error::Protocol("a channel message isn't JSON".into()))?;
    let event = value
        .get("event")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(match event.as_deref() {
        Some("ms.channel.connect") => {
            // Moved out of the JSON value, not copied, straight into a Secret
            // (wiped on drop) or a Zeroizing string.
            let token = match value.pointer_mut("/data/token").map(Value::take) {
                None | Some(Value::Null) => None,
                Some(Value::String(t)) => {
                    let t = Secret::new(t);
                    if !valid_token(t.expose()) {
                        return Err(unexpected_token());
                    }
                    Some(t)
                }
                Some(Value::Number(n)) => {
                    let t = Secret::new(n.to_string());
                    if !valid_token(t.expose()) {
                        return Err(unexpected_token());
                    }
                    Some(t)
                }
                Some(_) => return Err(unexpected_token()),
            };
            Event::Connect { token }
        }
        Some("ms.channel.unauthorized") => Event::Unauthorized,
        Some("ms.channel.timeOut") => Event::PairingTimedOut,
        _ => Event::Other,
    })
}

fn unexpected_token() -> Error {
    Error::Protocol("the TV sent a token in an unexpected form".into())
}

/// The channel URL. Contains the token: pass it to the handshake only. It is
/// wiped from memory when dropped.
///
/// # Errors
/// When the host or the stored token isn't valid, or the name isn't short
/// printable ASCII.
pub fn channel_url(
    host: &str,
    client_name: &str,
    token: Option<&Secret>,
) -> Result<Zeroizing<String>> {
    validate_host(host)?;
    if client_name.is_empty()
        || client_name.len() > 40
        || !client_name.bytes().all(|b| (0x20..0x7f).contains(&b))
    {
        return Err(Error::Config(
            "client_name must be 1-40 printable ASCII characters".into(),
        ));
    }
    let name = base64::engine::general_purpose::STANDARD.encode(client_name);
    let mut url = Zeroizing::new(format!(
        "wss://{host}:{TLS_PORT}/api/v2/channels/samsung.remote.control?name={name}"
    ));
    // Room for the token up front, so pushing it never reallocates and leaves
    // an unwiped copy behind.
    url.reserve(8 + token.map_or(0, |t| t.expose().len()));
    if let Some(token) = token {
        if !valid_token(token.expose()) {
            return Err(Error::Config(
                "the stored token isn't valid; run `dimmer-samsung pair`".into(),
            ));
        }
        url.push_str("&token=");
        url.push_str(token.expose());
    }
    Ok(url)
}

/// A key press, as the TV's remote API takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Click,
    Press,
    Release,
}

/// The JSON for a key action.
#[must_use]
pub fn key_message(action: KeyAction, key: &str) -> String {
    let cmd = match action {
        KeyAction::Click => "Click",
        KeyAction::Press => "Press",
        KeyAction::Release => "Release",
    };
    json!({
        "method": "ms.remote.control",
        "params": {
            "Cmd": cmd,
            "DataOfCmd": key,
            "Option": "false",
            "TypeOfRemote": "SendRemoteKey",
        }
    })
    .to_string()
}

/// An open, connected channel. Every exchange on it runs under a deadline.
pub struct Remote<S: Read + Write + Timeouts> {
    ws: WebSocket<Deadlined<S>>,
}

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
}

fn io_kind_err(e: io::Error) -> Error {
    match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Error::Timeout,
        _ => Error::Io(e),
    }
}

/// tungstenite errors, without anything that could carry the URL.
fn ws_err(e: tungstenite::Error) -> Error {
    match e {
        tungstenite::Error::Io(io) => io_kind_err(io),
        tungstenite::Error::Tls(tls) => Error::Tls(tls.to_string()),
        tungstenite::Error::Http(response) => match response.status().as_u16() {
            401 | 403 => Error::Unauthorized,
            code => Error::Protocol(format!("WebSocket upgrade refused with HTTP {code}")),
        },
        tungstenite::Error::Url(_) => Error::Protocol("invalid channel URL".into()),
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
            Error::Protocol("the TV closed the channel".into())
        }
        tungstenite::Error::Capacity(_) => Error::Protocol("message from the TV too large".into()),
        other => Error::Protocol(format!("WebSocket: {}", kind_name(&other))),
    }
}

fn kind_name(e: &tungstenite::Error) -> &'static str {
    match e {
        tungstenite::Error::Protocol(_) => "protocol violation",
        tungstenite::Error::WriteBufferFull(_) => "write buffer full",
        tungstenite::Error::Utf8(_) => "invalid UTF-8",
        tungstenite::Error::AttackAttempt => "attack attempt",
        tungstenite::Error::HttpFormat(_) => "malformed HTTP",
        _ => "error",
    }
}

impl<S: Read + Write + Timeouts> Remote<S> {
    /// The WebSocket upgrade over an already-connected stream, all of it
    /// (including a TLS handshake the stream does lazily) by `deadline`.
    ///
    /// # Errors
    /// On timeout, refusal, or an invalid URL.
    pub fn handshake(stream: S, url: &str, deadline: Instant) -> Result<Self> {
        let stream = Deadlined::new(stream, deadline);
        match tungstenite::client::client_with_config(url, stream, Some(ws_config())) {
            Ok((ws, _response)) => Ok(Self { ws }),
            Err(HandshakeError::Interrupted(_)) => Err(Error::Timeout),
            Err(HandshakeError::Failure(e)) => Err(ws_err(e)),
        }
    }

    /// Reads until the TV's verdict on this connection, or `deadline`.
    ///
    /// # Errors
    /// [`Error::Unauthorized`], [`Error::PairingTimedOut`], [`Error::Timeout`]
    /// or a protocol error.
    pub fn wait_connect(&mut self, deadline: Instant) -> Result<Option<Secret>> {
        self.ws.get_mut().set_deadline(deadline);
        loop {
            let message = match self.ws.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::Interrupted => {
                    continue;
                }
                Err(e) => return Err(ws_err(e)),
            };
            match message {
                Message::Text(text) => match parse_event(text.as_str())? {
                    Event::Connect { token } => return Ok(token),
                    Event::Unauthorized => return Err(Error::Unauthorized),
                    Event::PairingTimedOut => return Err(Error::PairingTimedOut),
                    Event::Other => {}
                },
                Message::Close(_) => return Err(Error::Unauthorized),
                _ => {}
            }
        }
    }

    /// Sends one key action, within [`IO_TIMEOUT`].
    ///
    /// # Errors
    /// When the channel is closed or the write times out.
    pub fn send_key(&mut self, action: KeyAction, key: &str) -> Result<()> {
        self.ws.get_mut().set_deadline(Instant::now() + IO_TIMEOUT);
        self.ws
            .send(Message::text(key_message(action, key)))
            .map_err(ws_err)
    }

    /// Closes the channel politely, giving up after a second; errors are
    /// ignored (we are done anyway).
    pub fn close(mut self) {
        self.ws
            .get_mut()
            .set_deadline(Instant::now() + Duration::from_secs(1));
        let _ = self.ws.close(None);
        let _ = self.ws.flush();
    }
}

/// A TLS connection to the TV's port 8002 using the pin policy, then the
/// WebSocket upgrade. Returns the channel and the certificate seen.
///
/// # Errors
/// [`Error::CertificateMismatch`] when the certificate isn't the pinned one;
/// otherwise network, TLS or upgrade errors.
pub fn connect_tls(
    host: &str,
    url: &str,
    policy: Policy,
    timeout: Duration,
) -> Result<(
    Remote<StreamOwned<ClientConnection, TcpStream>>,
    Fingerprint,
)> {
    validate_host(host)?;
    let addr = (host, TLS_PORT)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| Error::Protocol(format!("{host} doesn't resolve")))?;
    connect_tls_to(addr, host, url, policy, timeout)
}

/// [`connect_tls`] to a resolved address.
///
/// # Errors
/// As [`connect_tls`].
pub fn connect_tls_to(
    addr: SocketAddr,
    host: &str,
    url: &str,
    policy: Policy,
    timeout: Duration,
) -> Result<(
    Remote<StreamOwned<ClientConnection, TcpStream>>,
    Fingerprint,
)> {
    validate_host(host)?;
    // One deadline for TCP connect, TLS handshake and WebSocket upgrade.
    let deadline = Instant::now() + timeout;
    let tcp = TcpStream::connect_timeout(&addr, timeout).map_err(io_kind_err)?;
    let (verifier, seen) = PinVerifier::new(policy);
    let config = verifier
        .client_config()
        .map_err(|e| Error::Tls(e.to_string()))?;
    let server_name = ServerName::try_from(host.to_owned())
        .map_err(|_| Error::InvalidHost(host.chars().take(64).collect()))?;
    let conn = ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| Error::Tls(e.to_string()))?;
    let stream = StreamOwned::new(conn, tcp);
    let result = Remote::handshake(stream, url, deadline);
    let seen = seen.lock().ok().and_then(|s| *s);
    match (result, policy, seen) {
        (Err(_), Policy::Pinned(pinned), Some(fp)) if fp != pinned => {
            Err(Error::CertificateMismatch)
        }
        (Err(e), _, _) => Err(e),
        (Ok(remote), _, Some(fp)) => Ok((remote, fp)),
        (Ok(_), _, None) => Err(Error::Tls("no certificate was presented".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    use tungstenite::handshake::server::{Request, Response};

    #[test]
    fn events() {
        let connect =
            r#"{"event":"ms.channel.connect","data":{"clients":[],"id":"x","token":"12345678"}}"#;
        match parse_event(connect).unwrap() {
            Event::Connect { token } => assert_eq!(token.unwrap().expose(), "12345678"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            parse_event(r#"{"event":"ms.channel.connect","data":{"token":12345678}}"#).unwrap(),
            Event::Connect {
                token: Some(Secret::new("12345678".into()))
            }
        );
        assert_eq!(
            parse_event(r#"{"event":"ms.channel.connect","data":{}}"#).unwrap(),
            Event::Connect { token: None }
        );
        assert_eq!(
            parse_event(r#"{"event":"ms.channel.unauthorized"}"#).unwrap(),
            Event::Unauthorized
        );
        assert_eq!(
            parse_event(r#"{"event":"ms.channel.timeOut"}"#).unwrap(),
            Event::PairingTimedOut
        );
        assert_eq!(
            parse_event(r#"{"event":"ed.edenTV.update"}"#).unwrap(),
            Event::Other
        );
        assert!(parse_event("nope").is_err());
        let evil = r#"{"event":"ms.channel.connect","data":{"token":"1&x=2"}}"#;
        let err = parse_event(evil).unwrap_err().to_string();
        assert!(!err.contains("1&x=2"), "{err}");
    }

    #[test]
    fn urls() {
        let url = channel_url("192.0.2.1", "Samsung Dimmer", None).unwrap();
        assert_eq!(
            url.as_str(),
            "wss://192.0.2.1:8002/api/v2/channels/samsung.remote.control?name=U2Ftc3VuZyBEaW1tZXI="
        );
        let token = Secret::new("1234".into());
        assert!(
            channel_url("192.0.2.1", "x", Some(&token))
                .unwrap()
                .ends_with("&token=1234")
        );
        assert!(channel_url("192.0.2.1", "x", Some(&Secret::new("1 2".into()))).is_err());
        assert!(channel_url("192.0.2.1", "bad\nname", None).is_err());
        assert!(channel_url("192.0.2.1:1", "x", None).is_err());
    }

    #[test]
    fn key_json() {
        let v: Value = serde_json::from_str(&key_message(KeyAction::Press, "KEY_POWER")).unwrap();
        assert_eq!(v["method"], "ms.remote.control");
        assert_eq!(v["params"]["Cmd"], "Press");
        assert_eq!(v["params"]["DataOfCmd"], "KEY_POWER");
        assert_eq!(v["params"]["TypeOfRemote"], "SendRemoteKey");
    }

    /// A stand-in TV: accepts the upgrade, checks the request path, sends
    /// `first` and records what the client sends.
    // tungstenite's header callback has to return its own large error type.
    #[allow(clippy::result_large_err)]
    fn fake_tv(first: &'static str) -> (u16, thread::JoinHandle<(String, Vec<String>)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut path = String::new();
            let mut ws = tungstenite::accept_hdr(stream, |req: &Request, resp: Response| {
                path = req.uri().to_string();
                Ok(resp)
            })
            .unwrap();
            ws.send(Message::text(r#"{"event":"ms.channel.ready"}"#))
                .unwrap();
            ws.send(Message::text(first)).unwrap();
            let mut got = Vec::new();
            while let Ok(m) = ws.read() {
                if let Message::Text(t) = m {
                    got.push(t.to_string());
                }
            }
            (path, got)
        });
        (port, handle)
    }

    #[test]
    fn pairs_and_sends_over_websocket() {
        let (port, tv) =
            fake_tv(r#"{"event":"ms.channel.connect","data":{"token":"87654321","clients":[]}}"#);
        let url = channel_url("127.0.0.1", "Samsung Dimmer", None).unwrap();
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut remote =
            Remote::handshake(tcp, &url, Instant::now() + Duration::from_secs(5)).unwrap();
        let token = remote
            .wait_connect(Instant::now() + Duration::from_secs(5))
            .unwrap();
        assert_eq!(token.unwrap().expose(), "87654321");
        remote.send_key(KeyAction::Click, "KEY_POWER").unwrap();
        remote.close();
        let (path, got) = tv.join().unwrap();
        assert!(
            path.ends_with("/api/v2/channels/samsung.remote.control?name=U2Ftc3VuZyBEaW1tZXI="),
            "{path}"
        );
        assert_eq!(got.len(), 1);
        assert!(got[0].contains("KEY_POWER"));
    }

    #[test]
    fn denied_pairing() {
        let (port, tv) = fake_tv(r#"{"event":"ms.channel.unauthorized"}"#);
        let url = channel_url("127.0.0.1", "x", None).unwrap();
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut remote =
            Remote::handshake(tcp, &url, Instant::now() + Duration::from_secs(5)).unwrap();
        assert!(matches!(
            remote.wait_connect(Instant::now() + Duration::from_secs(5)),
            Err(Error::Unauthorized)
        ));
        remote.close();
        tv.join().unwrap();
    }

    #[test]
    fn silent_tv_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let tv = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            // Say nothing until the client gives up.
            let _ = ws.read();
        });
        let url = channel_url("127.0.0.1", "x", None).unwrap();
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut remote =
            Remote::handshake(tcp, &url, Instant::now() + Duration::from_secs(5)).unwrap();
        let started = Instant::now();
        assert!(matches!(
            remote.wait_connect(Instant::now() + Duration::from_millis(300)),
            Err(Error::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
        remote.close();
        tv.join().unwrap();
    }

    /// A TLS stand-in TV with a fresh self-signed certificate (like the TV's,
    /// not valid for any name we connect to). Returns its port and the
    /// certificate's fingerprint.
    fn tls_tv(connections: usize) -> (u16, Fingerprint, thread::JoinHandle<()>) {
        use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
        let cert = rcgen::generate_simple_self_signed(vec!["tv.invalid".into()]).unwrap();
        let der = cert.cert.der().clone();
        let fp = crate::pin::fingerprint(der.as_ref());
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()));
        let config = Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![der], key)
            .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            for _ in 0..connections {
                let (tcp, _) = listener.accept().unwrap();
                let conn = rustls::ServerConnection::new(Arc::clone(&config)).unwrap();
                let tls = StreamOwned::new(conn, tcp);
                // A refused handshake (pin mismatch) ends here.
                let Ok(mut ws) = tungstenite::accept(tls) else {
                    continue;
                };
                let _ = ws.send(Message::text(
                    r#"{"event":"ms.channel.connect","data":{"token":"4242"}}"#,
                ));
                while ws.read().is_ok() {}
            }
        });
        (port, fp, handle)
    }

    #[test]
    fn tls_pinning_end_to_end() {
        let (port, fp, tv) = tls_tv(3);
        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        let url = channel_url("127.0.0.1", "x", None).unwrap();
        let t = Duration::from_secs(5);

        // First use: accepted, and the certificate is reported for pinning.
        let (mut remote, seen) =
            connect_tls_to(addr, "127.0.0.1", &url, Policy::TrustOnFirstUse, t).unwrap();
        assert_eq!(seen, fp);
        let token = remote.wait_connect(Instant::now() + t).unwrap();
        assert_eq!(token.unwrap().expose(), "4242");
        remote.close();

        // Pinned to that certificate: accepted.
        let (remote, _) = connect_tls_to(addr, "127.0.0.1", &url, Policy::Pinned(fp), t).unwrap();
        remote.close();

        // Pinned to another: refused before anything is sent.
        let err = connect_tls_to(addr, "127.0.0.1", &url, Policy::Pinned([7; 32]), t)
            .err()
            .unwrap();
        assert!(matches!(err, Error::CertificateMismatch), "{err:?}");
        tv.join().unwrap();
    }

    #[test]
    fn trickled_upgrade_hits_the_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let tv = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            // A valid status line, then a header line every 100 ms: each read
            // succeeds and parses, but the reply never ends.
            let _ = s.write_all(b"HTTP/1.1 101 Switching Protocols\r\n");
            for _ in 0..30 {
                if s.write_all(b"X-Slow: 1\r\n").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        let url = channel_url("127.0.0.1", "x", None).unwrap();
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let started = Instant::now();
        let result = Remote::handshake(tcp, &url, started + Duration::from_millis(500));
        assert!(
            matches!(result, Err(Error::Timeout)),
            "{:?}",
            result.as_ref().err()
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        drop(result);
        tv.join().unwrap();
    }

    #[test]
    fn upgrade_errors_hide_the_url() {
        // A server that answers the upgrade with 401 and nothing else.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let tv = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).unwrap();
            s.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        let token = Secret::new("99999999".into());
        let url = channel_url("127.0.0.1", "x", Some(&token)).unwrap();
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let err = Remote::handshake(tcp, &url, Instant::now() + Duration::from_secs(5))
            .err()
            .unwrap();
        assert!(matches!(err, Error::Unauthorized));
        assert!(!format!("{err} {err:?}").contains("99999999"));
        tv.join().unwrap();
    }
}
