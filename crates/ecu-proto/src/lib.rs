//! Speeduino serial communication.
//!
//! Implements the TunerStudio Protocol 3 primary-serial framing (big-endian
//! length + payload + CRC32 trailer) and the secondary-serial `r`/0x30 och
//! read, mirroring the proven logic in pi-speeduino-logger. All I/O goes
//! through a [`Transport`] trait so tests can run against a pty-backed fake
//! ECU or in-memory pipes instead of real hardware. Command bytes are built
//! from the INI's template strings, never hardcoded.

pub mod command;
pub mod envelope;
pub mod secondary;
pub mod session;
pub mod transport;

pub use command::{Args, Template};
pub use session::{Config, Mode, Session};
pub use transport::{MockTransport, SerialTransport, Transport};

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad command template: {0}")]
    Template(String),
    #[error("response CRC mismatch")]
    CrcMismatch,
    #[error("ECU returned error code {0:#04x}")]
    EcuError(u8),
    #[error("no response from ECU")]
    Timeout,
}
