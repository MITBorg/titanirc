mod primitives;

pub use crate::primitives::*;

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

            impl Command {
                pub fn parse(input: &[u8]) -> Result<Option<Self>, nom::Err<nom::error::Error<&[u8]>>> {
                    let (rest, kind) = nom::bytes::complete::take_till(|c| c == b' ')(input)?;

                    $(const [<$name _BYTES>]: &[u8] = stringify!($name).as_bytes();)*

                    eprintln!("kind: {:?}", std::str::from_utf8(kind));

                    match kind {
                        $([<$name _BYTES>] => Ok(Some(Self::[<$name:camel>]([<$name:camel Command>]::parse(rest)?)))),*,
                        _ => Ok(None)
                    }
                }
            }

            $(
                #[derive(Debug)]
                pub struct [<$name:camel Command>] {
                    $($([<$param:snake>]: $param),*),*
                }

                impl [<$name:camel Command>] {
                    pub fn parse(rest: &[u8]) -> Result<Self, nom::Err<nom::error::Error<&[u8]>>> {
                        $(
                            $(
                                let (rest, _) = nom::bytes::complete::tag(" ")(rest)?;
                                let (rest, [<$param:snake>]) = $param::parse(rest)?;
                            )*
                        )*

                        Ok(Self {
                            $($([<$param:snake>]),*),*
                        })
                    }
                }
            )*
        }
    };
}

define_commands! {
    USER(Username, HostName, ServerName, RealName),
    NICK(Nick),

    VERSION,
    HELP,
    USERS,
    TIME,
    LIST,
    WHOIS(Nick),
    USERHOST(Nick),
    USERIP(Nick),
    JOIN(Channel),

    PRIVMSG(Receiver, Message),
}
