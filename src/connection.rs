use std::{
    io::{Error, ErrorKind},
    net::SocketAddr,
    str::FromStr,
};

use actix::{io::FramedWrite, Addr};
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

use crate::{
    database::verify_password,
    persistence::{events::ReserveNick, Persistence},
};

pub type MessageStream = FramedRead<ReadHalf<TcpStream>, irc_proto::IrcCodec>;
pub type MessageSink = FramedWrite<Message, WriteHalf<TcpStream>, irc_proto::IrcCodec>;

pub const SUPPORTED_CAPABILITIES: &[&str] = &[concatcp!("sasl=", AuthStrategy::SUPPORTED)];

#[derive(Copy, Clone, Debug)]
pub struct UserId(pub i64);

#[derive(Default)]
pub struct ConnectionRequest {
    host: Option<SocketAddr>,
    nick: Option<String>,
    user: Option<String>,
    mode: Option<String>,
    real_name: Option<String>,
}

#[derive(Clone)]
pub struct InitiatedConnection {
    pub host: SocketAddr,
    pub nick: String,
    pub user: String,
    pub mode: String,
    pub real_name: String,
    pub user_id: UserId,
}

impl InitiatedConnection {
    #[must_use]
    pub fn to_nick(&self) -> Prefix {
        Prefix::Nickname(
            self.nick.to_string(),
            self.user.to_string(),
            self.host.ip().to_string(),
        )
    }
}

impl TryFrom<ConnectionRequest> for InitiatedConnection {
    type Error = ConnectionRequest;

    fn try_from(value: ConnectionRequest) -> Result<Self, Self::Error> {
        let ConnectionRequest {
            host: Some(host),
            nick: Some(nick),
            user: Some(user),
            mode: Some(mode),
            real_name: Some(real_name),
        } = value else {
            return Err(value);
        };

        Ok(Self {
            host,
            nick,
            user,
            mode,
            real_name,
            user_id: UserId(0),
        })
    }
}

/// Currently just awaits client preamble (nick, user), but can be expanded to negotiate
/// capabilities with the client in the future.
#[instrument(skip_all)]
pub async fn negotiate_client_connection(
    s: &mut MessageStream,
    write: &mut tokio_util::codec::FramedWrite<WriteHalf<TcpStream>, IrcCodec>,
    host: SocketAddr,
    persistence: &Addr<Persistence>,
    database: sqlx::Pool<sqlx::Any>,
) -> Result<Option<InitiatedConnection>, ProtocolError> {
    let mut request = ConnectionRequest {
        host: Some(host),
        ..ConnectionRequest::default()
    };

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
    let Some(mut initiated) = initiated else {
        return Ok(None);
    };

    if !capabilities_requested {
        return Err(ProtocolError::Io(Error::new(
            ErrorKind::InvalidData,
            "capabilities not requested by client, so SASL authentication can not be performed",
        )));
    }

    let mut user_id = None;

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
                user_id =
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

    if let Some(user_id) = user_id {
        initiated.user_id.0 = user_id;

        let reserved_nick = persistence
            .send(ReserveNick {
                user_id: initiated.user_id,
                nick: initiated.nick.clone(),
            })
            .await
            .map_err(|e| ProtocolError::Io(Error::new(ErrorKind::InvalidData, e)))?;

        if !reserved_nick {
            return Err(ProtocolError::Io(Error::new(
                ErrorKind::InvalidData,
                "nick is already in use by another user",
            )));
        }

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
) -> Result<Option<i64>, ProtocolError> {
    let Ok(auth_strategy) = AuthStrategy::from_str(&strategy) else {
        write.send(SaslStrategyUnsupported(connection.nick.to_string()).into_message())
            .await?;
        return Ok(None);
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

        let user_id = match auth_strategy {
            AuthStrategy::Plain => {
                // TODO: this needs to deal with the case where the full arguments can be split over
                //  multiple messages
                handle_plain_authentication(&arguments, connection, database).await?
            }
        };

        if user_id.is_some() {
            for message in SaslSuccess(connection.clone()).into_messages() {
                write.send(message).await?;
            }

            return Ok(user_id);
        }

        write
            .send(SaslFail(connection.nick.to_string()).into_message())
            .await?;
    }

    Ok(None)
}

/// Attempts to handle an `AUTHENTICATE` command for the `PLAIN` authentication method.
///
/// This will parse the full message, ensure that the identity is correct and compare the hashes
/// to what we have stored in the database.
///
/// This function will return the authenticated user id, or none if the password was incorrect.
pub async fn handle_plain_authentication(
    arguments: &str,
    connection: &InitiatedConnection,
    database: &sqlx::Pool<sqlx::Any>,
) -> Result<Option<i64>, Error> {
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
    let (user_id, password_hash) =
        crate::database::create_user_or_fetch_password_hash(database, &connection.user, password)
            .await
            .unwrap();
    let password_hash = PasswordHash::new(&password_hash).unwrap();

    // check the user's password
    match verify_password(password, &password_hash) {
        Ok(()) => Ok(Some(user_id)),
        Err(argon2::password_hash::Error::Password) => Ok(None),
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
pub struct SaslSuccess(InitiatedConnection);

impl SaslSuccess {
    #[must_use]
    pub fn into_messages(self) -> [Message; 2] {
        [
            Message {
                tags: None,
                prefix: None,
                command: Command::Response(
                    Response::RPL_SASLSUCCESS,
                    vec![
                        self.0.nick.to_string(),
                        "SASL authentication successful".to_string(),
                    ],
                ),
            },
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
            },
        ]
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
