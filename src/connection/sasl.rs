use std::{
    io::{Error, ErrorKind},
    str::FromStr,
};

use irc_proto::{Command, Message, Response};

use crate::connection::InitiatedConnection;

/// A requested SASL strategy.
#[derive(Copy, Clone, Debug)]
pub enum AuthStrategy {
    Plain,
}

impl AuthStrategy {
    /// A list of all supported SASL strategies.
    pub const SUPPORTED: &'static str = "PLAIN";
}

/// Parse a SASL strategy from the wire.
impl FromStr for AuthStrategy {
    type Err = std::io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PLAIN" => Ok(Self::Plain),
            _ => Err(Error::new(ErrorKind::InvalidData, "unknown auth strategy")),
        }
    }
}

/// Returned to the client if they try to call AUTHENTICATE again after negotiation.
pub struct SaslAlreadyAuthenticated(pub String);

impl SaslAlreadyAuthenticated {
    #[must_use]
    pub fn into_message(self) -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::ERR_SASLALREADY,
                vec![
                    self.0,
                    "You have already authenticated using SASL".to_string(),
                ],
            ),
        }
    }
}

/// Returned to the client when an invalid SASL strategy is attempted.
pub struct SaslStrategyUnsupported;

impl SaslStrategyUnsupported {
    #[must_use]
    pub fn into_message() -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::RPL_SASLMECHS,
                vec![
                    AuthStrategy::SUPPORTED.to_string(),
                    "are available SASL mechanisms".to_string(),
                ],
            ),
        }
    }
}

/// Returned to the client when SASL authentication is successful.
pub struct SaslSuccess;

impl SaslSuccess {
    #[must_use]
    pub fn into_message() -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::RPL_SASLSUCCESS,
                vec!["SASL authentication successful".to_string()],
            ),
        }
    }
}

/// Returned to the client when the whole connection flow is successful.
pub struct ConnectionSuccess(pub InitiatedConnection);

impl ConnectionSuccess {
    #[must_use]
    pub fn into_message(self) -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::RPL_LOGGEDIN,
                vec![
                    self.0.nick.to_string(),
                    self.0.to_nick().to_string(),
                    self.0.user.to_string(),
                    format!("You are now logged in as {}", self.0.user),
                ],
            ),
        }
    }
}

/// Returned to the client when SASL authentication fails.
pub struct SaslFail;

impl SaslFail {
    #[must_use]
    pub fn into_message() -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::ERR_SASLFAIL,
                vec!["SASL authentication failed".to_string()],
            ),
        }
    }
}

/// Returned to the client when they abort SASL.
pub struct SaslAborted;

impl SaslAborted {
    #[must_use]
    pub fn into_message() -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::ERR_SASLABORT,
                vec!["SASL authentication aborted".to_string()],
            ),
        }
    }
}
