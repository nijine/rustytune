//! TunerStudio Protocol 3 framing ("msEnvelope_1.0").
//!
//! TX: `length(BE16) + payload + CRC32(BE32 over payload)`
//! RX: `length(BE16) + returnCode + data + CRC32(BE32 over returnCode+data)`
//!
//! The decoder is a byte-at-a-time state machine mirroring the proven logic
//! in pi-speeduino-logger: an implausible length header shifts one byte and
//! retries (resync), and a CRC mismatch is reported so the caller can flush
//! the input and let the ECU's parser settle.

/// CRC32 (IEEE reflected, same as zlib). Known vector from live ECU
/// captures: `crc32(b"A") == 0xD3D99E8B`.
pub fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

/// Wrap a command payload in the envelope.
pub fn encode(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 6);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&crc32(payload).to_be_bytes());
    out
}

/// Largest plausible RX payload: return code + a full page read chunk.
/// Headers above this are treated as noise and resynced past.
const MAX_PAYLOAD: u16 = 2048;

/// ECU return code for success (first payload byte of an RX frame).
pub const RC_OK: u8 = 0x00;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A complete frame with a valid CRC: `payload[0]` is the return code.
    Frame(Vec<u8>),
    /// A complete-length frame whose CRC didn't match; caller should flush
    /// the transport input before the next command.
    CrcMismatch,
}

enum State {
    Header,
    Payload,
}

pub struct Decoder {
    state: State,
    header: [u8; 2],
    header_len: usize,
    payload_len: usize,
    buf: Vec<u8>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Decoder {
            state: State::Header,
            header: [0; 2],
            header_len: 0,
            payload_len: 0,
            buf: Vec::new(),
        }
    }

    /// Drop any partial frame (call after a timeout or input flush).
    pub fn reset(&mut self) {
        self.state = State::Header;
        self.header_len = 0;
        self.buf.clear();
    }

    pub fn push(&mut self, byte: u8) -> Option<Event> {
        match self.state {
            State::Header => {
                self.header[self.header_len] = byte;
                self.header_len += 1;
                if self.header_len == 2 {
                    let len = u16::from_be_bytes(self.header);
                    if len > 0 && len <= MAX_PAYLOAD {
                        self.payload_len = len as usize;
                        self.buf.clear();
                        self.state = State::Payload;
                    } else {
                        // Implausible length: shift one byte, keep hunting.
                        self.header[0] = self.header[1];
                        self.header_len = 1;
                    }
                }
                None
            }
            State::Payload => {
                self.buf.push(byte);
                if self.buf.len() < self.payload_len + 4 {
                    return None;
                }
                self.state = State::Header;
                self.header_len = 0;

                let (payload, trailer) = self.buf.split_at(self.payload_len);
                let wire = u32::from_be_bytes(trailer.try_into().unwrap());
                if crc32(payload) == wire {
                    Some(Event::Frame(payload.to_vec()))
                } else {
                    Some(Event::CrcMismatch)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_live_capture_vector() {
        assert_eq!(crc32(b"A"), 0xD3D9_9E8B);
    }

    #[test]
    fn encode_layout() {
        let f = encode(b"A");
        assert_eq!(f, [0x00, 0x01, 0x41, 0xD3, 0xD9, 0x9E, 0x8B]);
    }

    fn feed(dec: &mut Decoder, bytes: &[u8]) -> Vec<Event> {
        bytes.iter().filter_map(|&b| dec.push(b)).collect()
    }

    #[test]
    fn roundtrip() {
        let mut dec = Decoder::new();
        let frame = encode(&[RC_OK, 1, 2, 3]);
        let events = feed(&mut dec, &frame);
        assert_eq!(events, vec![Event::Frame(vec![RC_OK, 1, 2, 3])]);
    }

    #[test]
    fn resyncs_past_noise_before_header() {
        let mut dec = Decoder::new();
        let mut stream = vec![0xFF, 0xFE]; // implausible length 0xFFFE
        stream.extend(encode(&[RC_OK, 9]));
        let events = feed(&mut dec, &stream);
        assert_eq!(events, vec![Event::Frame(vec![RC_OK, 9])]);
    }

    #[test]
    fn crc_mismatch_reported() {
        let mut dec = Decoder::new();
        let mut frame = encode(&[RC_OK, 1, 2, 3]);
        let mid = frame.len() / 2;
        frame[mid] ^= 0xFF;
        let events = feed(&mut dec, &frame);
        assert_eq!(events, vec![Event::CrcMismatch]);
    }
}
