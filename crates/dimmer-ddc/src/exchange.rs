//! The I2C transactions of one DDC/CI exchange, with the spec's delays,
//! bounded retries and a deadline. Runs on a worker thread only.

use std::{
    io, thread,
    time::{Duration, Instant},
};

use crate::{
    DdcError,
    edid::{BLOCK_LEN, Edid},
    protocol::{
        DDC_ADDRESS, EDID_ADDRESS, GET_VCP_REPLY_LEN, ReplyError, VcpValue, get_vcp_request,
        parse_get_vcp_reply, set_vcp_request,
    },
    transport::Bus,
};

/// Delays and tries for one exchange, already scaled by the sleep multiplier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timing {
    /// Between writing a request and reading the reply (spec: 40 ms).
    pub write_to_read: Duration,
    /// After a Set VCP Feature before the bus is used again (spec: 50 ms).
    pub after_set: Duration,
    /// Before trying a failed exchange again.
    pub retry: Duration,
    /// Tries per exchange, the first included.
    pub max_tries: u8,
}

impl Timing {
    /// The DDC/CI 1.1 minimum delays times `multiplier`.
    #[must_use]
    pub fn from_spec(multiplier: f32, max_tries: u8) -> Self {
        let scale = |ms: u64| Duration::from_millis(ms).mul_f64(f64::from(multiplier));
        Self {
            write_to_read: scale(40),
            after_set: scale(50),
            retry: scale(50),
            max_tries,
        }
    }

    /// Longest an exchange can take when every try fails slowly, not
    /// counting time spent inside the kernel.
    #[must_use]
    pub fn worst_case(self) -> Duration {
        (self.write_to_read + self.retry) * u32::from(self.max_tries) + self.after_set
    }
}

pub(crate) fn io_error(op: &'static str, err: &io::Error) -> DdcError {
    DdcError::Io {
        op,
        errno: err.raw_os_error(),
        detail: err.to_string(),
    }
}

/// Read the 128-byte EDID base block at 0x50. One try: EDID reads are
/// plain EEPROM reads, and failure just means this bus is not the monitor.
pub(crate) fn read_edid(bus: &mut dyn Bus) -> Result<Edid, DdcError> {
    bus.write(EDID_ADDRESS, &[0])
        .map_err(|e| io_error("EDID offset write", &e))?;
    let mut block = [0u8; BLOCK_LEN];
    bus.read(EDID_ADDRESS, &mut block)
        .map_err(|e| io_error("EDID read", &e))?;
    Edid::parse(&block).map_err(|e| DdcError::BadEdid(e.to_string()))
}

