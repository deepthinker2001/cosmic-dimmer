//! The tool's config file: the TV's address, its MAC for Wake-on-LAN, the
//! pinned TLS certificate and the pairing token.
//!
//! It lives at `$XDG_CONFIG_HOME/samsung-dimmer/samsung-tv.json` (else
//! `~/.config/...`), in a directory of mode 0700, written atomically with mode
//! 0600. A file anyone else can read, or a symlink, is refused rather than
//! used. Nothing in it is ever printed; the token's `Debug` is redacted.

use std::fmt;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::wol::MacAddr;
use crate::{Error, Result, validate_host};

const APP_DIR: &str = "samsung-dimmer";
const FILE_NAME: &str = "samsung-tv.json";
/// Name the TV shows in its pairing prompt and its device list.
pub const DEFAULT_CLIENT_NAME: &str = "Samsung Dimmer";

/// A credential. Serialized as a plain string in the 0600 config file, but its
/// `Debug` is redacted and it has no `Display`, so it can't reach a log line
/// by accident. It isn't `Clone`, and its memory is wiped when it is dropped
/// (best effort: copies inside libraries, e.g. tungstenite's read buffer,
/// are out of reach).
#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Secret {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The value, for putting on the wire only.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// The TV's IPv4 address or hostname.
    pub host: String,
    /// The TV's network MAC (from its device info), for Wake-on-LAN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<MacAddr>,
    /// SHA-256 of the TV's TLS certificate on port 8002, pinned at pairing
    /// (lowercase hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_sha256: Option<String>,
    /// The token the TV issued when the user allowed this client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<Secret>,
    /// The name shown on the TV.
    #[serde(default = "default_name")]
    pub client_name: String,
}

fn default_name() -> String {
    DEFAULT_CLIENT_NAME.to_owned()
}

/// The config file's path under the XDG config directory.
///
/// # Errors
/// When neither an absolute `XDG_CONFIG_HOME` nor `HOME` is set.
pub fn default_path() -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if Path::new(&dir).is_absolute() => PathBuf::from(dir),
        _ => {
            let home = std::env::var_os("HOME")
                .filter(|h| Path::new(h).is_absolute())
                .ok_or_else(|| Error::Config("neither XDG_CONFIG_HOME nor HOME is set".into()))?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(base.join(APP_DIR).join(FILE_NAME))
}

impl Config {
    /// Reads the config, or `None` when there is none yet.
    ///
    /// The file is opened with `O_NOFOLLOW` and checked on the open handle,
    /// so it can't be swapped between the check and the read.
    ///
    /// # Errors
    /// When the file is a symlink or not a regular file, isn't owned by this
    /// user, is readable or writable by anyone else, can't be read, or
    /// doesn't parse.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let err = |e: std::io::Error| Error::Config(format!("{}: {e}", path.display()));
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
                return Err(Error::Config(format!(
                    "{} is a symlink; symlinks are refused",
                    path.display()
                )));
            }
            Err(e) => return Err(err(e)),
        };
        let meta = file.metadata().map_err(err)?;
        if !meta.file_type().is_file() {
            return Err(Error::Config(format!(
                "{} is not a regular file",
                path.display()
            )));
        }
        // The process's own uid, without unsafe code: /proc/self belongs to it.
        let me = fs::metadata("/proc/self").map_err(err)?.uid();
        if meta.uid() != me {
            return Err(Error::Config(format!(
                "{} belongs to another user; it holds a credential, so it is refused",
                path.display()
            )));
        }
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(Error::Config(format!(
                "{} has mode {mode:03o}; it holds a credential, so it must be 600 (chmod 600 it, or delete it and pair again)",
                path.display()
            )));
        }
        let mut text = Zeroizing::new(String::new());
        file.read_to_string(&mut text).map_err(err)?;
        // serde_json's error says where, never echoes the value.
        let config: Self = serde_json::from_str(&text)
            .map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
        validate_host(&config.host)?;
        Ok(Some(config))
    }

    /// Writes the config atomically: a new file created with mode 0600 in a
    /// directory of mode 0700, then renamed over the old one.
    ///
    /// # Errors
    /// When the directory or file can't be created or written.
    pub fn save(&self, path: &Path) -> Result<()> {
        let err = |e: std::io::Error| Error::Config(format!("{}: {e}", path.display()));
        let dir = path
            .parent()
            .ok_or_else(|| Error::Config("config path has no directory".into()))?;
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(err)?;
        // Our own directory: make sure it is private even if it existed.
        if dir.file_name().is_some_and(|n| n == APP_DIR) {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(err)?;
        }
        let mut text = Zeroizing::new(
            serde_json::to_string_pretty(self)
                .map_err(|e| Error::Config(format!("serializing: {e}")))?,
        );
        text.push('\n');

        let tmp = dir.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
        let _ = fs::remove_file(&tmp);
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result.map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dimmer-samsung-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join(APP_DIR).join(FILE_NAME)
    }

    fn sample() -> Config {
        Config {
            host: "192.0.2.10".into(),
            mac: Some("02:00:00:00:00:01".parse().unwrap()),
            cert_sha256: Some("ab".repeat(32)),
            token: Some(Secret::new("12345678".into())),
            client_name: DEFAULT_CLIENT_NAME.into(),
        }
    }

    #[test]
    fn round_trip_with_private_modes() {
        let path = scratch("round");
        assert_eq!(Config::load(&path).unwrap(), None);
        sample().save(&path).unwrap();
        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let dir_mode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);
        assert_eq!(dir_mode, 0o700);
        assert_eq!(Config::load(&path).unwrap(), Some(sample()));
        // Saving again replaces it and leaves no temporary file behind.
        sample().save(&path).unwrap();
        let entries = fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(entries, 1);
    }

    #[test]
    fn refuses_readable_file_and_symlink() {
        let path = scratch("loose");
        sample().save(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let err = Config::load(&path).unwrap_err().to_string();
        assert!(err.contains("644"), "{err}");

        let link = path.with_file_name("link.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(Config::load(&link).is_err());
    }

    #[test]
    fn rejects_bad_host_in_file() {
        let path = scratch("host");
        let mut config = sample();
        config.host = "tv\r\nInjected: 1".into();
        config.save(&path).unwrap();
        assert!(matches!(Config::load(&path), Err(Error::InvalidHost(_))));
    }

    #[test]
    fn token_never_in_debug() {
        let debug = format!("{:?}", sample());
        assert!(!debug.contains("12345678"), "{debug}");
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn xdg_path() {
        // Only checks the file name: the environment is shared by tests.
        let path = default_path().unwrap();
        assert!(path.ends_with("samsung-dimmer/samsung-tv.json"));
    }
}
