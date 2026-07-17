//! Secondary-serial (SER3) response decoder.
//!
//! The secondary port has no envelope and no checksum: a telemetry poll
//! (`'r' canId 0x30 offset count`) is answered with `'r' 0x30` followed by
//! `count` raw och bytes. The decoder resyncs on the `'r' 0x30` echo, with
//! the same recovery quirk as the C implementation: a byte that isn't 0x30
//! but is `'r'` may itself start the next echo.

#[derive(Debug, PartialEq, Eq)]
enum State {
    CmdEcho,
    CanCmd,
    Payload,
}

pub struct Decoder {
    state: State,
    expect: usize,
    buf: Vec<u8>,
}

impl Decoder {
    /// `expect` — payload size of the pending request (its `count`).
    pub fn new(expect: usize) -> Self {
        Decoder {
            state: State::CmdEcho,
            expect,
            buf: Vec::new(),
        }
    }

    pub fn reset(&mut self, expect: usize) {
        self.state = State::CmdEcho;
        self.expect = expect;
        self.buf.clear();
    }

    /// Returns the raw och payload once complete.
    pub fn push(&mut self, byte: u8) -> Option<Vec<u8>> {
        match self.state {
            State::CmdEcho => {
                if byte == b'r' {
                    self.state = State::CanCmd;
                }
                None
            }
            State::CanCmd => {
                if byte == 0x30 {
                    self.buf.clear();
                    self.state = State::Payload;
                } else if byte != b'r' {
                    // 'r' would mean this byte starts the next echo: stay.
                    self.state = State::CmdEcho;
                }
                None
            }
            State::Payload => {
                self.buf.push(byte);
                if self.buf.len() == self.expect {
                    self.state = State::CmdEcho;
                    Some(std::mem::take(&mut self.buf))
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_echo_and_payload() {
        let mut dec = Decoder::new(3);
        let stream = [b'r', 0x30, 1, 2, 3];
        let got: Vec<_> = stream.iter().filter_map(|&b| dec.push(b)).collect();
        assert_eq!(got, vec![vec![1, 2, 3]]);
    }

    #[test]
    fn resyncs_past_garbage() {
        let mut dec = Decoder::new(2);
        // noise, then a stray 'r' followed by another 'r' 0x30 (the real echo)
        let stream = [0xAA, b'r', b'r', 0x30, 7, 8];
        let got: Vec<_> = stream.iter().filter_map(|&b| dec.push(b)).collect();
        assert_eq!(got, vec![vec![7, 8]]);
    }
}
