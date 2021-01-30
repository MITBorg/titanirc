#![deny(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

mod primitives;
mod replies;

pub use crate::primitives::*;
pub use crate::replies::{Reply, ServerMessage, Source};

use bytes::Bytes;
use nom::{
    bytes::complete::{tag, take_till},
    error::Error as NomError,
};
use nom_bytes::BytesWrapper;

fn parse_optional_source(input: BytesWrapper) -> nom::IResult<BytesWrapper, BytesWrapper> {
    let (rest, _) = tag(":".as_bytes())(input)?;
    let (rest, _) = take_till(|c| c == b' ')(rest)?;
    tag(" ".as_bytes())(rest)
}

macro_rules! define_commands {
    (
        $(
            $name:ident$((
                $($param:ty),*
            ))?
        ),* $(,)?
    ) => {
        paste::paste! {
            #[derive(Debug)]
            pub enum Command {
                $([<$name:camel>]([<$name:camel Command>])),*
            }

            $(const [<$name _BYTES>]: &[u8] = stringify!($name).as_bytes();)*

            impl Command {
                pub fn parse(input: Bytes) -> Result<Option<Self>, nom::Err<NomError<BytesWrapper>>> {
                    let input = BytesWrapper::from(input);

                    // skip the optional source at the start of the message
                    let input = if let Ok((input, _)) = parse_optional_source(input.clone()) {
                        input
                    } else {
                        input
                    };

                    let (params, command) = take_till(|c| c == b' ')(input)?;

                    match command.to_ascii_uppercase().as_ref() {
                        $([<$name _BYTES>] => Ok(Some(Self::[<$name:camel>]([<$name:camel Command>]::parse(params)?)))),*,
                        _ => Ok(None)
                    }
                }
            }

            impl std::fmt::Display for Command {
                fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    match self {
                        $(Self::[<$name:camel>](cmd) => cmd.fmt(fmt)),*
                    }
                }
            }

            $(
                #[derive(Debug)]
                pub struct [<$name:camel Command>] {
                    $($(pub [<$param:snake>]: $param),*),*
                }

                impl [<$name:camel Command>] {
                    #[allow(unused_variables)]
                    pub fn parse(rest: BytesWrapper) -> Result<Self, nom::Err<nom::error::Error<BytesWrapper>>> {
                        $(
                            $(
                                let (rest, _) = tag(" ".as_bytes())(rest)?;
                                let (rest, [<$param:snake>]) = $param::parse(rest)?;
                            )*
                        )*

                        Ok(Self {
                            $($([<$param:snake>]),*),*
                        })
                    }
                }

                impl std::fmt::Display for [<$name:camel Command>] {
                    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        fmt.write_str(stringify!($name))?;

                        $(
                            $(
                                fmt.write_str(" ")?;
                                self.[<$param:snake>].fmt(fmt)?;
                            )*
                        )*

                        Ok(())
                    }
                }

                impl Into<Command> for [<$name:camel Command>] {
                    fn into(self) -> Command {
                        Command::[<$name:camel>](self)
                    }
                }
            )*
        }
    };
}

define_commands! {
    USER(Username, HostName, ServerName, RealName),
    NICK(Nick),

    MOTD,
    VERSION,
    HELP,
    USERS,
    TIME,
    PONG(ServerName),
    PING(ServerName),
    LIST,
    MODE(Nick, Mode),
    WHOIS(Nick),
    USERHOST(Nick),
    USERIP(Nick),
    JOIN(Channel),

    PRIVMSG(Receiver, FreeText),
}

#[cfg(test)]
mod tests {
    use super::Command;
    use bytes::Bytes;

    #[test]
    fn parse_empty() {
        assert!(matches!(Command::parse(Bytes::from_static(b"")), Ok(None)));
    }

    #[test]
    fn parse_privmsg() {
        assert!(matches!(
            Command::parse(Bytes::from_static(b"PRIVMSG foo :baz")),
            Ok(Some(Command::Privmsg(super::PrivmsgCommand {
                receiver: super::Receiver::User(super::Nick(nick)),
                free_text: super::primitives::FreeText(msg),
            }))) if nick == "foo" && msg == "baz"
        ))
    }

    #[test]
    fn parse_privmsg_opt_source() {
        assert!(matches!(
            Command::parse(Bytes::from_static(b":some-fake-source!dude@nice PRIVMSG foo :baz")),
            Ok(Some(Command::Privmsg(super::PrivmsgCommand {
                receiver: super::Receiver::User(super::Nick(nick)),
                free_text: super::primitives::FreeText(msg),
            }))) if nick == "foo" && msg == "baz"
        ))
    }
}
