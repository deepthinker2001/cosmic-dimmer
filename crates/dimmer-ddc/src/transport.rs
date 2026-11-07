//! Access to one I2C bus: the Linux i2c-dev implementation and the trait the
//! worker uses, so the protocol logic can be tested against recorded frames.

use std::{
    fs::File,
    io::{self, Read, Write},
    os::fd::AsFd,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

/// One open I2C bus. Each call is one I2C transaction to a 7-bit address.
pub trait Bus: Send {
    /// Write `data` to `address`.
    ///
    /// # Errors
    /// The kernel's error: on a bus where nothing acknowledges `address`,
    /// usually `EIO`, `ENXIO` or `EREMOTEIO`.
    fn write(&mut self, address: u16, data: &[u8]) -> io::Result<()>;

    /// Fill `buf` from `address`.
    ///
    /// # Errors
    /// The kernel's error, or `UnexpectedEof` on a short read.
    fn read(&mut self, address: u16, buf: &mut [u8]) -> io::Result<()>;

    /// Take the bus's advisory cross-process lock, waiting until `deadline`.
    ///
    /// # Errors
    /// `WouldBlock` when another process still holds it at the deadline.
    fn lock(&mut self, _deadline: Instant) -> io::Result<()> {
        Ok(())
    }
}

/// Opens buses by number. The worker opens a bus per job and closes it
/// afterwards, so no file descriptor is held while idle (the stock daemon
/// had to add an idle expiry for exactly that, after DP-MST teardown hangs).
pub trait BusOpener: Send + Sync {
    /// # Errors
    /// The error from opening the device node.
    fn open(&self, bus: u32) -> io::Result<Box<dyn Bus>>;
}

/// `/dev/i2c-N` through the kernel's i2c-dev interface. Needs read and write
/// access to the node (udev rule and group, R4/R8), never root.
#[derive(Clone, Debug)]
pub struct I2cDevOpener {
    dev_root: PathBuf,
}

impl I2cDevOpener {
    /// `dev_root` is `/dev` outside tests.
    #[must_use]
    pub fn new(dev_root: impl Into<PathBuf>) -> Self {
        Self {
            dev_root: dev_root.into(),
        }
    }

    #[must_use]
    pub fn node(&self, bus: u32) -> PathBuf {
        self.dev_root.join(format!("i2c-{bus}"))
    }
}

impl BusOpener for I2cDevOpener {
    fn open(&self, bus: u32) -> io::Result<Box<dyn Bus>> {
        Ok(Box::new(I2cDev::open(&self.node(bus))?))
    }
}

/// An open `/dev/i2c-N`.
#[derive(Debug)]
pub struct I2cDev {
    file: File,
    address: Option<u16>,
}

impl I2cDev {
    /// Open the node read-write. Never follows the open with any bus traffic.
    ///
    /// # Errors
    /// The error from `open(2)` (`EACCES` without the udev rule and group).
    pub fn open(node: &Path) -> io::Result<Self> {
        let file = File::options().read(true).write(true).open(node)?;
        Ok(Self {
            file,
            address: None,
        })
    }

    fn select(&mut self, address: u16) -> io::Result<()> {
        if self.address != Some(address) {
            ioctl::set_target_address(self.file.as_fd(), address)?;
            self.address = Some(address);
        }
        Ok(())
    }
}

impl Bus for I2cDev {
    fn write(&mut self, address: u16, data: &[u8]) -> io::Result<()> {
        self.select(address)?;
        let written = self.file.write(data)?;
        if written == data.len() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("short I2C write ({written} of {} bytes)", data.len()),
            ))
        }
    }

    fn read(&mut self, address: u16, buf: &mut [u8]) -> io::Result<()> {
        self.select(address)?;
        // One read(2) is one I2C read transaction; a loop would issue more.
        let read = self.file.read(buf)?;
        if read == buf.len() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("short I2C read ({read} of {} bytes)", buf.len()),
            ))
        }
    }

    /// `flock(2)` on the device node: advisory, so it orders only tools that
    /// take the same lock. The stock daemon's ddc-hi does not (see README).
    fn lock(&mut self, deadline: Instant) -> io::Result<()> {
        use rustix::fs::{FlockOperation, flock};
        loop {
            match flock(&self.file, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => return Ok(()),
                Err(rustix::io::Errno::WOULDBLOCK) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(rustix::io::Errno::WOULDBLOCK) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "bus is locked by another process",
                    ));
                }
                Err(err) => return Err(err.into()),
            }
        }
        // The lock is released when the file closes at the end of the job.
    }
}

