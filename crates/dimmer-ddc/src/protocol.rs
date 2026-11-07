//! DDC/CI frames (VESA DDC/CI 1.1, MCCS) for Get/Set VCP Feature, and the
//! mapping between MCCS raw values and percent levels. No I/O.

use std::fmt;

use dimmer_core::Level;

/// 7-bit I2C address of the display's DDC/CI endpoint (0x6E/0x6F on the wire).
pub const DDC_ADDRESS: u16 = 0x37;
/// 7-bit I2C address of the EDID EEPROM (0xA0/0xA1 on the wire).
pub const EDID_ADDRESS: u16 = 0x50;

/// Destination byte that seeds the checksum of host-to-display frames.
const DISPLAY_WRITE: u8 = 0x6e;
/// Source byte the host puts first in every frame it sends.
const HOST_SOURCE: u8 = 0x51;
/// Virtual host address that seeds the checksum of display replies.
const HOST_VIRTUAL: u8 = 0x50;

const GET_VCP: u8 = 0x01;
const GET_VCP_REPLY: u8 = 0x02;
const SET_VCP: u8 = 0x03;

/// Bytes the host reads back for a Get VCP Feature reply.
pub const GET_VCP_REPLY_LEN: usize = 11;

fn xor(seed: u8, bytes: &[u8]) -> u8 {
    bytes.iter().fold(seed, |acc, b| acc ^ b)
}

/// The frame written to [`DDC_ADDRESS`] to read VCP `code`.
#[must_use]
pub fn get_vcp_request(code: u8) -> [u8; 5] {
    let mut frame = [HOST_SOURCE, 0x82, GET_VCP, code, 0];
    frame[4] = xor(DISPLAY_WRITE, &frame[..4]);
    frame
}

/// The frame written to [`DDC_ADDRESS`] to set VCP `code` to `value`.
#[must_use]
pub fn set_vcp_request(code: u8, value: u16) -> [u8; 7] {
    let [hi, lo] = value.to_be_bytes();
    let mut frame = [HOST_SOURCE, 0x84, SET_VCP, code, hi, lo, 0];
    frame[6] = xor(DISPLAY_WRITE, &frame[..6]);
    frame
}

/// A VCP continuous value as the display reports it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VcpValue {
    pub current: u16,
    pub maximum: u16,
}

impl VcpValue {
    /// The current value as a level, when it can be one. `None` when the
    /// maximum is zero (unusable) or the display reports zero, which a
    /// [`Level`] cannot represent; the raw value stays available.
    #[must_use]
    pub fn level(self) -> Option<Level> {
        if self.maximum == 0 || self.current == 0 {
            return None;
        }
        let percent = (u32::from(self.current.min(self.maximum)) * 100
            + u32::from(self.maximum) / 2)
            / u32::from(self.maximum);
        Level::new(u8::try_from(percent.clamp(1, 100)).unwrap_or(100)).ok()
    }
}

/// The raw value to write for `level` on a control whose maximum is
/// `maximum`. Rounds up, so a nonzero level never becomes raw zero.
#[must_use]
pub fn level_to_raw(level: Level, maximum: u16) -> u16 {
    let raw = (u32::from(level.percent()) * u32::from(maximum)).div_ceil(100);
    u16::try_from(raw).unwrap_or(maximum)
}

/// A reply that is not a usable Get VCP Feature answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyError {
    /// The DDC/CI null message: the display had nothing to say (yet).
    Null,
    /// The display says it does not support the code.
    Unsupported(u8),
    /// First byte is not the display's address (often 0xff: nobody drove the bus).
    BadSource(u8),
    /// Length byte malformed or longer than what was read.
    BadLength,
    BadChecksum,
    /// A reply to some other request.
    UnexpectedOpcode(u8),
    /// A reply about another VCP code.
    WrongCode {
        expected: u8,
        got: u8,
    },
}

impl fmt::Display for ReplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("display sent the DDC/CI null message"),
            Self::Unsupported(code) => write!(f, "display does not support VCP {code:#04x}"),
            Self::BadSource(byte) => write!(f, "reply source byte {byte:#04x} is not 0x6e"),
            Self::BadLength => f.write_str("reply length is malformed"),
            Self::BadChecksum => f.write_str("reply checksum is wrong"),
            Self::UnexpectedOpcode(op) => write!(f, "reply opcode {op:#04x} is not Get VCP"),
            Self::WrongCode { expected, got } => {
                write!(f, "reply is for VCP {got:#04x}, asked {expected:#04x}")
            }
        }
    }
}

