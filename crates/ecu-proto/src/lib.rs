//! Speeduino serial communication.
//!
//! Implements the TunerStudio Protocol 3 primary-serial framing (big-endian
//! length + payload + CRC32 trailer) and the secondary-serial `r`/0x30 och
//! read, mirroring the proven logic in pi-speeduino-logger. All I/O goes
//! through a `Transport` trait so tests can run against a pty-backed fake
//! ECU or in-memory pipes instead of real hardware.

#[cfg(test)]
mod tests {
    /// Known-good vector from live ECU captures: the CRC32 the Protocol 3
    /// envelope carries for a bare 'A' realtime-data command.
    #[test]
    fn crc32_matches_live_capture_vector() {
        assert_eq!(crc32fast::hash(b"A"), 0xD3D9_9E8B);
    }
}