/// The only unsafe code in the crate: one ioctl with an integer argument.
mod ioctl {
    #![expect(
        unsafe_code,
        reason = "i2c-dev's I2C_SLAVE ioctl has no safe wrapper in rustix"
    )]

    use std::{io, os::fd::BorrowedFd};

    use rustix::ioctl::{IntegerSetter, Opcode, ioctl};

    /// `I2C_SLAVE` from `<linux/i2c-dev.h>`: set the 7-bit target address of
    /// later read(2)/write(2) calls. Unlike `I2C_SLAVE_FORCE` (never used
    /// here), it fails with `EBUSY` when a kernel driver (such as ddcci)
    /// already owns the address, rather than going around that driver.
    const I2C_SLAVE: Opcode = 0x0703;

    pub(super) fn set_target_address(fd: BorrowedFd<'_>, address: u16) -> io::Result<()> {
        if address > 0x7f {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "I2C address is not 7-bit",
            ));
        }
        // SAFETY: I2C_SLAVE takes its argument by value as an unsigned
        // long, not as a pointer (drivers/i2c/i2c-dev.c, i2cdev_ioctl), and
        // the value was checked to be a valid 7-bit address above. The fd is
        // borrowed for the call only.
        unsafe {
            let setter = IntegerSetter::<I2C_SLAVE>::new_usize(usize::from(address));
            ioctl(fd, setter)
        }
        .map_err(io::Error::from)
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A scripted bus for tests: each step names the transaction expected
    //! and what the "display" answers.

    use std::{
        collections::VecDeque,
        io,
        sync::{Arc, Mutex},
        time::Instant,
    };

    use super::{Bus, BusOpener};

    #[derive(Clone, Debug)]
    pub enum Step {
        /// Expect a write of exactly these bytes; answer with the errno.
        Write(u16, Vec<u8>, Option<i32>),
        /// Expect a read; answer with these bytes or the errno.
        Read(u16, Result<Vec<u8>, i32>),
        /// Hold the next transaction until the test opens the gate.
        Gate(Arc<Gate>),
    }

    /// A door the worker waits at, so a test can act while it is mid-job.
    #[derive(Debug, Default)]
    pub struct Gate {
        state: Mutex<(bool, bool)>, // (someone waiting, open)
        changed: std::sync::Condvar,
    }

    impl Gate {
        fn pass(&self) {
            let mut state = self.state.lock().unwrap();
            state.0 = true;
            self.changed.notify_all();
            while !state.1 {
                state = self.changed.wait(state).unwrap();
            }
        }

        /// Wait until the worker is held here.
        pub fn wait_until_held(&self) {
            let mut state = self.state.lock().unwrap();
            while !state.0 {
                state = self.changed.wait(state).unwrap();
            }
        }

        pub fn open(&self) {
            self.state.lock().unwrap().1 = true;
            self.changed.notify_all();
        }
    }

    fn pass_gate(script: &Arc<Mutex<Script>>) {
        let gate = {
            let mut guard = script.lock().unwrap();
            match guard.steps.front() {
                Some(Step::Gate(gate)) => {
                    let gate = gate.clone();
                    guard.steps.pop_front();
                    gate
                }
                _ => return,
            }
        };
        gate.pass();
    }

    #[derive(Default)]
    pub struct Script {
        pub steps: VecDeque<Step>,
        /// Every transaction as it happened, for assertions.
        pub log: Vec<String>,
        pub opens: usize,
        /// Errno for opening the bus, if any.
        pub open_error: Option<i32>,
    }

    #[derive(Clone, Default)]
    pub struct FakeOpener {
        pub buses: Arc<Mutex<std::collections::HashMap<u32, Arc<Mutex<Script>>>>>,
    }

    impl FakeOpener {
        pub fn script(&self, bus: u32) -> Arc<Mutex<Script>> {
            self.buses.lock().unwrap().entry(bus).or_default().clone()
        }

        pub fn push(&self, bus: u32, steps: impl IntoIterator<Item = Step>) {
            self.script(bus).lock().unwrap().steps.extend(steps);
        }

        pub fn log(&self, bus: u32) -> Vec<String> {
            self.script(bus).lock().unwrap().log.clone()
        }

        pub fn remaining(&self, bus: u32) -> usize {
            self.script(bus).lock().unwrap().steps.len()
        }
    }

    impl BusOpener for FakeOpener {
        fn open(&self, bus: u32) -> io::Result<Box<dyn Bus>> {
            let script = self.script(bus);
            let mut guard = script.lock().unwrap();
            guard.opens += 1;
            if let Some(errno) = guard.open_error {
                return Err(io::Error::from_raw_os_error(errno));
            }
            drop(guard);
            Ok(Box::new(FakeBus { script }))
        }
    }

    struct FakeBus {
        script: Arc<Mutex<Script>>,
    }

    impl Bus for FakeBus {
        fn write(&mut self, address: u16, data: &[u8]) -> io::Result<()> {
            pass_gate(&self.script);
            let mut script = self.script.lock().unwrap();
            script.log.push(format!("w {address:#04x} {data:02x?}"));
            match script.steps.pop_front() {
                Some(Step::Write(a, bytes, errno)) if a == address && bytes == data => {
                    errno.map_or(Ok(()), |e| Err(io::Error::from_raw_os_error(e)))
                }
                other => {
                    panic!("unexpected write {address:#04x} {data:02x?}, script had {other:?}")
                }
            }
        }

        fn read(&mut self, address: u16, buf: &mut [u8]) -> io::Result<()> {
            pass_gate(&self.script);
            let mut script = self.script.lock().unwrap();
            script.log.push(format!("r {address:#04x} {}", buf.len()));
            match script.steps.pop_front() {
                Some(Step::Read(a, Ok(bytes))) if a == address => {
                    let n = bytes.len().min(buf.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                    buf[n..].fill(0);
                    Ok(())
                }
                Some(Step::Read(a, Err(errno))) if a == address => {
                    Err(io::Error::from_raw_os_error(errno))
                }
                other => panic!("unexpected read {address:#04x}, script had {other:?}"),
            }
        }

        fn lock(&mut self, _deadline: Instant) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_node_fails_to_open_without_panicking() {
        let opener = I2cDevOpener::new("/nonexistent-dimmer-ddc-test");
        let err = opener.open(3).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(
            opener.node(3),
            Path::new("/nonexistent-dimmer-ddc-test/i2c-3")
        );
    }

    #[test]
    fn non_i2c_file_rejects_the_address_ioctl() {
        // /dev/null accepts open(O_RDWR) but not I2C_SLAVE: the ioctl path
        // runs for real and must fail cleanly (ENOTTY), not touch a bus.
        let mut dev = I2cDev::open(Path::new("/dev/null")).unwrap();
        assert!(dev.write(0x37, &[0]).is_err());
        assert!(dev.read(0x37, &mut [0; 1]).is_err());
        assert!(dev.write(0x80, &[0]).is_err());
    }
}
