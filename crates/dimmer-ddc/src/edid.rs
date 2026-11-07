//! EDID base-block parsing: enough to identify a monitor and to prove that a
//! bus and a DRM connector see the same one. No I/O.

use std::fmt;

/// Length of the EDID base block, the part every display has.
pub const BLOCK_LEN: usize = 128;

const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

/// Why bytes are not a usable EDID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdidError {
    /// Fewer than 128 bytes.
    TooShort(usize),
    /// The fixed 8-byte header is wrong.
    BadHeader,
    /// The base block does not sum to zero modulo 256.
    BadChecksum,
}

impl fmt::Display for EdidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort(len) => write!(f, "EDID too short ({len} bytes)"),
            Self::BadHeader => f.write_str("EDID header invalid"),
            Self::BadChecksum => f.write_str("EDID checksum invalid"),
        }
    }
}

impl std::error::Error for EdidError {}

/// A validated EDID base block. Extension blocks are ignored: the base block
/// carries the identity, and comparing it is what ties a bus to a connector.
#[derive(Clone, Eq, PartialEq)]
pub struct Edid {
    base: [u8; BLOCK_LEN],
}

impl fmt::Debug for Edid {
    // The serial number stays out of logs and test output.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Edid")
            .field("manufacturer", &self.manufacturer())
            .field(
                "product_code",
                &format_args!("{:#06x}", self.product_code()),
            )
            .finish_non_exhaustive()
    }
}

