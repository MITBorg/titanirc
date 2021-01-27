#![deny(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

mod primitives;
mod replies;

pub use crate::primitives::*;
pub use crate::replies::{Reply, ServerMessage};

use nom::{
    bytes::complete::{tag, take_till},
    error::Error as NomError,
};

fn parse_optional_source(input: &[u8]) -> nom::IResult<&[u8], &[u8]> {
    let (rest, _) = tag(":")(input)?;
    let (rest, _) = take_till(|c| c == b' ')(rest)?;
    tag(" ")(rest)
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
                pub fn parse(input: &[u8]) -> Result<Option<Self>, nom::Err<NomError<&[u8]>>> {
                    // skip the optional source at the start of the message
                    let rest = if let Ok((rest, _)) = parse_optional_source(input) {
                        rest
                    } else {
                        input
                    };

                    let (rest, kind) = take_till(|c| c == b' ')(rest)?;

                    // fix this shit
                    match std::str::from_utf8(kind).unwrap().to_uppercase().as_bytes() {
                        $([<$name _BYTES>] => Ok(Some(Self::[<$name:camel>]([<$name:camel Command>]::parse(rest)?)))),*,
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
                    pub fn parse(rest: &[u8]) -> Result<Self, nom::Err<nom::error::Error<&[u8]>>> {
                        $(
                            $(
                                let (rest, _) = tag(" ")(rest)?;
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

    #[test]
    fn parse_empty() {
        assert!(matches!(Command::parse(b""), Ok(None)));
    }

    #[test]
    fn parse_privmsg() {
        assert!(matches!(
            Command::parse(b"PRIVMSG foo :baz"),
            Ok(Some(Command::Privmsg(super::PrivmsgCommand {
                receiver: super::Receiver::User(super::Nick(nick)),
                free_text: super::primitives::FreeText(msg),
            }))) if nick == "foo" && msg == "baz"
        ))
    }

    #[test]
    fn parse_privmsg_opt_source() {
        eprintln!(
            "{:?}",
            Command::parse(b":some-fake-source!dude@nice PRIVMSG foo :baz")
        );

        assert!(matches!(
            Command::parse(b":some-fake-source!dude@nice PRIVMSG foo :baz"),
            Ok(Some(Command::Privmsg(super::PrivmsgCommand {
                receiver: super::Receiver::User(super::Nick(nick)),
                free_text: super::primitives::FreeText(msg),
            }))) if nick == "foo" && msg == "baz"
        ))
    }
}