/// Parse a Get VCP Feature reply for `code`, as read from [`DDC_ADDRESS`].
///
/// # Errors
/// Any [`ReplyError`]; `Null` and the framing errors are worth a retry,
/// `Unsupported` is not.
pub fn parse_get_vcp_reply(code: u8, frame: &[u8]) -> Result<VcpValue, ReplyError> {
    let (&source, rest) = frame.split_first().ok_or(ReplyError::BadLength)?;
    if source != DISPLAY_WRITE {
        return Err(ReplyError::BadSource(source));
    }
    let &length_byte = rest.first().ok_or(ReplyError::BadLength)?;
    if length_byte & 0x80 == 0 {
        return Err(ReplyError::BadLength);
    }
    let length = usize::from(length_byte & 0x7f);
    let end = 2 + length;
    let &checksum = frame.get(end).ok_or(ReplyError::BadLength)?;
    if xor(HOST_VIRTUAL, &frame[..end]) != checksum {
        return Err(ReplyError::BadChecksum);
    }
    let payload = &frame[2..end];
    match payload {
        [] => Err(ReplyError::Null),
        [
            GET_VCP_REPLY,
            result,
            got,
            _kind,
            max_hi,
            max_lo,
            cur_hi,
            cur_lo,
        ] => {
            if *got != code {
                return Err(ReplyError::WrongCode {
                    expected: code,
                    got: *got,
                });
            }
            if *result != 0 {
                return Err(ReplyError::Unsupported(code));
            }
            Ok(VcpValue {
                current: u16::from_be_bytes([*cur_hi, *cur_lo]),
                maximum: u16::from_be_bytes([*max_hi, *max_lo]),
            })
        }
        [GET_VCP_REPLY, ..] => Err(ReplyError::BadLength),
        [opcode, ..] => Err(ReplyError::UnexpectedOpcode(*opcode)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Frames below are written out byte for byte from the DDC/CI 1.1 frame
    // layout, checksums worked by hand, so the tests do not reuse the code's
    // own checksum routine. No real exchange has succeeded on this project's
    // host (docs/ddc-findings.md), so none is a capture from the Samsung.

    /// Host -> display: Get VCP 0x10 (brightness).
    const GET_BRIGHTNESS: [u8; 5] = [0x51, 0x82, 0x01, 0x10, 0xac];
    /// Host -> display: Set VCP 0x10 to 50.
    const SET_BRIGHTNESS_50: [u8; 7] = [0x51, 0x84, 0x03, 0x10, 0x00, 0x32, 0x9a];
    /// Display -> host: VCP 0x10, continuous, max 100, current 50.
    pub(crate) const REPLY_BRIGHTNESS_50_OF_100: [u8; 11] = [
        0x6e, 0x88, 0x02, 0x00, 0x10, 0x00, 0x00, 0x64, 0x00, 0x32, 0xf2,
    ];
    /// Display -> host: VCP 0x10 unsupported (result code 1).
    const REPLY_UNSUPPORTED: [u8; 11] = [
        0x6e, 0x88, 0x02, 0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa5,
    ];
    /// Display -> host: null message, padded as an 11-byte read returns it.
    pub(crate) const REPLY_NULL: [u8; 11] = [0x6e, 0x80, 0xbe, 0, 0, 0, 0, 0, 0, 0, 0];

    #[test]
    fn requests_match_recorded_frames() {
        assert_eq!(get_vcp_request(0x10), GET_BRIGHTNESS);
        assert_eq!(set_vcp_request(0x10, 50), SET_BRIGHTNESS_50);
    }

    #[test]
    fn reply_parses() {
        assert_eq!(
            parse_get_vcp_reply(0x10, &REPLY_BRIGHTNESS_50_OF_100),
            Ok(VcpValue {
                current: 50,
                maximum: 100
            })
        );
    }

    #[test]
    fn null_unsupported_and_garbage_are_told_apart() {
        assert_eq!(
            parse_get_vcp_reply(0x10, &REPLY_NULL),
            Err(ReplyError::Null)
        );
        assert_eq!(
            parse_get_vcp_reply(0x10, &REPLY_UNSUPPORTED),
            Err(ReplyError::Unsupported(0x10))
        );
        assert_eq!(
            parse_get_vcp_reply(0x10, &[0xff; 11]),
            Err(ReplyError::BadSource(0xff))
        );
        let mut corrupt = REPLY_BRIGHTNESS_50_OF_100;
        corrupt[9] = 0x33;
        assert_eq!(
            parse_get_vcp_reply(0x10, &corrupt),
            Err(ReplyError::BadChecksum)
        );
        assert_eq!(
            parse_get_vcp_reply(0x12, &REPLY_BRIGHTNESS_50_OF_100),
            Err(ReplyError::WrongCode {
                expected: 0x12,
                got: 0x10
            })
        );
        assert_eq!(
            parse_get_vcp_reply(0x10, &REPLY_BRIGHTNESS_50_OF_100[..6]),
            Err(ReplyError::BadLength)
        );
        assert_eq!(parse_get_vcp_reply(0x10, &[]), Err(ReplyError::BadLength));
    }

    #[test]
    fn levels_never_map_to_raw_zero() {
        let one = Level::new(1).unwrap();
        assert_eq!(level_to_raw(one, 100), 1);
        assert_eq!(level_to_raw(one, 50), 1);
        assert_eq!(level_to_raw(one, 1), 1);
        assert_eq!(level_to_raw(Level::FULL, 100), 100);
        assert_eq!(level_to_raw(Level::FULL, 0xffff), 0xffff);
        assert_eq!(level_to_raw(Level::new(50).unwrap(), 255), 128);
    }

    #[test]
    fn raw_values_map_back_to_levels() {
        let value = |current, maximum| VcpValue { current, maximum }.level();
        assert_eq!(value(50, 100), Level::new(50).ok());
        assert_eq!(value(128, 255), Level::new(50).ok());
        assert_eq!(value(1, 255), Level::new(1).ok());
        assert_eq!(value(0, 100), None);
        assert_eq!(value(10, 0), None);
        // A current above the maximum is clamped, not trusted.
        assert_eq!(value(500, 100), Some(Level::FULL));
        for percent in 1..=100 {
            let level = Level::new(percent).unwrap();
            for maximum in [100, 255, 1000] {
                let raw = level_to_raw(level, maximum);
                assert_eq!(
                    VcpValue {
                        current: raw,
                        maximum
                    }
                    .level(),
                    Some(level),
                    "{percent}% of {maximum}"
                );
            }
        }
    }
}
