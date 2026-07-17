//! Blocking request/response session over a [`Transport`].
//!
//! Discipline mirrors pi-speeduino-logger: strictly one command in flight
//! (a second command while the ECU is still assembling the first desyncs
//! its parser), a response deadline after which the ECU's own parser is
//! assumed to have flushed, and an input flush after a CRC mismatch.

use std::time::{Duration, Instant};

use crate::command::{Args, Template};
use crate::transport::Transport;
use crate::{ProtoError, envelope, secondary};

/// How the ECU is wired up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// USB/primary serial: commands wrapped in the Protocol 3 CRC envelope.
    Primary,
    /// SER3 secondary serial: raw commands, no checksum, telemetry only.
    Secondary,
}

/// Protocol parameters lifted from the INI (never hardcoded).
#[derive(Debug, Clone)]
pub struct Config {
    pub mode: Mode,
    pub can_id: u8,
    /// `ochGetCommand` template, e.g. `r\$tsCanId\x30%2o%2c`.
    pub och_get_command: String,
    /// `ochBlockSize` — bytes in a realtime frame.
    pub och_block_size: u16,
    /// Deadline for a response before the command is considered unanswered.
    pub response_timeout: Duration,
}

impl Config {
    pub fn new(mode: Mode, och_get_command: &str, och_block_size: u16) -> Self {
        Config {
            mode,
            can_id: 0,
            och_get_command: och_get_command.to_string(),
            och_block_size,
            // POLL_RESPONSE_TIMEOUT_MS in the C implementation.
            response_timeout: Duration::from_millis(700),
        }
    }
}

pub struct Session<T: Transport> {
    transport: T,
    config: Config,
    och_get: Template,
}

