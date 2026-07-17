//! Tune page operations: read, chunk write, CRC check, burn — all built
//! from the INI [Constants] header templates (`pageReadCommand`,
//! `pageChunkWrite`, `crc32CheckCommand`, `burnCommand`, `pageIdentifier`).
//!
//! Byte layouts verified against speeduino/speeduino comms.cpp (dispatch
//! identical 202402.3 → 202501.7): payloads are `cmd + canId + page +
//! offset(LE16) + count(LE16) [+ values]`, `d` answers RC_OK + CRC32
//! big-endian, and burn acks with SERIAL_RC_BURN_OK (0x04) — not RC_OK.

use crate::command::{Args, Template};
use crate::session::Session;
use crate::transport::Transport;
use crate::{ProtoError, envelope};

/// SERIAL_RC_BURN_OK in the firmware: the success code for `b`/`B`.
pub const RC_BURN_OK: u8 = 0x04;

/// Raw template strings for the page command set, as parsed from the INI
/// (one entry per page in each list).
#[derive(Debug, Clone)]
pub struct PagesConfig<'a> {
    pub identifiers: &'a [String],
    pub page_read: &'a [String],
    pub chunk_write: &'a [String],
    pub crc_check: &'a [String],
    pub burn: &'a [String],
    pub sizes: &'a [u32],
    /// Largest chunk the ECU serial buffer can answer (INI blockingFactor).
    pub blocking_factor: u16,
    pub can_id: u8,
}

struct PageDef {
    /// Pre-built `%2i` bytes (from this page's `pageIdentifier`).
    identifier: Vec<u8>,
    read: Template,
    chunk_write: Template,
    crc: Template,
    burn: Template,
    size: u32,
}

/// Parsed page command set; issues requests through a primary-mode
/// [`Session`].
pub struct PageCommands {
    pages: Vec<PageDef>,
    blocking_factor: u16,
    can_id: u8,
}

