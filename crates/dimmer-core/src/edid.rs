//! Monitor identity from the EDID base block, for keying saved settings.
//!
//! Parsing only: reading the EDID (over I2C, DRM or Wayland) is a backend's job.

use std::fmt;

/// Length of the EDID base block, the only part this module reads.
pub const EDID_BASE_LEN: usize = 128;

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

/// A monitor's identity as its EDID states it: the PNP manufacturer ID, a
/// fingerprint of how the monitor describes itself, and the numeric serial
/// number.
///
/// The fingerprint covers the monitor name, the screen size and the serial
/// number text from the base block, but **not the product code** when the
/// EDID names the monitor. Some displays change their product code with a
/// setting: the Samsung Frame TV reports one with Game Mode on, another with
/// it off, and a third in its 4K 30 Hz fallback. Without a monitor name
/// the product code is fingerprinted instead, so nameless monitors stay apart.
///
/// This is stable across connectors and reboots, unlike [`crate::OutputId`].
/// Many monitors report a placeholder serial; two identical such monitors
/// share one identity, and so share saved settings.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EdidId {
    manufacturer: [u8; 3],
    fingerprint: u32,
    serial: u32,
}

/// Why bytes are not a usable EDID base block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EdidError {
    TooShort(usize),
    BadHeader,
    BadChecksum,
    BadManufacturer,
}

impl fmt::Display for EdidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort(len) => write!(
                f,
                "EDID is {len} bytes; the base block needs {EDID_BASE_LEN}"
            ),
            Self::BadHeader => f.write_str("EDID does not start with the fixed header"),
            Self::BadChecksum => f.write_str("EDID base block checksum does not match"),
            Self::BadManufacturer => f.write_str("EDID manufacturer ID is not three letters"),
        }
    }
}

impl std::error::Error for EdidError {}

impl EdidId {
    /// An identity from its parts.
    ///
    /// # Errors
    /// Returns `BadManufacturer` unless `manufacturer` is three ASCII capitals.
    pub fn new(manufacturer: &str, fingerprint: u32, serial: u32) -> Result<Self, EdidError> {
        let bytes: [u8; 3] = manufacturer
            .as_bytes()
            .try_into()
            .map_err(|_| EdidError::BadManufacturer)?;
        if !bytes.iter().all(u8::is_ascii_uppercase) {
            return Err(EdidError::BadManufacturer);
        }
        Ok(Self {
            manufacturer: bytes,
            fingerprint,
            serial,
        })
    }

