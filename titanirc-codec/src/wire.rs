use bytes::{Buf, BytesMut};
use titanirc_types::Command;
use tokio_util::codec::Decoder as FrameDecoder;

pub const MAX_LENGTH: usize = 1024;

pub struct Decoder;

impl FrameDecoder for Decoder {
    type Item = Command;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let length = if let Some(len) = find_crlf(src) {
            len
        } else {
            if src.len() > MAX_LENGTH {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Frame of length {} is too large.", src.len()),
                ));
            }

            // tell Framed we need more bytes
            return Ok(None);
        };

        let bytes = src.copy_to_bytes(length + 1);

        match Command::parse(&bytes[..bytes.len() - 2]) {
            Ok(Some(msg)) => Ok(Some(msg)),
            Ok(None) => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unknown command",
            )),
            Err(err) => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        }
    }
}

fn find_crlf(src: &mut BytesMut) -> Option<usize> {
    let mut iter = src.iter().enumerate();

    while let Some((_, byte)) = iter.next() {
        if byte == &b'\r' {
            if let Some((pos, &b'\n')) = iter.next() {
                return Some(pos);
            }
        }
    }

    None
}