impl PageCommands {
    pub fn new(cfg: &PagesConfig) -> Result<Self, ProtoError> {
        fn tpl(list: &[String], i: usize, what: &str) -> Result<Template, ProtoError> {
            let text = list.get(i).ok_or_else(|| {
                ProtoError::Template(format!("missing {what} for page {}", i + 1))
            })?;
            Template::parse(text)
        }

        let mut pages = Vec::with_capacity(cfg.sizes.len());
        for (i, &size) in cfg.sizes.iter().enumerate() {
            let identifier = tpl(cfg.identifiers, i, "pageIdentifier")?.build(&Args {
                can_id: cfg.can_id,
                ..Default::default()
            })?;
            pages.push(PageDef {
                identifier,
                read: tpl(cfg.page_read, i, "pageReadCommand")?,
                chunk_write: tpl(cfg.chunk_write, i, "pageChunkWrite")?,
                crc: tpl(cfg.crc_check, i, "crc32CheckCommand")?,
                burn: tpl(cfg.burn, i, "burnCommand")?,
                size,
            });
        }
        Ok(PageCommands {
            pages,
            blocking_factor: cfg.blocking_factor.max(1),
            can_id: cfg.can_id,
        })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page_size(&self, page_idx: usize) -> Option<u32> {
        self.pages.get(page_idx).map(|p| p.size)
    }

    fn page(&self, page_idx: usize) -> Result<&PageDef, ProtoError> {
        self.pages.get(page_idx).ok_or_else(|| {
            ProtoError::Template(format!(
                "page index {page_idx} out of range ({} pages)",
                self.pages.len()
            ))
        })
    }

    /// Download a whole page, chunked by blockingFactor.
    pub fn read_page<T: Transport>(
        &self,
        session: &mut Session<T>,
        page_idx: usize,
    ) -> Result<Vec<u8>, ProtoError> {
        let page = self.page(page_idx)?;
        let mut out = Vec::with_capacity(page.size as usize);
        let mut offset: u32 = 0;
        while offset < page.size {
            let count = (page.size - offset).min(self.blocking_factor as u32) as u16;
            let cmd = page.read.build(&Args {
                can_id: self.can_id,
                page_id: Some(&page.identifier),
                offset: Some(offset as u16),
                count: Some(count),
                ..Default::default()
            })?;
            let data = session.request(&cmd, &[envelope::RC_OK])?;
            if data.len() != count as usize {
                return Err(ProtoError::ShortResponse {
                    expected: count as usize,
                    got: data.len(),
                });
            }
            out.extend_from_slice(&data);
            offset += count as u32;
        }
        Ok(out)
    }

    /// Write `values` into the ECU's working copy of the page (`M`).
    pub fn write_chunk<T: Transport>(
        &self,
        session: &mut Session<T>,
        page_idx: usize,
        offset: u16,
        values: &[u8],
    ) -> Result<(), ProtoError> {
        let page = self.page(page_idx)?;
        let cmd = page.chunk_write.build(&Args {
            can_id: self.can_id,
            page_id: Some(&page.identifier),
            offset: Some(offset),
            count: Some(values.len() as u16),
            value: Some(values),
        })?;
        session.request(&cmd, &[envelope::RC_OK])?;
        Ok(())
    }

    /// CRC32 of the ECU's working copy of the page (`d`), for verifying
    /// writes without re-downloading.
    pub fn page_crc<T: Transport>(
        &self,
        session: &mut Session<T>,
        page_idx: usize,
    ) -> Result<u32, ProtoError> {
        let page = self.page(page_idx)?;
        let cmd = page.crc.build(&Args {
            can_id: self.can_id,
            page_id: Some(&page.identifier),
            ..Default::default()
        })?;
        let data = session.request(&cmd, &[envelope::RC_OK])?;
        if data.len() != 4 {
            return Err(ProtoError::ShortResponse {
                expected: 4,
                got: data.len(),
            });
        }
        // Firmware reverse_bytes()es the value: big-endian on the wire.
        Ok(u32::from_be_bytes(data.try_into().unwrap()))
    }

    /// Commit the page to EEPROM (`b`/`B`). The firmware acks with
    /// RC_BURN_OK and may defer the physical write (burnPending).
    pub fn burn<T: Transport>(
        &self,
        session: &mut Session<T>,
        page_idx: usize,
    ) -> Result<(), ProtoError> {
        let page = self.page(page_idx)?;
        let cmd = page.burn.build(&Args {
            can_id: self.can_id,
            page_id: Some(&page.identifier),
            ..Default::default()
        })?;
        session.request(&cmd, &[envelope::RC_OK, RC_BURN_OK])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Config, Mode};
    use crate::transport::MockTransport;

    fn commands() -> PageCommands {
        let identifiers: Vec<String> = (1..=2).map(|n| format!(r"\$tsCanId\x{n:02X}")).collect();
        let repeat = |s: &str| vec![s.to_string(); 2];
        PageCommands::new(&PagesConfig {
            identifiers: &identifiers,
            page_read: &repeat("p%2i%2o%2c"),
            chunk_write: &repeat("M%2i%2o%2c%v"),
            crc_check: &repeat("d%2i"),
            burn: &repeat("b%2i"),
            sizes: &[6, 300],
            blocking_factor: 251,
            can_id: 0,
        })
        .unwrap()
    }

    fn session(mock: MockTransport) -> Session<MockTransport> {
        let mut config = Config::new(Mode::Primary, r"r\$tsCanId\x30%2o%2c", 4);
        config.response_timeout = std::time::Duration::from_millis(50);
        Session::new(mock, config).unwrap()
    }

    #[test]
    fn read_page_chunks_by_blocking_factor() {
        let content: Vec<u8> = (0..300u16).map(|i| (i % 251) as u8).collect();
        let mut mock = MockTransport::new();
        let mut first = vec![envelope::RC_OK];
        first.extend_from_slice(&content[..251]);
        mock.queue(envelope::encode(&first));
        let mut second = vec![envelope::RC_OK];
        second.extend_from_slice(&content[251..]);
        mock.queue(envelope::encode(&second));

        let mut s = session(mock);
        let page = commands().read_page(&mut s, 1).unwrap();
        assert_eq!(page, content);

        // Two requests: offsets 0 and 251, counts 251 and 49; page id 0x00 0x02.
        let sent = &s.transport_mut().sent;
        assert_eq!(sent.len(), 2);
        assert_eq!(
            sent[0][2..9],
            [b'p', 0x00, 0x02, 0x00, 0x00, 251, 0x00],
            "first chunk"
        );
        assert_eq!(
            sent[1][2..9],
            [b'p', 0x00, 0x02, 251, 0x00, 49, 0x00],
            "second chunk"
        );
    }

    #[test]
    fn burn_accepts_burn_ok_code() {
        let mut mock = MockTransport::new();
        mock.queue(envelope::encode(&[RC_BURN_OK]));
        let mut s = session(mock);
        commands().burn(&mut s, 0).unwrap();
        assert_eq!(s.transport_mut().sent[0][2..5], [b'b', 0x00, 0x01]);
    }

    #[test]
    fn crc_is_big_endian() {
        let mut mock = MockTransport::new();
        mock.queue(envelope::encode(&[envelope::RC_OK, 0xD3, 0xD9, 0x9E, 0x8B]));
        let mut s = session(mock);
        assert_eq!(commands().page_crc(&mut s, 0).unwrap(), 0xD3D99E8B);
    }

    #[test]
    fn write_chunk_builds_m_command() {
        let mut mock = MockTransport::new();
        mock.queue(envelope::encode(&[envelope::RC_OK]));
        let mut s = session(mock);
        commands().write_chunk(&mut s, 0, 4, &[0xAB, 0xCD]).unwrap();
        assert_eq!(
            s.transport_mut().sent[0][2..11],
            [b'M', 0x00, 0x01, 0x04, 0x00, 0x02, 0x00, 0xAB, 0xCD]
        );
    }

    #[test]
    fn secondary_mode_rejects_requests() {
        let mut config = Config::new(Mode::Secondary, r"r\$tsCanId\x30%2o%2c", 4);
        config.response_timeout = std::time::Duration::from_millis(50);
        let mut s = Session::new(MockTransport::new(), config).unwrap();
        assert!(matches!(
            commands().read_page(&mut s, 0),
            Err(ProtoError::Unsupported(_))
        ));
    }
}