    /// Parse the identity from an EDID, checking the header and the base
    /// block's checksum so a corrupted read is not mistaken for another monitor.
    /// Extension blocks after the first 128 bytes are ignored.
    ///
    /// # Errors
    /// Any [`EdidError`] describing why the bytes are not a valid base block.
    pub fn from_edid(edid: &[u8]) -> Result<Self, EdidError> {
        let base = edid
            .get(..EDID_BASE_LEN)
            .ok_or(EdidError::TooShort(edid.len()))?;
        if base[..8] != HEADER {
            return Err(EdidError::BadHeader);
        }
        if base.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
            return Err(EdidError::BadChecksum);
        }
        // Bytes 8-9, big-endian: a zero bit, then three 5-bit letters, 1 = 'A'.
        let packed = u16::from_be_bytes([base[8], base[9]]);
        let mut manufacturer = [0u8; 3];
        for (index, shift) in [10u16, 5, 0].into_iter().enumerate() {
            let letter = u8::try_from((packed >> shift) & 0x1F).unwrap_or(0);
            if !(1..=26).contains(&letter) {
                return Err(EdidError::BadManufacturer);
            }
            manufacturer[index] = b'A' + letter - 1;
        }
        Ok(Self {
            manufacturer,
            fingerprint: fingerprint(base),
            serial: u32::from_le_bytes([base[12], base[13], base[14], base[15]]),
        })
    }

    /// The three-letter PNP manufacturer ID, such as `SAM`.
    #[must_use]
    pub fn manufacturer(&self) -> &str {
        // Constructors admit only ASCII capitals.
        std::str::from_utf8(&self.manufacturer).unwrap_or("???")
    }

    /// The fingerprint of the monitor's self-description; see [`EdidId`].
    #[must_use]
    pub const fn fingerprint(&self) -> u32 {
        self.fingerprint
    }

    #[must_use]
    pub const fn serial(&self) -> u32 {
        self.serial
    }

    /// The inverse of [`EdidId::entry_name`]: `PNP-FFFFFFFF-SSSSSSSS` in
    /// upper-case hex. Anything else, including a near miss, is `None`.
    #[must_use]
    pub fn from_entry_name(text: &str) -> Option<Self> {
        let mut parts = text.split('-');
        let (manufacturer, fingerprint, serial) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || fingerprint.len() != 8 || serial.len() != 8 {
            return None;
        }
        let id = Self::new(
            manufacturer,
            u32::from_str_radix(fingerprint, 16).ok()?,
            u32::from_str_radix(serial, 16).ok()?,
        )
        .ok()?;
        // from_str_radix accepts a leading '+'; the round trip refuses it.
        (id.entry_name() == text).then_some(id)
    }

    /// A name safe as a file name and settings entry: the manufacturer, then
    /// the fingerprint and serial as eight upper-case hex digits each, joined
    /// by hyphens.
    #[must_use]
    pub fn entry_name(&self) -> String {
        format!(
            "{}-{:08X}-{:08X}",
            self.manufacturer(),
            self.fingerprint,
            self.serial
        )
    }
}

/// Display descriptor tags (EDID 1.4, section 3.10.3).
const TAG_SERIAL_TEXT: u8 = 0xFF;
const TAG_NAME: u8 = 0xFC;

/// The text of the base block's descriptor with `tag`, if there is one: up to
/// 13 bytes, ending at a line feed, trailing spaces dropped.
fn descriptor_text(base: &[u8], tag: u8) -> Option<&[u8]> {
    (54..126).step_by(18).find_map(|offset| {
        let block = &base[offset..offset + 18];
        if block[..3] != [0, 0, 0] || block[3] != tag {
            return None;
        }
        let text = &block[5..];
        let end = text.iter().position(|&b| b == b'\n').unwrap_or(text.len());
        Some(text[..end].trim_ascii_end())
    })
}