/// Run `attempt` up to `max_tries` times until the deadline. `Ok(Err(e))`
/// from the attempt ends the loop at once (a definite answer, no retry).
fn with_retries<T>(
    timing: Timing,
    deadline: Instant,
    mut attempt: impl FnMut() -> Result<Result<T, DdcError>, DdcError>,
) -> Result<T, DdcError> {
    let mut last = DdcError::TimedOut;
    for n in 0..timing.max_tries {
        if n > 0 {
            if Instant::now() + timing.retry >= deadline {
                break;
            }
            thread::sleep(timing.retry);
        } else if Instant::now() >= deadline {
            break;
        }
        match attempt() {
            Ok(result) => return result,
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// Get VCP Feature `code`.
pub(crate) fn get_vcp(
    bus: &mut dyn Bus,
    code: u8,
    timing: Timing,
    deadline: Instant,
) -> Result<VcpValue, DdcError> {
    let request = get_vcp_request(code);
    with_retries(timing, deadline, || {
        bus.write(DDC_ADDRESS, &request)
            .map_err(|e| io_error("DDC request write", &e))?;
        thread::sleep(timing.write_to_read);
        let mut reply = [0u8; GET_VCP_REPLY_LEN];
        bus.read(DDC_ADDRESS, &mut reply)
            .map_err(|e| io_error("DDC reply read", &e))?;
        match parse_get_vcp_reply(code, &reply) {
            Ok(value) => Ok(Ok(value)),
            Err(ReplyError::Unsupported(code)) => Ok(Err(DdcError::UnsupportedCode(code))),
            Err(err) => Err(DdcError::BadReply(err)),
        }
    })
}

/// Set VCP Feature `code` to `value`. DDC/CI has no acknowledgement beyond
/// the I2C ACK, so success means the display accepted the bytes.
pub(crate) fn set_vcp(
    bus: &mut dyn Bus,
    code: u8,
    value: u16,
    timing: Timing,
    deadline: Instant,
) -> Result<(), DdcError> {
    let request = set_vcp_request(code, value);
    let result = with_retries(timing, deadline, || {
        bus.write(DDC_ADDRESS, &request)
            .map(Ok)
            .map_err(|e| io_error("DDC set write", &e))
    });
    // Give the display its quiet time even after a failure.
    thread::sleep(timing.after_set);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        edid::tests::SAMSUNG_FRAME,
        protocol::tests::{REPLY_BRIGHTNESS_50_OF_100, REPLY_NULL},
        transport::{
            BusOpener,
            fake::{FakeOpener, Step},
        },
    };

    const EIO: i32 = 5;

    fn zero(max_tries: u8) -> Timing {
        Timing {
            write_to_read: Duration::ZERO,
            after_set: Duration::ZERO,
            retry: Duration::ZERO,
            max_tries,
        }
    }

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    fn get_req() -> Step {
        Step::Write(DDC_ADDRESS, get_vcp_request(0x10).to_vec(), None)
    }

    #[test]
    fn spec_timing_scales() {
        let t = Timing::from_spec(2.0, 3);
        assert_eq!(t.write_to_read, Duration::from_millis(80));
        assert_eq!(t.after_set, Duration::from_millis(100));
        assert_eq!(t.worst_case(), Duration::from_millis(3 * 180 + 100));
    }

    #[test]
    fn edid_read_is_offset_write_then_block_read() {
        let opener = FakeOpener::default();
        opener.push(
            3,
            [
                Step::Write(EDID_ADDRESS, vec![0], None),
                Step::Read(EDID_ADDRESS, Ok(SAMSUNG_FRAME.to_vec())),
            ],
        );
        let edid = read_edid(opener.open(3).unwrap().as_mut()).unwrap();
        assert_eq!(edid.product_code(), 0x7a8d);

        // Nothing at 0x50: the offset write is not acknowledged.
        opener.push(4, [Step::Write(EDID_ADDRESS, vec![0], Some(6))]);
        let err = read_edid(opener.open(4).unwrap().as_mut()).unwrap_err();
        assert!(
            matches!(err, DdcError::Io { errno: Some(6), .. }),
            "{err:?}"
        );
    }

    #[test]
    fn get_vcp_retries_null_then_succeeds() {
        let opener = FakeOpener::default();
        opener.push(
            3,
            [
                get_req(),
                Step::Read(DDC_ADDRESS, Ok(REPLY_NULL.to_vec())),
                get_req(),
                Step::Read(DDC_ADDRESS, Ok(REPLY_BRIGHTNESS_50_OF_100.to_vec())),
            ],
        );
        let value = get_vcp(opener.open(3).unwrap().as_mut(), 0x10, zero(3), far()).unwrap();
        assert_eq!((value.current, value.maximum), (50, 100));
        assert_eq!(opener.remaining(3), 0);
    }

    #[test]
    fn get_vcp_gives_up_after_max_tries_of_eio() {
        // The Samsung on this host: the request write itself fails with EIO.
        let opener = FakeOpener::default();
        let eio = Step::Write(DDC_ADDRESS, get_vcp_request(0x10).to_vec(), Some(EIO));
        opener.push(3, [eio.clone(), eio.clone(), eio]);
        let err = get_vcp(opener.open(3).unwrap().as_mut(), 0x10, zero(3), far()).unwrap_err();
        assert!(
            matches!(
                err,
                DdcError::Io {
                    errno: Some(EIO),
                    ..
                }
            ),
            "{err:?}"
        );
        assert_eq!(opener.log(3).len(), 3, "exactly max_tries writes");
    }

    #[test]
    fn unsupported_code_is_final() {
        let opener = FakeOpener::default();
        let unsupported = [
            0x6e, 0x88, 0x02, 0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa5,
        ];
        opener.push(
            3,
            [get_req(), Step::Read(DDC_ADDRESS, Ok(unsupported.to_vec()))],
        );
        let err = get_vcp(opener.open(3).unwrap().as_mut(), 0x10, zero(3), far()).unwrap_err();
        assert_eq!(err, DdcError::UnsupportedCode(0x10));
    }

    #[test]
    fn deadline_stops_retries() {
        let opener = FakeOpener::default();
        let timing = Timing {
            retry: Duration::from_secs(10),
            ..zero(5)
        };
        opener.push(
            3,
            [Step::Write(
                DDC_ADDRESS,
                get_vcp_request(0x10).to_vec(),
                Some(EIO),
            )],
        );
        let start = Instant::now();
        let deadline = start + Duration::from_millis(50);
        assert!(get_vcp(opener.open(3).unwrap().as_mut(), 0x10, timing, deadline).is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(opener.log(3).len(), 1);
        // A deadline already past means no bus traffic at all.
        assert_eq!(
            get_vcp(opener.open(3).unwrap().as_mut(), 0x10, zero(3), start),
            Err(DdcError::TimedOut)
        );
        assert_eq!(opener.log(3).len(), 1);
    }

    #[test]
    fn set_vcp_writes_the_frame() {
        let opener = FakeOpener::default();
        opener.push(
            3,
            [Step::Write(
                DDC_ADDRESS,
                vec![0x51, 0x84, 0x03, 0x10, 0x00, 0x32, 0x9a],
                None,
            )],
        );
        set_vcp(opener.open(3).unwrap().as_mut(), 0x10, 50, zero(2), far()).unwrap();
        assert_eq!(opener.remaining(3), 0);
    }
}