impl Edid {
    /// Validate the header and checksum of the first 128 bytes.
    ///
    /// # Errors
    /// Fails when the bytes are too short, the header is wrong or the base
    /// block's checksum does not hold.
    pub fn parse(bytes: &[u8]) -> Result<Self, EdidError> {
        let base: [u8; BLOCK_LEN] = bytes
            .get(..BLOCK_LEN)
            .and_then(|b| b.try_into().ok())
            .ok_or(EdidError::TooShort(bytes.len()))?;
        if base[..8] != HEADER {
            return Err(EdidError::BadHeader);
        }
        if base.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) != 0 {
            return Err(EdidError::BadChecksum);
        }
        Ok(Self { base })
    }

    /// The validated base block.
    #[must_use]
    pub fn base_block(&self) -> &[u8; BLOCK_LEN] {
        &self.base
    }

    /// Three-letter PNP manufacturer ID (`SAM` for Samsung).
    #[must_use]
    pub fn manufacturer(&self) -> String {
        let id = u16::from_be_bytes([self.base[8], self.base[9]]);
        [10u16, 5, 0]
            .iter()
            .map(|shift| {
                let letter = u8::try_from((id >> shift) & 0x1f).unwrap_or(0);
                if (1..=26).contains(&letter) {
                    char::from(b'A' + letter - 1)
                } else {
                    '?'
                }
            })
            .collect()
    }

    /// Manufacturer's product code (little-endian in the block).
    #[must_use]
    pub fn product_code(&self) -> u16 {
        u16::from_le_bytes([self.base[10], self.base[11]])
    }

    /// Numeric serial number; zero when the display does not set one.
    #[must_use]
    pub fn serial_number(&self) -> u32 {
        u32::from_le_bytes([self.base[12], self.base[13], self.base[14], self.base[15]])
    }

    /// The monitor name descriptor (tag 0xFC), printable ASCII only.
    #[must_use]
    pub fn monitor_name(&self) -> Option<String> {
        self.descriptor_text(0xfc)
    }

    fn descriptor_text(&self, tag: u8) -> Option<String> {
        let (descriptors, _) = self.base[54..126].as_chunks::<18>();
        descriptors
            .iter()
            .find(|d| d[..3] == [0, 0, 0] && d[3] == tag)
            .map(|d| {
                // Display text from a device is untrusted: keep printable
                // ASCII, stop at the newline terminator.
                d[5..]
                    .iter()
                    .take_while(|b| **b != b'\n')
                    .map(|b| {
                        if b.is_ascii_graphic() || *b == b' ' {
                            char::from(*b)
                        } else {
                            '?'
                        }
                    })
                    .collect::<String>()
                    .trim()
                    .to_owned()
            })
            .filter(|name| !name.is_empty())
    }

    /// Short human label: the monitor name, else manufacturer and product.
    #[must_use]
    pub fn label(&self) -> String {
        self.monitor_name()
            .unwrap_or_else(|| format!("{} {:04x}", self.manufacturer(), self.product_code()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Base block of the Samsung QN55LS03H Frame TV on this project's host,
    /// read from `/sys/class/drm/card0-HDMI-A-1/edid` on 2026-11-05. The
    /// numeric serial (bytes 12..16) was zeroed before recording (it was
    /// already zero; there is no serial descriptor) and the checksum
    /// recomputed. The 128-byte CEA extension that follows is not recorded.
    pub(crate) const SAMSUNG_FRAME: [u8; BLOCK_LEN] = [
        0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x4c, 0x2d, 0x8d, 0x7a, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x24, 0x01, 0x03, 0x80, 0x7a, 0x45, 0x78, 0x0a, 0xb3, 0x61, 0xab, 0x4f, 0x46,
        0xa8, 0x27, 0x0b, 0x50, 0x54, 0xbd, 0xef, 0x80, 0x71, 0x4f, 0x81, 0xc0, 0x81, 0x00, 0x81,
        0x80, 0x95, 0x00, 0xa9, 0xc0, 0xb3, 0x00, 0xd1, 0xc0, 0x08, 0xe8, 0x00, 0x30, 0xf2, 0x70,
        0x5a, 0x80, 0xb0, 0x58, 0x8a, 0x00, 0x50, 0x1d, 0x74, 0x00, 0x00, 0x1e, 0x56, 0x5e, 0x00,
        0xa0, 0xa0, 0xa0, 0x29, 0x50, 0x30, 0x20, 0x35, 0x00, 0x50, 0x1d, 0x74, 0x00, 0x00, 0x1a,
        0x00, 0x00, 0x00, 0xfd, 0x00, 0x18, 0xf0, 0x0f, 0xff, 0x86, 0x00, 0x0a, 0x20, 0x20, 0x20,
        0x20, 0x20, 0x20, 0x00, 0x00, 0x00, 0xfc, 0x00, 0x53, 0x41, 0x4d, 0x53, 0x55, 0x4e, 0x47,
        0x0a, 0x20, 0x20, 0x20, 0x20, 0x20, 0x01, 0x37,
    ];

    /// A second, made-up monitor: the Samsung block with another
    /// manufacturer ("DEL"), product 0x1234, serial 7 and no name.
    pub(crate) fn other_monitor() -> [u8; BLOCK_LEN] {
        let mut block = SAMSUNG_FRAME;
        let id: u16 = (0x4 << 10) | (0x5 << 5) | 0xc; // D E L
        block[8..10].copy_from_slice(&id.to_be_bytes());
        block[10..12].copy_from_slice(&0x1234u16.to_le_bytes());
        block[12..16].copy_from_slice(&7u32.to_le_bytes());
        block[111] = 0x10; // name descriptor becomes a dummy one
        fix_checksum(&mut block);
        block
    }

    pub(crate) fn fix_checksum(block: &mut [u8; BLOCK_LEN]) {
        let sum = block[..127].iter().fold(0u8, |s, b| s.wrapping_add(*b));
        block[127] = 0u8.wrapping_sub(sum);
    }

    #[test]
    fn recorded_samsung_block_parses() {
        let edid = Edid::parse(&SAMSUNG_FRAME).unwrap();
        assert_eq!(edid.manufacturer(), "SAM");
        assert_eq!(edid.product_code(), 0x7a8d);
        assert_eq!(edid.serial_number(), 0);
        assert_eq!(edid.monitor_name().as_deref(), Some("SAMSUNG"));
        assert_eq!(edid.label(), "SAMSUNG");
    }

    #[test]
    fn extension_blocks_are_ignored() {
        let mut bytes = SAMSUNG_FRAME.to_vec();
        bytes.extend_from_slice(&[0x02; BLOCK_LEN]);
        assert_eq!(
            Edid::parse(&bytes).unwrap(),
            Edid::parse(&SAMSUNG_FRAME).unwrap()
        );
    }

    #[test]
    fn corrupt_blocks_are_rejected() {
        assert_eq!(
            Edid::parse(&SAMSUNG_FRAME[..100]),
            Err(EdidError::TooShort(100))
        );
        let mut bad = SAMSUNG_FRAME;
        bad[1] = 0;
        assert_eq!(Edid::parse(&bad), Err(EdidError::BadHeader));
        let mut bad = SAMSUNG_FRAME;
        bad[20] ^= 1;
        assert_eq!(Edid::parse(&bad), Err(EdidError::BadChecksum));
        // An all-0xff read (nothing at 0x50) is not an EDID.
        assert_eq!(Edid::parse(&[0xff; 128]), Err(EdidError::BadHeader));
    }

    #[test]
    fn other_monitor_fields_and_label() {
        let edid = Edid::parse(&other_monitor()).unwrap();
        assert_eq!(edid.manufacturer(), "DEL");
        assert_eq!(edid.product_code(), 0x1234);
        assert_eq!(edid.serial_number(), 7);
        assert_eq!(edid.monitor_name(), None);
        assert_eq!(edid.label(), "DEL 1234");
    }

    #[test]
    fn untrusted_name_text_is_sanitized() {
        let mut block = SAMSUNG_FRAME;
        block[116] = 0x1b; // ESC in place of the second S
        fix_checksum(&mut block);
        let name = Edid::parse(&block).unwrap().monitor_name().unwrap();
        assert_eq!(name, "SAM?UNG");
    }

    #[test]
    fn debug_output_does_not_print_serials() {
        let edid = Edid::parse(&other_monitor()).unwrap();
        assert!(!format!("{edid:?}").contains("serial"));
    }
}
