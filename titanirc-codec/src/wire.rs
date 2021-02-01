use bytes::BytesMut;
use titanirc_types::{protocol::commands::Command, RegisteredNick};
use tokio_util::codec::Decoder as FrameDecoder;

pub const MAX_LENGTH: usize = 1024;

pub struct Decoder;

impl FrameDecoder for Decoder {
    /// Returns `'static` since we just return `BytesCow::Owned(bytes::Bytes)` and doesn't use the lifetime.
    type Item = Command<'static>;
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

        let bytes = {
            let mut b = src.split_to(length + 1);
            b.truncate(b.len() - 2); // remove the crlf at the end of the buffer
            b.freeze()
        };

        eprintln!("{:?}", std::str::from_utf8(&bytes[..]));

        match Command::parse(bytes) {
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

pub struct Encoder {
    server_name: &'static str,
    nick: RegisteredNick,
}

impl Encoder {
    #[must_use]
    pub fn new(server_name: &'static str, nick: RegisteredNick) -> Self {
        Self { server_name, nick }
    }
}

impl tokio_util::codec::Encoder<titanirc_types::protocol::ServerMessage<'_>> for Encoder {
    type Error = std::io::Error;

    fn encode(
        &mut self,
        item: titanirc_types::protocol::ServerMessage,
        dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        item.write(&self.server_name, &self.nick, dst);
        dst.extend_from_slice(b"\r\n");
        Ok(())
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
