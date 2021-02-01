use super::primitives::*;

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
                $($param:ident$(<$($gen:tt),+>)?),*
            ))?
        ),* $(,)?
    ) => {
        paste::paste! {
            /// All the commands that can be ran by a client, also provides a `Display`
            /// implementation that serialises the command for sending over the wire,
            /// ie. for forwarding.
            #[derive(Debug)]
            pub enum Command<'a> {
                $([<$name:camel>]([<$name:camel Command>]<'a>)),*
            }

            $(const [<$name _BYTES>]: &[u8] = stringify!($name).as_bytes();)*

            impl Command<'_> {
                /// Parses a command from the wire, returning an `Err` if the command was unparsable or
                /// `Ok(None)` if the command was unrecognsied. The given `Bytes` should have the CRLF
                /// stripped.
                pub fn parse(input: Bytes) -> Result<Option<Self>, nom::Err<NomError<BytesWrapper>>> {
                    let mut input = BytesWrapper::from(input);

                    // skip the optional source at the start of the message
                    if let Ok((input_source_stripped, _)) = parse_optional_source(input.clone()) {
                        input = input_source_stripped;
                    }

                    let (params, command) = take_till(|c| c == b' ')(input)?;

                    match command.to_ascii_uppercase().as_ref() {
                        $([<$name _BYTES>] => Ok(Some(Self::[<$name:camel>]([<$name:camel Command>]::parse(params)?)))),*,
                        _ => Ok(None)
                    }
                }
            }

            /// Serialises the command for sending over the wire.
            impl std::fmt::Display for Command<'_> {
                fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    match self {
                        $(Self::[<$name:camel>](cmd) => cmd.fmt(fmt)),*
                    }
                }
            }

            $(
                #[derive(Debug)]
                pub struct [<$name:camel Command>]<'a> {
                    pub _phantom: std::marker::PhantomData<&'a ()>,
                    $($(pub [<$param:snake>]: $param$(<$($gen),+>)?),*),*
                }

                impl [<$name:camel Command>]<'_> {
                    /// Parses the command's arguments, with each parameter separated by a space.
                    #[allow(unused_variables)]
                    pub fn parse(rest: BytesWrapper) -> Result<Self, nom::Err<nom::error::Error<BytesWrapper>>> {
                        $(
                            $(
                                let (rest, _) = tag(" ".as_bytes())(rest)?;
                                let (rest, [<$param:snake>]) = $param::parse(rest)?;
                            )*
                        )*

                        Ok(Self {
                            _phantom: std::marker::PhantomData,
                            $($([<$param:snake>]),*),*
                        })
                    }
                }

                /// Serialises the command's arguments for sending over the wire, joining
                /// all the arguments separating them with a space.
                impl std::fmt::Display for [<$name:camel Command>]<'_> {
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

                impl<'a> Into<Command<'a>> for [<$name:camel Command>]<'a> {
                    fn into(self) -> Command<'a> {
                        Command::[<$name:camel>](self)
                    }
                }
            )*
        }
    };
}

define_commands! {
    USER(Username<'a>, HostName<'a>, ServerName<'a>, RealName<'a>),
    NICK(Nick<'a>),

    MOTD,
    VERSION,
    HELP,
    USERS,
    TIME,
    PONG(ServerName<'a>),
    PING(ServerName<'a>),
    LIST,
    MODE(Nick<'a>, Mode<'a>),
    WHOIS(Nick<'a>),
    USERHOST(Nick<'a>),
    USERIP(Nick<'a>),
    JOIN(Channel<'a>),

    PRIVMSG(Receiver<'a>, FreeText<'a>),
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
                _phantom: std::marker::PhantomData,
            }))) if &*nick == b"foo" && &*msg == b"baz"
        ))
    }

    #[test]
    fn parse_privmsg_opt_source() {
        assert!(matches!(
            Command::parse(Bytes::from_static(b":some-fake-source!dude@nice PRIVMSG foo :baz")),
            Ok(Some(Command::Privmsg(super::PrivmsgCommand {
                receiver: super::Receiver::User(super::Nick(nick)),
                free_text: super::primitives::FreeText(msg),
                _phantom: std::marker::PhantomData,
            }))) if &*nick == b"foo" && &*msg == b"baz"
        ))
    }
}
