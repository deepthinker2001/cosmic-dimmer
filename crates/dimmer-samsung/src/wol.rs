//! Wake-on-LAN: the "magic packet" (6 × `0xFF`, then the MAC 16 times) sent
//! over UDP. Samsung TVs answer it from network standby when the TV allows it
//! (see `docs/network-findings.md`: on Wi-Fi that isn't guaranteed).

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// UDP port for the magic packet (the "discard" port, the usual choice).
pub const WOL_PORT: u16 = 9;
/// Length of a magic packet.
pub const MAGIC_LEN: usize = 6 + 16 * 6;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MacAddr(pub [u8; 6]);

impl FromStr for MacAddr {
    type Err = Error;

    /// Parses `aa:bb:cc:dd:ee:ff` or `aa-bb-cc-dd-ee-ff`.
    fn from_str(s: &str) -> Result<Self> {
        let mut out = [0u8; 6];
        let mut parts = s.trim().split([':', '-']);
        for byte in &mut out {
            let part = parts.next().ok_or(Error::InvalidMac)?;
            if part.len() != 2 {
                return Err(Error::InvalidMac);
            }
            *byte = u8::from_str_radix(part, 16).map_err(|_| Error::InvalidMac)?;
        }
        if parts.next().is_some() || out == [0; 6] || out == [0xff; 6] {
            return Err(Error::InvalidMac);
        }
        Ok(Self(out))
    }
}

impl TryFrom<String> for MacAddr {
    type Error = Error;
    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<MacAddr> for String {
    fn from(mac: MacAddr) -> String {
        mac.to_string()
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(":")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MacAddr({self})")
    }
}

/// The magic packet for `mac`.
#[must_use]
pub fn magic_packet(mac: MacAddr) -> [u8; MAGIC_LEN] {
    let mut packet = [0xff; MAGIC_LEN];
    for chunk in packet[6..].as_chunks_mut::<6>().0 {
        chunk.copy_from_slice(&mac.0);
    }
    packet
}

/// Where to send it: the limited broadcast, plus the TV's own IPv4 address
/// when known (a unicast copy helps where broadcast is filtered, as long as
/// the network still knows the TV's MAC).
#[must_use]
pub fn targets(tv_ip: Option<Ipv4Addr>) -> Vec<SocketAddr> {
    let mut out = vec![SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::BROADCAST,
        WOL_PORT,
    ))];
    if let Some(ip) = tv_ip {
        out.push(SocketAddr::V4(SocketAddrV4::new(ip, WOL_PORT)));
    }
    out
}

/// Sends the magic packet to each target; succeeds if at least one send did.
///
/// # Errors
/// When the socket can't be opened or every send fails.
pub fn send(mac: MacAddr, targets: &[SocketAddr]) -> Result<usize> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_broadcast(true)?;
    let packet = magic_packet(mac);
    let mut sent = 0;
    let mut last_err = None;
    for target in targets {
        match socket.send_to(&packet, target) {
            Ok(_) => sent += 1,
            Err(e) => last_err = Some(e),
        }
    }
    match (sent, last_err) {
        (0, Some(e)) => Err(e.into()),
        _ => Ok(sent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats() {
        let mac: MacAddr = "02:1A:2b:3c:4D:5e".parse().unwrap();
        assert_eq!(mac.0, [0x02, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e]);
        assert_eq!(mac.to_string(), "02:1a:2b:3c:4d:5e");
        assert_eq!("02-1a-2b-3c-4d-5e".parse::<MacAddr>().unwrap(), mac);
        for bad in [
            "",
            "02:1a:2b:3c:4d",
            "02:1a:2b:3c:4d:5e:6f",
            "2:1a:2b:3c:4d:5e",
            "zz:1a:2b:3c:4d:5e",
            "00:00:00:00:00:00",
            "ff:ff:ff:ff:ff:ff",
        ] {
            assert!(bad.parse::<MacAddr>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn magic_packet_layout() {
        let mac = MacAddr([1, 2, 3, 4, 5, 6]);
        let packet = magic_packet(mac);
        assert_eq!(packet.len(), 102);
        assert_eq!(&packet[..6], &[0xff; 6]);
        for rep in packet[6..].chunks(6) {
            assert_eq!(rep, &[1, 2, 3, 4, 5, 6]);
        }
    }

    #[test]
    fn sends_to_loopback() {
        let rx = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mac = MacAddr([2, 0, 0, 0, 0, 7]);
        let sent = send(mac, &[rx.local_addr().unwrap()]).unwrap();
        assert_eq!(sent, 1);
        let mut buf = [0u8; 200];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], &magic_packet(mac));
    }

    #[test]
    fn targets_include_unicast() {
        let t = targets(Some(Ipv4Addr::new(192, 0, 2, 9)));
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].to_string(), "255.255.255.255:9");
        assert_eq!(t[1].to_string(), "192.0.2.9:9");
    }
}
