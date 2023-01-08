use std::{
    io::{Error, ErrorKind},
    str::FromStr,
};

use actix::io::FramedWrite;
use argon2::PasswordHash;
use base64::{prelude::BASE64_STANDARD, Engine};
use const_format::concatcp;
use futures::{SinkExt, TryStreamExt};
use irc_proto::{
    error::ProtocolError, CapSubCommand, Command, IrcCodec, Message, Prefix, Response,
};
use tokio::{
    io::{ReadHalf, WriteHalf},
    net::TcpStream,
};
use tokio_util::codec::FramedRead;
use tracing::{instrument, warn};

use crate::database::verify_password;

pub type MessageStream = FramedRead<ReadHalf<TcpStream>, irc_proto::IrcCodec>;
pub type MessageSink = FramedWrite<Message, WriteHalf<TcpStream>, irc_proto::IrcCodec>;

pub const SUPPORTED_CAPABILITIES: &[&str] = &[concatcp!("sasl=", AuthStrategy::SUPPORTED)];

#[derive(Default)]
pub struct ConnectionRequest {
    nick: Option<String>,
    user: Option<String>,
    mode: Option<String>,
    real_name: Option<String>,
}

#[derive(Clone)]
pub struct InitiatedConnection {
    pub nick: String,
    pub user: String,
    pub mode: String,
    pub real_name: String,
}

impl InitiatedConnection {
    #[must_use]
    pub fn to_nick(&self) -> Prefix {
        Prefix::Nickname(
            self.nick.to_string(),
            self.user.to_string(),
            "my-host".to_string(),
        )
    }
}

impl TryFrom<ConnectionRequest> for InitiatedConnection {
    type Error = ConnectionRequest;

    fn try_from(value: ConnectionRequest) -> Result<Self, Self::Error> {
        let ConnectionRequest {
            nick: Some(nick),
            user: Some(user),
            mode: Some(mode),
            real_name: Some(real_name),
        } = value else {
            return Err(value);
        };

        Ok(Self {
            nick,
            user,
            mode,
            real_name,
        })
    }
}

/// Currently just awaits client preamble (nick, user), but can be expanded to negotiate
/// capabilities with the client in the future.
#[instrument(skip_all)]
pub async fn negotiate_client_connection(
    s: &mut MessageStream,
    write: &mut tokio_util::codec::FramedWrite<WriteHalf<TcpStream>, IrcCodec>,
    database: sqlx::Pool<sqlx::Any>,
) -> Result<Option<InitiatedConnection>, ProtocolError> {
    let mut request = ConnectionRequest::default();

    let mut capabilities_requested = false;

    // wait for the initiating commands from the user, giving us their NICK & USER and the user
    // requesting the server's capabilities - any clients not requesting capabilities are not
    // supported, as SASL auth is required
    let initiated = loop {
        let Some(msg) = s.try_next().await? else {
            break None;
        };

        #[allow(clippy::match_same_arms)]
        match msg.command {
            Command::PASS(_) => {}
            Command::NICK(nick) => request.nick = Some(nick),
            Command::USER(user, mode, real_name) => {
                request.user = Some(user);
                request.mode = Some(mode);
                request.real_name = Some(real_name);
            }
            Command::CAP(_, CapSubCommand::LIST | CapSubCommand::LS, _, _) => {
                capabilities_requested = true;

                write
                    .send(Message {
                        tags: None,
                        prefix: None,
                        command: Command::CAP(
                            Some("*".to_string()),
                            CapSubCommand::LS,
                            None,
                            Some(SUPPORTED_CAPABILITIES.join(" ")),
                        ),
                    })
                    .await
                    .unwrap();
            }
            _ => {
                warn!(?msg, "Client sent unknown command during negotiation");
            }
        };

        match InitiatedConnection::try_from(std::mem::take(&mut request)) {
            Ok(v) => break Some(v),
            Err(v) => {
                // connection isn't fully initiated yet...
                request = v;
            }
        }
    };

    // if the user closed the connection before the connection was fully established,
    // return back early
    let Some(initiated) = initiated else {
        return Ok(None);
    };

    if !capabilities_requested {
        return Err(ProtocolError::Io(Error::new(
            ErrorKind::InvalidData,
            "capabilities not requested by client, so SASL authentication can not be performed",
        )));
    }

    let mut has_authenticated = false;

    // start negotiating capabilities with the client
    while let Some(msg) = s.try_next().await? {
        match msg.command {
            Command::CAP(_, CapSubCommand::REQ, Some(arguments), None) => {
                write
                    .send(AcknowledgedCapabilities(arguments).into_message())
                    .await?;
            }
            Command::CAP(_, CapSubCommand::END, _, _) => {
                break;
            }
            Command::AUTHENTICATE(strategy) => {
                has_authenticated =
                    start_authenticate_flow(s, write, &initiated, strategy, &database).await?;
            }
            _ => {
                return Err(ProtocolError::Io(Error::new(
                    ErrorKind::InvalidData,
                    format!("client sent non-cap message during negotiation {msg:?}"),
                )))
            }
        }
    }

    if has_authenticated {
        Ok(Some(initiated))
    } else {
        Err(ProtocolError::Io(Error::new(
            ErrorKind::InvalidData,
            "user has not authenticated",
        )))
    }
}

