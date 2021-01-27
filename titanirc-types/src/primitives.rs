use derive_more::{Deref, From};
use nom::{
    bytes::complete::{tag, take_till},
    combinator::{iterator, map_res},
    sequence::terminated,
    IResult,
};

pub trait ValidatingParser {
    fn validate(bytes: &[u8]) -> bool;
}

pub trait PrimitiveParser {
    fn parse(bytes: &[u8]) -> IResult<&[u8], Self>
    where
        Self: Sized;
}

macro_rules! noop_validator {
    ($name:ty) => {
        impl ValidatingParser for $name {
            fn validate(_: &[u8]) -> bool {
                true
            }
        }
    };
}

macro_rules! free_text_primitive {
    ($name:ty) => {
        impl PrimitiveParser for $name {
            fn parse(bytes: &[u8]) -> IResult<&[u8], Self> {
                let (rest, _) = tag(b":")(bytes)?;
                Ok((&[], Self(std::str::from_utf8(rest).unwrap().to_string())))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

macro_rules! space_terminated_primitive {
    ($name:ty) => {
        impl PrimitiveParser for $name {
            fn parse(bytes: &[u8]) -> IResult<&[u8], Self> {
                let (rest, val) = map_res(take_till(|c| c == b' '), std::str::from_utf8)(bytes)?;

                if !<Self as ValidatingParser>::validate(val.as_bytes()) {
                    return Err(nom::Err::Failure(nom::error::Error::new(
                        bytes,
                        nom::error::ErrorKind::Verify,
                    )));
                }

                // TODO: don't clone
                Ok((rest, Self(val.to_string())))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

macro_rules! space_delimited_display {
    ($name:ty) => {
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                // once the first iteration is complete, we'll start adding spaces before
                // each nickname.
                let mut space = false;

                for value in &self.0 {
                    if space {
                        f.write_str(" ")?;
                    } else {
                        space = true;
                    }

                    value.fmt(f)?;
                }

                Ok(())
            }
        }
    };
}

pub struct Letter;

impl ValidatingParser for Letter {
    fn validate(bytes: &[u8]) -> bool {
        bytes
            .iter()
            .all(|c| (b'a'..=b'z').contains(c) || (b'A'..=b'Z').contains(c))
    }
}

pub struct Number;

impl ValidatingParser for Number {
    fn validate(bytes: &[u8]) -> bool {
        bytes.iter().all(|c| (b'0'..=b'9').contains(c))
    }
}

pub struct Special;

impl ValidatingParser for Special {
    fn validate(bytes: &[u8]) -> bool {
        const ALLOWED: &[u8] = &[b'-', b'[', b']', b'\\', b'`', b'^', b'{', b'}'];

        bytes.iter().all(|c| ALLOWED.contains(c))
    }
}

#[derive(Debug, Deref, From)]
pub struct Username(pub String);
space_terminated_primitive!(Username);
noop_validator!(Username);

#[derive(Debug, Deref, From)]
pub struct Mode(pub String);
space_terminated_primitive!(Mode);
noop_validator!(Mode);

#[derive(Debug, Deref, From)]
pub struct HostName(pub String);
space_terminated_primitive!(HostName);
noop_validator!(HostName);

#[derive(Debug, Deref, From)]
pub struct ServerName(pub String);
space_terminated_primitive!(ServerName);
noop_validator!(ServerName);

#[derive(Debug, Deref, From)]
pub struct RealName(pub String);
space_terminated_primitive!(RealName);
noop_validator!(RealName);

#[derive(Debug, Deref, From)]
pub struct Nick(pub String);
space_terminated_primitive!(Nick);

// TODO: i feel like this would be better suited as a nom chomper to stop
// iterating over the string twice unnecessarily
impl ValidatingParser for Nick {
    fn validate(bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }

        if !Letter::validate(&[bytes[0]]) {
            return false;
        }

        bytes[1..]
            .iter()
            .all(|c| Letter::validate(&[*c]) || Number::validate(&[*c]) || Special::validate(&[*c]))
    }
}

#[derive(Debug, Deref, From)]
pub struct Channel(pub String);
space_terminated_primitive!(Channel);
noop_validator!(Channel);

#[derive(Debug, Deref, From)]
pub struct FreeText(pub String);
free_text_primitive!(FreeText);
noop_validator!(FreeText);

#[derive(Debug, Deref, From)]
pub struct Nicks(pub Vec<Nick>);
space_delimited_display!(Nicks);

impl PrimitiveParser for Nicks {
    fn parse(bytes: &[u8]) -> IResult<&[u8], Self> {
        let mut it = iterator(bytes, terminated(take_till(|c| c == b' '), tag(b" ")));

        let parsed = it
            .map(|v| Nick(std::str::from_utf8(v).unwrap().to_string()))
            .collect();

        it.finish()
            .map(move |(remaining, _)| (remaining, Self(parsed)))
    }
}

#[derive(Debug)]
pub struct RightsPrefixedNick(pub Rights, pub Nick);

impl std::fmt::Display for RightsPrefixedNick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)?;
        self.1.fmt(f)
    }
}

#[derive(Debug, Deref, From)]
pub struct RightsPrefixedNicks(pub Vec<RightsPrefixedNick>);
space_delimited_display!(RightsPrefixedNicks);

#[derive(Debug)]
pub struct RightsPrefixedChannel(pub Rights, pub Nick);

impl std::fmt::Display for RightsPrefixedChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)?;
        self.1.fmt(f)
    }
}

#[derive(Debug, Deref, From)]
pub struct RightsPrefixedChannels(pub Vec<RightsPrefixedChannel>);
space_delimited_display!(RightsPrefixedChannels);

#[derive(Debug)]
pub enum Rights {
    Op,
    Voice,
}

impl std::fmt::Display for Rights {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Op => "@",
            Self::Voice => "+",
        })
    }
}

#[derive(Debug, From)]
pub enum Receiver {
    User(Nick),
    Channel(Channel),
}

impl std::ops::Deref for Receiver {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::User(nick) => &*nick,
            Self::Channel(channel) => &*channel,
        }
    }
}

impl PrimitiveParser for Receiver {
    fn parse(bytes: &[u8]) -> IResult<&[u8], Self> {
        if let Ok((_, _)) = nom::bytes::complete::tag::<_, _, nom::error::Error<&[u8]>>("#")(bytes)
        {
            let (rest, channel) = Channel::parse(bytes)?;
            Ok((rest, Self::Channel(channel)))
        } else {
            let (rest, nick) = Nick::parse(bytes)?;
            Ok((rest, Self::User(nick)))
        }
    }
}
