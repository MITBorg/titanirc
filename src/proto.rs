use std::{convert::identity, str::FromStr, time::Duration};

use irc_proto::{Command, Message, Prefix, Response};
use thiserror::Error;

use crate::{host_mask::HostMask, server::response::IntoProtocol, SERVER_NAME};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalCommand {
    ListGline,
    /// Unbans a hostmask
    RemoveGline(HostMask<'static>),
    /// Bans a hostmask from the network for the given duration with the given message
    Gline(HostMask<'static>, Option<Duration>, Option<String>),
}

impl TryFrom<(String, Vec<String>)> for LocalCommand {
    type Error = Error;

    fn try_from((command, args): (String, Vec<String>)) -> Result<Self, Self::Error> {
        match command.as_str() {
            "GLINE" if args.is_empty() => Ok(Self::ListGline),
            "GLINE" if args.len() == 1 && args[0].starts_with('-') => parse1(
                Self::RemoveGline,
                args,
                required(truncate_first_character(parse_host_mask)),
            ),
            "GLINE" => parse3(
                Self::Gline,
                args,
                required(parse_host_mask),
                opt(parse_duration),
                opt(wrap_ok(identity)),
            ),
            _ => Err(Error::UnknownCommand),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("unknown command")]
    UnknownCommand,
    #[error("missing argument")]
    MissingArgument,
    #[error("invalid duration: {0}")]
    InvalidDuration(humantime::DurationError),
    #[error("invalid host mask: {0}")]
    InvalidHostMask(std::io::Error),
    #[error("too many arguments")]
    TooManyArguments,
}

impl IntoProtocol for Error {
    fn into_messages(self, for_user: &str) -> Vec<Message> {
        vec![Message {
            tags: None,
            prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
            command: Command::Response(
                Response::ERR_UNKNOWNCOMMAND,
                vec![
                    for_user.to_string(),
                    "command".to_string(), // TODO
                    "Unknown command".to_string(),
                ],
            ),
        }]
    }
}

fn opt<T>(
    transform: impl FnOnce(String) -> Result<T, Error>,
) -> impl FnOnce(Option<String>) -> Result<Option<T>, Error> {
    move |v| v.map(transform).transpose()
}

fn required<T>(
    transform: impl FnOnce(String) -> Result<T, Error>,
) -> impl FnOnce(Option<String>) -> Result<T, Error> {
    move |v| v.ok_or(Error::MissingArgument).and_then(transform)
}

/// Truncates the first character from the first argument and calls the inner transform function.
fn truncate_first_character<T>(
    transform: fn(String) -> Result<T, Error>,
) -> impl Fn(String) -> Result<T, Error> {
    move |mut v| {
        v.remove(0);
        (transform)(v)
    }
}

/// Parses a host mask argument
#[allow(clippy::needless_pass_by_value)]
fn parse_host_mask(v: String) -> Result<HostMask<'static>, Error> {
    HostMask::from_str(&v).map_err(Error::InvalidHostMask)
}

/// Parses a humantime duration
#[allow(clippy::needless_pass_by_value)]
fn parse_duration(v: String) -> Result<Duration, Error> {
    humantime::parse_duration(&v).map_err(Error::InvalidDuration)
}

/// Takes a string argument as-is
fn wrap_ok<T>(transform: fn(String) -> T) -> impl Fn(String) -> Result<T, Error> {
    move |v| Ok((transform)(v))
}

/// Parses a single argument from `args`, transforming it using `t1`
/// and returns a `LocalCommand`.
fn parse1<T1>(
    out: fn(T1) -> LocalCommand,
    args: Vec<String>,
    t1: impl FnOnce(Option<String>) -> Result<T1, Error>,
) -> Result<LocalCommand, Error> {
    if args.len() > 1 {
        return Err(Error::TooManyArguments);
    }

    let mut i = args.into_iter();
    Ok((out)(t1(i.next())?))
}

/// Parses three arguments from `args`, transforming them using `t1`, `t2` and `t3`
/// and returns a `LocalCommand`.
fn parse3<T1, T2, T3>(
    out: fn(T1, T2, T3) -> LocalCommand,
    args: Vec<String>,
    t1: impl FnOnce(Option<String>) -> Result<T1, Error>,
    t2: impl FnOnce(Option<String>) -> Result<T2, Error>,
    t3: impl FnOnce(Option<String>) -> Result<T3, Error>,
) -> Result<LocalCommand, Error> {
    if args.len() > 3 {
        return Err(Error::TooManyArguments);
    }

    let mut i = args.into_iter();
    Ok((out)(t1(i.next())?, t2(i.next())?, t3(i.next())?))
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use crate::proto::{Error, LocalCommand};

    #[test]
    fn remove_gline() {
        let command =
            LocalCommand::try_from(("GLINE".to_string(), vec!["-aaa!bbb@ccc".to_string()]))
                .unwrap();
        assert_eq!(
            command,
            LocalCommand::RemoveGline("aaa!bbb@ccc".try_into().unwrap())
        );
    }

    #[test]
    fn gline() {
        let command = LocalCommand::try_from((
            "GLINE".to_string(),
            vec![
                "aaa!bbb@ccc".to_string(),
                "1d".to_string(),
                "comment".to_string(),
            ],
        ))
        .unwrap();
        assert_eq!(
            command,
            LocalCommand::Gline(
                "aaa!bbb@ccc".try_into().unwrap(),
                Some(Duration::from_secs(86_400)),
                Some("comment".to_string())
            )
        );
    }

    #[test]
    fn too_many_arguments() {
        let command = LocalCommand::try_from((
            "GLINE".to_string(),
            vec![
                "aaa!bbb@ccc".to_string(),
                "1d".to_string(),
                "comment".to_string(),
                "toomany".to_string(),
            ],
        ));
        assert!(
            matches!(command, Err(Error::TooManyArguments)),
            "{command:?}"
        );
    }
}
