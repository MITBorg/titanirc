use bytes::Bytes;
use derive_more::{Deref, From};
use nom::{
    bytes::complete::{tag, take_till},
    combinator::iterator,
    sequence::terminated,
    IResult,
};
use nom_bytes::BytesWrapper;

pub trait ValidatingParser {
    fn validate(bytes: &[u8]) -> bool;
}

pub trait PrimitiveParser {
    fn parse(bytes: BytesWrapper) -> IResult<BytesWrapper, Self>
    where
        Self: Sized;
}

/// A `Cow`-like implementation where `Owned` is a `bytes::Bytes` and `Borrowed`
/// is `&[u8]`.
#[derive(Debug, From)]
pub enum BytesCow<'a> {
    Owned(Bytes),
    Borrowed(&'a [u8]),
}

impl From<BytesWrapper> for BytesCow<'_> {
    fn from(other: BytesWrapper) -> Self {
        Self::Owned(other.into())
    }
}

impl Clone for BytesCow<'_> {
    fn clone(&self) -> Self {
        Self::Owned(match self {
            Self::Owned(b) => b.clone(),
            Self::Borrowed(b) => Bytes::copy_from_slice(b),
        })
    }
}

impl std::ops::Deref for BytesCow<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(b) => &*b,
            Self::Borrowed(b) => *b,
        }
    }
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
            fn parse(bytes: BytesWrapper) -> IResult<BytesWrapper, Self> {
                let (rest, _) = tag(":".as_bytes())(bytes)?;
                Ok((Bytes::new().into(), Self(rest.into())))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match std::str::from_utf8(&self.0[..]) {
                    Ok(v) => f.write_str(v),
                    Err(_e) => {
                        // todo: report this better
                        eprintln!("Invalid utf-8 in {}", stringify!($name));
                        Err(std::fmt::Error)
                    }
                }
            }
        }
    };
}

macro_rules! space_terminated_primitive {
    ($name:ty) => {
        impl PrimitiveParser for $name {
            fn parse(bytes: BytesWrapper) -> IResult<BytesWrapper, Self> {
                let (rest, val) = take_till(|c| c == b' ')(bytes.clone())?;

                if !<Self as ValidatingParser>::validate(&val[..]) {
                    return Err(nom::Err::Failure(nom::error::Error::new(
                        bytes,
                        nom::error::ErrorKind::Verify,
                    )));
                }

                Ok((rest, Self(val.into())))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match std::str::from_utf8(&self.0[..]) {
                    Ok(v) => f.write_str(v),
                    Err(_e) => {
                        // todo: report this better
                        eprintln!("Invalid utf-8 in {}", stringify!($name));
                        Err(std::fmt::Error)
                    }
                }
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

#[derive(Debug, Deref, Clone, From)]
pub struct Username<'a>(pub BytesCow<'a>);
space_terminated_primitive!(Username<'_>);
noop_validator!(Username<'_>);

#[derive(Debug, Deref, Clone, From)]
pub struct Mode<'a>(pub BytesCow<'a>);
space_terminated_primitive!(Mode<'_>);
noop_validator!(Mode<'_>);

#[derive(Debug, Deref, Clone, From)]
pub struct HostName<'a>(pub BytesCow<'a>);
space_terminated_primitive!(HostName<'_>);
noop_validator!(HostName<'_>);

#[derive(Debug, Deref, Clone, From)]
pub struct ServerName<'a>(pub BytesCow<'a>);
space_terminated_primitive!(ServerName<'_>);
noop_validator!(ServerName<'_>);

#[derive(Debug, Deref, Clone, From)]
pub struct RealName<'a>(pub BytesCow<'a>);
space_terminated_primitive!(RealName<'_>);
noop_validator!(RealName<'_>);

#[derive(Debug, Deref, Clone, From)]
pub struct Nick<'a>(pub BytesCow<'a>);
space_terminated_primitive!(Nick<'_>);

// TODO: i feel like this would be better suited as a nom chomper to stop
// iterating over the string twice unnecessarily
impl ValidatingParser for Nick<'_> {
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

#[derive(Debug, Deref, Clone, From)]
pub struct Channel<'a>(pub BytesCow<'a>);
space_terminated_primitive!(Channel<'_>);
noop_validator!(Channel<'_>);

#[derive(Debug, Deref, Clone, From)]
pub struct FreeText<'a>(pub BytesCow<'a>);
free_text_primitive!(FreeText<'_>);
noop_validator!(FreeText<'_>);

#[derive(Debug, Deref, Clone, From)]
pub struct Nicks<'a>(pub Vec<Nick<'a>>);
space_delimited_display!(Nicks<'_>);

impl PrimitiveParser for Nicks<'_> {
    fn parse(bytes: BytesWrapper) -> IResult<BytesWrapper, Self> {
        let mut it = iterator(
            bytes,
            terminated(take_till(|c| c == b' '), tag(" ".as_bytes())),
        );

        let parsed = it.map(|v| Nick(v.into())).collect();

        it.finish()
            .map(move |(remaining, _)| (remaining, Self(parsed)))
    }
}

#[derive(Debug, Clone)]
pub struct RightsPrefixedNick<'a>(pub Rights, pub Nick<'a>);

impl std::fmt::Display for RightsPrefixedNick<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)?;
        self.1.fmt(f)
    }
}

#[derive(Debug, Deref, Clone, From)]
pub struct RightsPrefixedNicks<'a>(pub Vec<RightsPrefixedNick<'a>>);
space_delimited_display!(RightsPrefixedNicks<'_>);

#[derive(Debug, Clone)]
pub struct RightsPrefixedChannel<'a>(pub Rights, pub Nick<'a>);

impl std::fmt::Display for RightsPrefixedChannel<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)?;
        self.1.fmt(f)
    }
}

#[derive(Debug, Deref, Clone, From)]
pub struct RightsPrefixedChannels<'a>(pub Vec<RightsPrefixedChannel<'a>>);
space_delimited_display!(RightsPrefixedChannels<'_>);

#[derive(Debug, Copy, Clone)]
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

#[derive(Debug, From, Clone)]
pub enum Receiver<'a> {
    User(Nick<'a>),
    Channel(Channel<'a>),
}

impl std::ops::Deref for Receiver<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        std::str::from_utf8(match self {
            Self::User(nick) => &*nick,
            Self::Channel(channel) => &*channel,
        })
        .unwrap()
    }
}

impl PrimitiveParser for Receiver<'_> {
    fn parse(bytes: BytesWrapper) -> IResult<BytesWrapper, Self> {
        if bytes.get(0) == Some(&b'#') {
            let (rest, channel) = Channel::parse(bytes)?;
            Ok((rest, Self::Channel(channel)))
        } else {
            let (rest, nick) = Nick::parse(bytes)?;
            Ok((rest, Self::User(nick)))
        }
    }
}
