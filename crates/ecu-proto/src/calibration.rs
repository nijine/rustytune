//! Speeduino reference-table commands. Unlike tune pages, calibration
//! offsets/counts are big-endian and the final chunk saves directly to EEPROM.
use crate::{
    ProtoError, Session,
    command::{Args, Template},
    envelope,
    transport::Transport,
};

pub fn crc<T: Transport>(
    session: &mut Session<T>,
    template: &str,
    id: u16,
) -> Result<u32, ProtoError> {
    let id = [u8::try_from(id)
        .map_err(|_| ProtoError::Unsupported("calibration identifier exceeds one byte"))?];
    let cmd = Template::parse(template)?.build(&Args {
        can_id: session.config().can_id,
        page_id: Some(&id),
        offset: Some(0),
        count: Some(0),
        ..Default::default()
    })?;
    let data = session.request(&cmd, &[envelope::RC_OK])?;
    if data.len() != 4 {
        return Err(ProtoError::ShortResponse {
            expected: 4,
            got: data.len(),
        });
    }
    Ok(u32::from_be_bytes(data.try_into().unwrap()))
}

pub fn write<T: Transport>(
    session: &mut Session<T>,
    template: &str,
    id: u16,
    blocking_factor: usize,
    data: &[u8],
) -> Result<(), ProtoError> {
    if data.len() != 1024
        || blocking_factor == 0
        || blocking_factor > 256
        || !blocking_factor.is_multiple_of(32)
    {
        return Err(ProtoError::Unsupported(
            "unsupported AFR calibration table layout",
        ));
    }
    let id = [u8::try_from(id)
        .map_err(|_| ProtoError::Unsupported("calibration identifier exceeds one byte"))?];
    // Prebuild all chunks so template errors cannot leave a partial calibration.
    let template = Template::parse(template)?;
    let mut commands = Vec::new();
    for (i, chunk) in data.chunks(blocking_factor).enumerate() {
        commands.push(template.build(&Args {
            can_id: session.config().can_id,
            page_id: Some(&id),
            offset: Some(((i * blocking_factor) as u16).swap_bytes()),
            count: Some((chunk.len() as u16).swap_bytes()),
            value: Some(chunk),
        })?);
    }
    for cmd in commands {
        session.request(&cmd, &[envelope::RC_OK])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Mode, transport::MockTransport};
    #[test]
    fn calibration_uses_big_endian_chunks_and_stops_on_error() {
        let mut mock = MockTransport::new();
        mock.queue(envelope::encode(&[envelope::RC_OK]));
        mock.queue(envelope::encode(&[0x84]));
        let mut s = Session::new(mock, Config::new(Mode::Primary, "A", 1)).unwrap();
        assert!(write(&mut s, r"t\$tsCanId%2i%2o%2c%v", 2, 256, &[147; 1024]).is_err());
        let sent = &s.transport_mut().sent;
        assert_eq!(sent.len(), 2);
        assert_eq!(&sent[0][2..9], &[b't', 0, 2, 0, 0, 1, 0]);
        assert_eq!(&sent[1][2..9], &[b't', 0, 2, 1, 0, 1, 0]);
    }
}
