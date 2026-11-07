//! An overall deadline for a whole exchange with the TV.
//!
//! A socket timeout bounds each read on its own, so a TV (or anything
//! pretending to be one) that trickles a byte at a time could stretch an
//! exchange without end. [`Deadlined`] sets the socket's timeout to the time
//! left before every read and write, and fails with `TimedOut` once the
//! deadline has passed.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use rustls::{ClientConnection, StreamOwned};

/// Streams whose read and write timeouts we can set.
pub trait Timeouts {
    /// # Errors
    /// When the socket rejects the timeout.
    fn set_timeouts(&self, timeout: Duration) -> io::Result<()>;
}

impl Timeouts for TcpStream {
    fn set_timeouts(&self, timeout: Duration) -> io::Result<()> {
        self.set_read_timeout(Some(timeout))?;
        self.set_write_timeout(Some(timeout))
    }
}

impl Timeouts for StreamOwned<ClientConnection, TcpStream> {
    fn set_timeouts(&self, timeout: Duration) -> io::Result<()> {
        self.sock.set_timeouts(timeout)
    }
}

/// A stream with a deadline that every read and write honours.
pub struct Deadlined<S> {
    inner: S,
    deadline: Instant,
}

impl<S: Timeouts> Deadlined<S> {
    #[must_use]
    pub fn new(inner: S, deadline: Instant) -> Self {
        Self { inner, deadline }
    }

    /// Moves the deadline (for the next exchange on the same stream).
    pub fn set_deadline(&mut self, deadline: Instant) {
        self.deadline = deadline;
    }

    fn arm(&self) -> io::Result<()> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "deadline passed"));
        }
        self.inner.set_timeouts(left)
    }
}

impl<S: Read + Timeouts> Read for Deadlined<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.arm()?;
        self.inner.read(buf)
    }
}

impl<S: Write + Timeouts> Write for Deadlined<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.arm()?;
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.arm()?;
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    /// A peer that sends one byte every 100 ms forever must not keep a reader
    /// past its overall deadline, although no single read times out.
    #[test]
    fn trickling_peer_hits_the_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            while s.write_all(b"x").is_ok() {
                thread::sleep(Duration::from_millis(100));
            }
        });
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let started = Instant::now();
        let mut stream = Deadlined::new(tcp, started + Duration::from_millis(500));
        let mut sink = Vec::new();
        let err = stream.read_to_end(&mut sink).unwrap_err();
        let elapsed = started.elapsed();
        assert!(
            matches!(
                err.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ),
            "{err:?}"
        );
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
        assert_ne!(sink.len(), 0);
        drop(stream);
        peer.join().unwrap();
    }
}