impl<T: Transport> Session<T> {
    pub fn new(transport: T, config: Config) -> Result<Self, ProtoError> {
        let och_get = Template::parse(&config.och_get_command)?;
        Ok(Session {
            transport,
            config,
            och_get,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Poll one realtime telemetry frame (the raw och block).
    pub fn read_realtime(&mut self) -> Result<Vec<u8>, ProtoError> {
        let count = self.config.och_block_size;
        let cmd = self.och_get.build(&Args {
            can_id: self.config.can_id,
            offset: Some(0),
            count: Some(count),
            ..Default::default()
        })?;

        match self.config.mode {
            Mode::Secondary => {
                self.transport.write_all(&cmd)?;
                self.read_secondary(count as usize)
            }
            Mode::Primary => self.request(&cmd, &[envelope::RC_OK]),
        }
    }

    /// One enveloped request/response roundtrip (primary serial only).
    /// `ok` lists the return codes counted as success — burn commands ack
    /// with RC_BURN_OK (0x04), everything else with RC_OK. Returns the data
    /// after the return code.
    pub fn request(&mut self, payload: &[u8], ok: &[u8]) -> Result<Vec<u8>, ProtoError> {
        if self.config.mode != Mode::Primary {
            return Err(ProtoError::Unsupported(
                "enveloped commands need the primary serial (SER3 is telemetry-only)",
            ));
        }
        self.transport.write_all(&envelope::encode(payload))?;
        let full = self.read_enveloped(ok)?;
        Ok(full[1..].to_vec()) // strip return code
    }

    /// Send an ASCII command template (INI `queryCommand`/`versionInfo`,
    /// e.g. `"Q"`) and decode the response as text.
    pub fn query_string(&mut self, command_template: &str) -> Result<String, ProtoError> {
        let cmd = Template::parse(command_template)?.build(&Args {
            can_id: self.config.can_id,
            ..Default::default()
        })?;
        let data = self.request(&cmd, &[envelope::RC_OK])?;
        Ok(String::from_utf8_lossy(&data)
            .trim_end_matches('\0')
            .trim()
            .to_string())
    }

    /// Await one enveloped response; returns the payload (return code first).
    fn read_enveloped(&mut self, ok: &[u8]) -> Result<Vec<u8>, ProtoError> {
        let deadline = Instant::now() + self.config.response_timeout;
        let mut dec = envelope::Decoder::new();
        let mut chunk = [0u8; 256];
        loop {
            let n = self.read_some(&mut chunk, deadline)?;
            for &b in &chunk[..n] {
                match dec.push(b) {
                    Some(envelope::Event::Frame(payload)) => {
                        let rc = payload[0];
                        if !ok.contains(&rc) {
                            return Err(ProtoError::EcuError(rc));
                        }
                        return Ok(payload);
                    }
                    Some(envelope::Event::CrcMismatch) => {
                        // Let the line settle so the next response starts on
                        // a clean boundary, exactly like the C logger.
                        self.transport.flush_input()?;
                        return Err(ProtoError::CrcMismatch);
                    }
                    None => {}
                }
            }
        }
    }

    fn read_secondary(&mut self, expect: usize) -> Result<Vec<u8>, ProtoError> {
        let deadline = Instant::now() + self.config.response_timeout;
        let mut dec = secondary::Decoder::new(expect);
        let mut chunk = [0u8; 256];
        loop {
            let n = self.read_some(&mut chunk, deadline)?;
            for &b in &chunk[..n] {
                if let Some(payload) = dec.push(b) {
                    return Ok(payload);
                }
            }
        }
    }

    /// One transport read, failing once the deadline has passed with no data.
    fn read_some(&mut self, chunk: &mut [u8], deadline: Instant) -> Result<usize, ProtoError> {
        loop {
            if Instant::now() >= deadline {
                return Err(ProtoError::Timeout);
            }
            let n = self.transport.read(chunk)?;
            if n > 0 {
                return Ok(n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;

    fn config(mode: Mode) -> Config {
        let mut c = Config::new(mode, r"r\$tsCanId\x30%2o%2c", 4);
        c.response_timeout = Duration::from_millis(50);
        c
    }

    #[test]
    fn primary_realtime_roundtrip() {
        let mut mock = MockTransport::new();
        let mut body = vec![envelope::RC_OK];
        body.extend([10, 20, 30, 40]);
        mock.queue(envelope::encode(&body));

        let mut s = Session::new(mock, config(Mode::Primary)).unwrap();
        assert_eq!(s.read_realtime().unwrap(), vec![10, 20, 30, 40]);

        // The command on the wire: envelope('r' 0x00 0x30 0x0000 0x0004)
        let sent = &s.transport_mut().sent[0];
        assert_eq!(
            sent[..9],
            [0x00, 0x07, b'r', 0x00, 0x30, 0x00, 0x00, 0x04, 0x00]
        );
    }

    #[test]
    fn secondary_realtime_roundtrip() {
        let mut mock = MockTransport::new();
        mock.queue([b'r', 0x30, 1, 2, 3, 4]);
        let mut s = Session::new(mock, config(Mode::Secondary)).unwrap();
        assert_eq!(s.read_realtime().unwrap(), vec![1, 2, 3, 4]);
        // Raw command, no envelope.
        assert_eq!(
            s.transport_mut().sent[0],
            vec![b'r', 0x00, 0x30, 0x00, 0x00, 0x04, 0x00]
        );
    }

    #[test]
    fn crc_mismatch_flushes_and_errors() {
        let mut mock = MockTransport::new();
        let mut frame = envelope::encode(&[envelope::RC_OK, 1, 2, 3, 4]);
        frame[4] ^= 0xFF;
        mock.queue(frame);
        mock.queue([0xAA, 0xBB]); // junk that must be flushed

        let mut s = Session::new(mock, config(Mode::Primary)).unwrap();
        assert!(matches!(s.read_realtime(), Err(ProtoError::CrcMismatch)));
        assert!(s.transport_mut().responses.is_empty(), "input not flushed");
    }

    #[test]
    fn ecu_error_code_surfaces() {
        let mut mock = MockTransport::new();
        mock.queue(envelope::encode(&[0x83]));
        let mut s = Session::new(mock, config(Mode::Primary)).unwrap();
        assert!(matches!(s.read_realtime(), Err(ProtoError::EcuError(0x83))));
    }

    #[test]
    fn timeout_when_no_response() {
        let mock = MockTransport::new();
        let mut s = Session::new(mock, config(Mode::Primary)).unwrap();
        assert!(matches!(s.read_realtime(), Err(ProtoError::Timeout)));
    }
}
