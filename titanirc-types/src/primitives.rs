use nom::IResult;

pub trait PrimitiveParser {
    fn parse(bytes: &[u8]) -> IResult<&[u8], Self>
    where
        Self: Sized;
}

macro_rules! standard_string_parser {
    ($name:ty) => {
        impl PrimitiveParser for $name {
            fn parse(bytes: &[u8]) -> IResult<&[u8], Self> {
                let (rest, val) = nom::combinator::map_res(
                    nom::bytes::complete::take_till(|c| c == b' '),
                    std::str::from_utf8,
                )(bytes)?;

                // TODO: don't clone
                Ok((rest, Self(val.to_string())))
            }
        }
    };
}

#[derive(Debug)]
pub struct Username(String);

standard_string_parser!(Username);

#[derive(Debug)]
pub struct HostName(String);

standard_string_parser!(HostName);

#[derive(Debug)]
pub struct ServerName(String);

standard_string_parser!(ServerName);

#[derive(Debug)]
pub struct RealName(String);

standard_string_parser!(RealName);

#[derive(Debug)]
pub struct Nick(String);

standard_string_parser!(Nick);

#[derive(Debug)]
pub struct Channel(String);

standard_string_parser!(Channel);

#[derive(Debug)]
pub enum Receiver {
    User(Nick),
    Channel(Channel),
}

impl PrimitiveParser for Receiver {
    fn parse(bytes: &[u8]) -> IResult<&[u8], Self> {
        if let Ok((rest, _)) =
            nom::bytes::complete::tag::<_, _, nom::error::Error<&[u8]>>("#")(bytes)
        {
            let (rest, channel) = Channel::parse(rest)?;
            Ok((rest, Self::Channel(channel)))
        } else {
            let (rest, nick) = Nick::parse(bytes)?;
            Ok((rest, Self::User(nick)))
        }
    }
}

#[derive(Debug)]
pub struct Message(String);

impl PrimitiveParser for Message {
    fn parse(bytes: &[u8]) -> IResult<&[u8], Self> {
        // TODO: don't clone, don't panic
        let val = std::str::from_utf8(bytes).expect("utf-8").to_string();

        Ok((b"", Self(val)))
    }
}