/// 32-bit FNV-1a: small, and fixed forever, unlike `std`'s hashers, so saved
/// entry names stay valid across Rust versions.
fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u32 {
    bytes.into_iter().fold(0x811C_9DC5, |hash: u32, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// The monitor name, screen size in centimetres and serial number text, each
/// tagged; the product code stands in for the name when there is none.
fn fingerprint(base: &[u8]) -> u32 {
    let mut fields = Vec::with_capacity(64);
    match descriptor_text(base, TAG_NAME) {
        Some(name) if !name.is_empty() => {
            fields.extend_from_slice(b"name=");
            fields.extend_from_slice(name);
        }
        _ => {
            fields.extend_from_slice(b"product=");
            fields.extend_from_slice(&base[10..12]);
        }
    }
    fields.extend_from_slice(b"\0size=");
    fields.extend_from_slice(&base[21..23]);
    fields.extend_from_slice(b"\0serial=");
    fields.extend_from_slice(descriptor_text(base, TAG_SERIAL_TEXT).unwrap_or_default());
    fnv1a(fields)
}

impl fmt::Display for EdidId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.entry_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic base block: manufacturer SAM, product 0x7A8D, a made-up
    /// serial, no descriptors.
    pub(crate) fn sample_edid(serial: u32) -> Vec<u8> {
        let mut edid = vec![0u8; EDID_BASE_LEN];
        edid[..8].copy_from_slice(&HEADER);
        // S=19, A=1, M=13 -> 0b0_10011_00001_01101
        let packed: u16 = (0x13 << 10) | (0x01 << 5) | 0x0D;
        edid[8..10].copy_from_slice(&packed.to_be_bytes());
        edid[10..12].copy_from_slice(&0x7A8Du16.to_le_bytes());
        edid[12..16].copy_from_slice(&serial.to_le_bytes());
        edid[18] = 1;
        edid[19] = 3;
        fix_checksum(&mut edid);
        edid
    }

    fn fix_checksum(edid: &mut [u8]) {
        let sum = edid[..127]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        edid[127] = 0u8.wrapping_sub(sum);
    }

    /// Put a text descriptor in descriptor slot `slot` (0 to 3).
    fn set_descriptor(edid: &mut [u8], slot: usize, tag: u8, text: &[u8]) {
        let offset = 54 + slot * 18;
        let mut block = [0u8; 18];
        block[3] = tag;
        let mut body = [b' '; 13];
        body[..text.len()].copy_from_slice(text);
        if text.len() < 13 {
            body[text.len()] = b'\n';
        }
        block[5..].copy_from_slice(&body);
        edid[offset..offset + 18].copy_from_slice(&block);
        fix_checksum(edid);
    }

    fn set_product(edid: &mut [u8], product: u16) {
        edid[10..12].copy_from_slice(&product.to_le_bytes());
        fix_checksum(edid);
    }

    fn id(edid: &[u8]) -> EdidId {
        EdidId::from_edid(edid).unwrap()
    }

    /// A named, sized monitor like the Frame TV's EDID, with a made-up serial.
    fn named_edid(product: u16) -> Vec<u8> {
        let mut edid = sample_edid(0x0100_0E00);
        edid[21] = 122;
        edid[22] = 69;
        set_product(&mut edid, product);
        set_descriptor(&mut edid, 3, TAG_NAME, b"SAMSUNG");
        edid
    }

    #[test]
    fn parses_identity_and_ignores_extensions() {
        let mut edid = sample_edid(0x1234_5678);
        edid.extend_from_slice(&[0xAA; 128]);
        let id = id(&edid);
        assert_eq!(id.manufacturer(), "SAM");
        assert_eq!(id.serial(), 0x1234_5678);
        let name = id.entry_name();
        assert_eq!(name.len(), "SAM-00000000-12345678".len());
        assert!(
            name.starts_with("SAM-") && name.ends_with("-12345678"),
            "{name}"
        );
        assert_eq!(
            id,
            EdidId::new("SAM", id.fingerprint(), 0x1234_5678).unwrap()
        );
    }

    #[test]
    fn fingerprint_is_fixed() {
        // Saved entries are named from it, so it must never change: this is
        // FNV-1a of "name=SAMSUNG\0size=\x7a\x45\0serial=".
        assert_eq!(fnv1a(*b""), 0x811C_9DC5);
        assert_eq!(fnv1a(*b"a"), 0xE40C_292C);
        let expected = fnv1a(*b"name=SAMSUNG\0size=\x7a\x45\0serial=");
        assert_eq!(expected, 0x2767_23A9);
        assert_eq!(id(&named_edid(0x7A08)).fingerprint(), expected);
    }

    #[test]
    fn a_named_monitor_keeps_its_identity_across_product_codes() {
        // The Frame TV's product codes: Game Mode off, 4K 30 Hz fallback, on.
        let ids: Vec<_> = [0x7A8D, 0x7A8C, 0x7A08]
            .map(|product| id(&named_edid(product)))
            .into();
        assert!(ids.windows(2).all(|pair| pair[0] == pair[1]), "{ids:?}");
    }

    #[test]
    fn name_size_and_serial_text_tell_monitors_apart() {
        let base = id(&named_edid(0x7A08));

        let mut other_name = named_edid(0x7A08);
        set_descriptor(&mut other_name, 3, TAG_NAME, b"Odyssey G70NC");
        assert_ne!(id(&other_name), base);

        let mut other_size = named_edid(0x7A08);
        other_size[21] = 143;
        fix_checksum(&mut other_size);
        assert_ne!(id(&other_size), base);

        let mut with_serial_text = named_edid(0x7A08);
        set_descriptor(&mut with_serial_text, 2, TAG_SERIAL_TEXT, b"H1AK500000");
        assert_ne!(id(&with_serial_text), base);
        let mut other_serial_text = with_serial_text.clone();
        set_descriptor(&mut other_serial_text, 2, TAG_SERIAL_TEXT, b"H1AK500001");
        assert_ne!(id(&other_serial_text), id(&with_serial_text));
    }

    #[test]
    fn without_a_name_the_product_code_counts() {
        let mut first = sample_edid(7);
        set_product(&mut first, 0x0001);
        let mut second = sample_edid(7);
        set_product(&mut second, 0x0002);
        assert_ne!(id(&first), id(&second));

        let mut blank_name = second.clone();
        set_descriptor(&mut blank_name, 3, TAG_NAME, b"");
        assert_eq!(id(&blank_name), id(&second), "an empty name is no name");
    }

    #[test]
    fn descriptor_text_reads_only_tagged_display_descriptors() {
        let mut edid = named_edid(0x7A08);
        assert_eq!(descriptor_text(&edid, TAG_NAME), Some(&b"SAMSUNG"[..]));
        assert_eq!(descriptor_text(&edid, TAG_SERIAL_TEXT), None);
        // A detailed timing descriptor (nonzero pixel clock) is not text.
        edid[54..56].copy_from_slice(&[0x01, 0x1D]);
        edid[57] = TAG_SERIAL_TEXT;
        fix_checksum(&mut edid);
        assert_eq!(descriptor_text(&edid, TAG_SERIAL_TEXT), None);
        // Thirteen bytes with no line feed are all text.
        set_descriptor(&mut edid, 1, TAG_SERIAL_TEXT, b"ABCDEFGHIJKLM");
        assert_eq!(
            descriptor_text(&edid, TAG_SERIAL_TEXT),
            Some(&b"ABCDEFGHIJKLM"[..])
        );
    }

    #[test]
    fn entry_names_parse_back_to_identities() {
        let id = EdidId::new("TST", 0x00AB_CDEF, 0x0102_0304).unwrap();
        assert_eq!(id.entry_name(), "TST-00ABCDEF-01020304");
        assert_eq!(EdidId::from_entry_name(&id.entry_name()), Some(id));
        for other in [
            "",
            "MOCK-1",
            "connector:HDMI-A-1",
            "TST-00abcdef-01020304",
            "TST-ABCDEF-01020304",
            "TST-00AB-01020304",
            "TST-00ABCDEF-01020304-5",
            "tst-00ABCDEF-01020304",
            "TST-00ABCDEF-0102030G",
            "TST-+0ABCDEF-01020304",
        ] {
            assert_eq!(EdidId::from_entry_name(other), None, "{other:?}");
        }
    }

    #[test]
    fn rejects_short_corrupt_or_bad_header() {
        let edid = sample_edid(1);
        assert_eq!(
            EdidId::from_edid(&edid[..100]),
            Err(EdidError::TooShort(100))
        );
        let mut corrupt = edid.clone();
        corrupt[12] ^= 1;
        assert_eq!(EdidId::from_edid(&corrupt), Err(EdidError::BadChecksum));
        let mut header = edid;
        header[0] = 1;
        header[127] = header[127].wrapping_sub(1);
        assert_eq!(EdidId::from_edid(&header), Err(EdidError::BadHeader));
    }

    #[test]
    fn rejects_bad_manufacturer() {
        let mut edid = sample_edid(1);
        edid[8] = 0;
        edid[9] = 0;
        fix_checksum(&mut edid);
        assert_eq!(EdidId::from_edid(&edid), Err(EdidError::BadManufacturer));
        assert_eq!(EdidId::new("sam", 1, 0), Err(EdidError::BadManufacturer));
        assert_eq!(EdidId::new("SAMS", 1, 0), Err(EdidError::BadManufacturer));
        assert_eq!(EdidId::new("S/M", 1, 0), Err(EdidError::BadManufacturer));
    }
}