/// When the client has given us a strategy to use, we can start the authentication flow.
///
/// This function will return true or false, depending on whether authentication was successful,
/// or an `Err` if an internal error occurs.
async fn start_authenticate_flow(
    s: &mut MessageStream,
    write: &mut tokio_util::codec::FramedWrite<WriteHalf<TcpStream>, IrcCodec>,
    connection: &InitiatedConnection,
    strategy: String,
    database: &sqlx::Pool<sqlx::Any>,
) -> Result<bool, ProtocolError> {
    let Ok(auth_strategy) = AuthStrategy::from_str(&strategy) else {
        write.send(SaslStrategyUnsupported(connection.nick.to_string()).into_message())
            .await?;
        return Ok(false);
    };

    // tell the client to go ahead with their authentication
    write
        .send(Message {
            tags: None,
            prefix: None,
            command: Command::AUTHENTICATE("+".to_string()),
        })
        .await?;

    // consume all AUTHENTICATE messages from the client
    while let Some(msg) = s.try_next().await? {
        let Command::AUTHENTICATE(arguments) = msg.command else {
            return Err(ProtocolError::Io(Error::new(
                ErrorKind::InvalidData,
                format!("client sent invalid message during authentication {msg:?}"),
            )));
        };

        // user has cancelled authentication
        if arguments == "*" {
            write
                .send(SaslAborted(connection.nick.to_string()).into_message())
                .await?;
            break;
        }

        let authenticated = match auth_strategy {
            AuthStrategy::Plain => {
                // TODO: this needs to deal with the case where the full arguments can be split over
                //  multiple messages
                handle_plain_authentication(&arguments, connection, database).await?
            }
        };

        if authenticated {
            write
                .send(SaslSuccess(connection.nick.to_string()).into_message())
                .await?;

            return Ok(true);
        }

        write
            .send(SaslFail(connection.nick.to_string()).into_message())
            .await?;
    }

    Ok(false)
}

/// Attempts to handle an `AUTHENTICATE` command for the `PLAIN` authentication method.
///
/// This will parse the full message, ensure that the identity is correct and compare the hashes
/// to what we have stored in the database.
pub async fn handle_plain_authentication(
    arguments: &str,
    connection: &InitiatedConnection,
    database: &sqlx::Pool<sqlx::Any>,
) -> Result<bool, Error> {
    let arguments = BASE64_STANDARD
        .decode(arguments)
        .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

    // split the PLAIN message into its respective parts
    let mut message = arguments.splitn(3, |f| *f == b'\0');
    let (Some(authorization_identity), Some(authentication_identity), Some(password)) = (message.next(), message.next(), message.next()) else {
        return Err(Error::new(ErrorKind::InvalidData, "bad plain message"));
    };

    // we don't want any ambiguity here, so we only identities matching the `USER` command
    if authorization_identity != connection.user.as_bytes()
        || authentication_identity != connection.user.as_bytes()
    {
        return Err(Error::new(ErrorKind::InvalidData, "username mismatch"));
    }

    // lookup the user's password based on the USER command they sent earlier
    let password_hash = crate::database::fetch_password_hash(database, &connection.user)
        .await
        .unwrap();
    let password_hash = password_hash
        .as_deref()
        .map(PasswordHash::new)
        .transpose()
        .unwrap();
    let Some(password_hash) = password_hash else {
        // this is a new user, so we'll create an account for them
        // TODO: we need to deal with races here, right now we'll just error out on dup
        crate::database::create_user(database, &connection.user, password).await.unwrap();

        return Ok(true);
    };

    // check the user's password
    match verify_password(password, &password_hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(Error::new(ErrorKind::InvalidData, e.to_string())),
    }
}

/// Return an acknowledgement to the client for their requested capabilities.
pub struct AcknowledgedCapabilities(String);

impl AcknowledgedCapabilities {
    #[must_use]
    pub fn into_message(self) -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::CAP(
                Some("*".to_string()),
                CapSubCommand::ACK,
                None,
                Some(self.0),
            ),
        }
    }
}

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
pub struct SaslStrategyUnsupported(String);

impl SaslStrategyUnsupported {
    #[must_use]
    pub fn into_message(self) -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::RPL_SASLMECHS,
                vec![
                    self.0,
                    AuthStrategy::SUPPORTED.to_string(),
                    "are available SASL mechanisms".to_string(),
                ],
            ),
        }
    }
}

/// Returned to the client when authentication is successful.
pub struct SaslSuccess(String);

impl SaslSuccess {
    #[must_use]
    pub fn into_message(self) -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::RPL_SASLSUCCESS,
                vec![self.0, "SASL authentication successful".to_string()],
            ),
        }
    }
}

/// Returned to the client when SASL authentication fails.
pub struct SaslFail(String);

impl SaslFail {
    #[must_use]
    pub fn into_message(self) -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::ERR_SASLFAIL,
                vec![self.0, "SASL authentication failed".to_string()],
            ),
        }
    }
}

/// Returned to the client when they abort SASL.
pub struct SaslAborted(String);

impl SaslAborted {
    #[must_use]
    pub fn into_message(self) -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::ERR_SASLABORT,
                vec![self.0, "SASL authentication aborted".to_string()],
            ),
        }
    }
}
