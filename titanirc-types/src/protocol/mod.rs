pub mod commands;
pub mod primitives;
pub mod replies;

use std::fmt::Write;

use crate::RegisteredNick;

/// A message to be sent to the client over the wire.
#[derive(Debug, derive_more::From)]
pub enum ServerMessage<'a> {
    /// A `RPL_*`/`ERR_*` type from the IRC spec.
    Reply(replies::Reply<'a>),
    /// Normally a 'forwarded' message, ie. a `VERSION` for another client or
    /// a `PRIVMSG`.
    Command(replies::Source<'a>, commands::Command<'a>), // change Nick to whatever type nick!user@netmask is..
    /// A server ping to the client.
    Ping,
    /// A server pong to the client.
    Pong,
}

impl ServerMessage<'_> {
    /// Writes out this `ServerMessage` to `dst`, in the expected format for the wire.
    ///
    /// This function omits the CRLF from the end of the line.
    pub fn write(
        self,
        server_name: &str,
        client_username: &RegisteredNick,
        dst: &mut bytes::BytesMut,
    ) {
        match self {
            Self::Reply(reply) => {
                write!(dst, ":{} {} ", server_name, reply.code()).unwrap();
                match client_username.load() {
                    Some(v) => dst.extend_from_slice(&v[..]),
                    None => dst.write_char('*').unwrap(),
                }
                write!(dst, " {}", reply)
            }
            Self::Ping => write!(dst, "PING :{}", server_name),
            Self::Pong => write!(dst, "PONG :{}", server_name),
            Self::Command(source, command) => {
                let source = match &source {
                    replies::Source::User(nick) => std::str::from_utf8(nick).unwrap(),
                    replies::Source::Server => server_name,
                };
                write!(dst, ":{} {}", source, command)
            }
        }
        .unwrap()
    }
}
