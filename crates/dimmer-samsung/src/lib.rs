//! COSMIC Dimmer's network route to a Samsung Tizen TV (the Frame).
//!
//! Wakes the TV with Wake-on-LAN and powers it through the TV's local
//! WebSocket remote API (`wss://<tv>:8002`, token pairing). Findings and the
//! reasoning behind each choice are in `docs/network-findings.md`.
//!
//! Everything the TV sends is treated as untrusted: replies are size-capped,
//! every socket has a timeout, and the TLS certificate is pinned rather than
//! accepted blindly. The pairing token is a [`config::Secret`]: it is kept in
//! the user's XDG config directory at mode 0600 and never printed.

pub mod config;
pub mod deadline;
pub mod info;
pub mod pin;
pub mod remote;
pub mod wol;

use std::time::Duration;

/// Port of the TV's plain-HTTP device-info and remote API.
pub const HTTP_PORT: u16 = 8001;
/// Port of the TV's TLS remote API (token pairing).
pub const TLS_PORT: u16 = 8002;
/// Default timeout for connecting to and reading from the TV.
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors at the boundary with the TV and the config file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid host {0:?}: use an IPv4 address or a hostname")]
    InvalidHost(String),
    #[error("invalid MAC address")]
    InvalidMac,
    #[error("network error talking to the TV: {0}")]
    Io(#[from] std::io::Error),
    #[error("timed out waiting for the TV")]
    Timeout,
    #[error("unexpected reply from the TV: {0}")]
    Protocol(String),
    #[error(
        "the TV refused the connection (pairing was denied or the token is no longer valid); run `dimmer-samsung pair`"
    )]
    Unauthorized,
    #[error("nobody answered the pairing prompt on the TV in time")]
    PairingTimedOut,
    #[error(
        "the TV's TLS certificate doesn't match the one pinned at pairing; if the TV's firmware was updated, check it and run `dimmer-samsung pair --repin`"
    )]
    CertificateMismatch,
    #[error("TLS error: {0}")]
    Tls(String),
    #[error("config: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Checks that `host` is something we can safely put in a URL and a `Host:`
/// header: an IPv4 address or a DNS name. Rejects anything else, so a value
/// from the config file can't inject into requests.
///
/// # Errors
/// [`Error::InvalidHost`] when it is empty, too long or has other characters.
pub fn validate_host(host: &str) -> Result<()> {
    let ok = !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['-', '.'])
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidHost(host.chars().take(64).collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert!(validate_host("192.0.2.38").is_ok());
        assert!(validate_host("Samsung.local").is_ok());
        for bad in [
            "",
            "-x",
            "a b",
            "tv/path",
            "tv:8001",
            "tv\r\nX: y",
            "fe80::1",
            "tv?token=",
        ] {
            assert!(validate_host(bad).is_err(), "{bad:?}");
        }
    }
}
