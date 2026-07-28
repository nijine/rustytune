//! Byte transport abstraction: real serial ports and pty-backed fakes in
//! production/integration, scripted in-memory pipes in unit tests.
//!
//! The Unix implementation deliberately mirrors pi-speeduino-logger's
//! open_serial(): plain `open(2)` + raw termios + `poll(2)`. Unlike the
//! serialport crate's macOS backend (whose IOSSIOSPEED ioctl fails with
//! ENOTTY on ptys), this works for USB adapters and fake-ECU ptys alike.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

pub trait Transport: Send {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()>;

    /// Read whatever is available, blocking up to the transport's timeout.
    /// `Ok(0)` means the timeout elapsed with no data (not EOF).
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;

    /// Discard any pending input (after a CRC error, so the next response
    /// starts on a clean boundary).
    fn flush_input(&mut self) -> io::Result<()>;
}

/// Per-read blocking granularity. Callers loop against their own deadline.
const READ_TIMEOUT: Duration = Duration::from_millis(50);

pub struct SerialTransport {
    file: File,
}

fn baud_constant(baud: u32) -> io::Result<libc::speed_t> {
    Ok(match baud {
        9600 => libc::B9600,
        19200 => libc::B19200,
        38400 => libc::B38400,
        57600 => libc::B57600,
        115_200 => libc::B115200,
        230_400 => libc::B230400,
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported baud rate {other}"),
            ));
        }
    })
}

impl SerialTransport {
    pub fn open(path: &str, baud: u32) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
            .open(path)?;
        let fd = file.as_raw_fd();

        // A stale simulator symlink can have its old pty number reused by
        // this process's controlling terminal. Never put stdin's terminal
        // into serial/raw mode: that would disable terminal input and ISIG,
        // making even Ctrl+C appear to stop working.
        unsafe {
            let mut opened: libc::stat = std::mem::zeroed();
            let mut stdin: libc::stat = std::mem::zeroed();
            if libc::fstat(fd, &mut opened) == 0
                && libc::fstat(libc::STDIN_FILENO, &mut stdin) == 0
                && libc::isatty(libc::STDIN_FILENO) == 1
                && opened.st_rdev == stdin.st_rdev
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "refusing to use the server terminal as a serial port",
                ));
            }
        }

        // SAFETY: fd is a valid open descriptor; termios is a plain struct
        // fully initialized by tcgetattr before use.
        unsafe {
            let mut tio: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut tio) != 0 {
                return Err(io::Error::last_os_error());
            }
            libc::cfmakeraw(&mut tio);
            tio.c_cc[libc::VMIN] = 0;
            tio.c_cc[libc::VTIME] = 0;
            let speed = baud_constant(baud)?;
            if libc::cfsetspeed(&mut tio, speed) != 0
                || libc::tcsetattr(fd, libc::TCSANOW, &tio) != 0
            {
                return Err(io::Error::last_os_error());
            }
            libc::tcflush(fd, libc::TCIOFLUSH);
        }
        Ok(SerialTransport { file })
    }

    fn poll(&self, events: libc::c_short, timeout: Duration) -> io::Result<bool> {
        let mut pfd = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events,
            revents: 0,
        };
        // SAFETY: pfd points to one valid pollfd for the duration of the call.
        let r = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as libc::c_int) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "serial device lost",
            ));
        }
        Ok(r > 0)
    }
}

impl Transport for SerialTransport {
    fn write_all(&mut self, mut buf: &[u8]) -> io::Result<()> {
        while !buf.is_empty() {
            match self.file.write(buf) {
                Ok(n) => buf = &buf[n..],
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.poll(libc::POLLOUT, READ_TIMEOUT)?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.poll(libc::POLLIN, READ_TIMEOUT)? {
            return Ok(0); // timeout
        }
        match self.file.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }

    fn flush_input(&mut self) -> io::Result<()> {
        // SAFETY: valid fd.
        if unsafe { libc::tcflush(self.file.as_raw_fd(), libc::TCIFLUSH) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Scripted transport for unit tests: `write_all` records what was sent,
/// `read` replays queued responses.
pub struct MockTransport {
    pub sent: Vec<Vec<u8>>,
    pub responses: std::collections::VecDeque<Vec<u8>>,
}

impl MockTransport {
    pub fn new() -> Self {
        MockTransport {
            sent: Vec::new(),
            responses: std::collections::VecDeque::new(),
        }
    }

    pub fn queue(&mut self, response: impl Into<Vec<u8>>) {
        self.responses.push_back(response.into());
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for MockTransport {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.sent.push(buf.to_vec());
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.responses.pop_front() {
            Some(mut r) => {
                let n = r.len().min(buf.len());
                buf[..n].copy_from_slice(&r[..n]);
                if n < r.len() {
                    self.responses.push_front(r.split_off(n));
                }
                Ok(n)
            }
            None => Ok(0), // behaves like a read timeout
        }
    }

    fn flush_input(&mut self) -> io::Result<()> {
        self.responses.clear();
        Ok(())
    }
}
