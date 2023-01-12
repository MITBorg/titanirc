mod authenticate;
pub mod sasl;

use std::{
    io::{Error, ErrorKind},
    net::SocketAddr,
};

use actix::{io::FramedWrite, Actor, Addr};
use const_format::concatcp;
use futures::{SinkExt, TryStreamExt};
use irc_proto::{error::ProtocolError, CapSubCommand, Command, IrcCodec, Message, Prefix};
use tokio::{
    io::{ReadHalf, WriteHalf},
    net::TcpStream,
};
use tokio_util::codec::FramedRead;
use tracing::{instrument, warn};

use crate::{
    connection::{
        authenticate::{Authenticate, AuthenticateMessage, AuthenticateResult},
        sasl::{AuthStrategy, ConnectionSuccess, SaslSuccess},
    },
    persistence::{events::ReserveNick, Persistence},
};

pub type MessageStream = FramedRead<ReadHalf<TcpStream>, irc_proto::IrcCodec>;
pub type MessageSink = FramedWrite<Message, WriteHalf<TcpStream>, irc_proto::IrcCodec>;

pub const SUPPORTED_CAPABILITIES: &[&str] = &[concatcp!("sasl=", AuthStrategy::SUPPORTED)];

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, sqlx::Type)]
#[sqlx(transparent)]
pub struct UserId(pub i64);

#[derive(Default)]
pub struct ConnectionRequest {
    host: Option<SocketAddr>,
    nick: Option<String>,
    user: Option<String>,
    mode: Option<String>,
    real_name: Option<String>,
    user_id: Option<UserId>,
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
            user_id: Some(user_id),
        } = value else {
            return Err(value);
        };

        Ok(Self {
            host,
            nick,
            user,
            mode,
            real_name,
            user_id,
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

    let authenticate_handle = Authenticate {
        selected_strategy: None,
        database: database.clone(),
    }
    .start();

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
            Command::USER(_user, mode, real_name) => {
                // we ignore the user here, as it will be set by the AUTHENTICATE command
                request.mode = Some(mode);
                request.real_name = Some(real_name);
            }
            Command::CAP(_, CapSubCommand::LIST | CapSubCommand::LS, _, _) => {
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
            Command::CAP(_, CapSubCommand::REQ, Some(arguments), None) => {
                write
                    .send(AcknowledgedCapabilities(arguments).into_message())
                    .await?;
            }
            Command::AUTHENTICATE(msg) => {
                match authenticate_handle
                    .send(AuthenticateMessage(msg))
                    .await
                    .unwrap()?
                {
                    AuthenticateResult::Reply(v) => {
                        write.send(*v).await?;
                    }
                    AuthenticateResult::Done(username, user_id) => {
                        request.user = Some(username);
                        request.user_id = Some(user_id);
                        write.send(SaslSuccess::into_message()).await?;
                    }
                }
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

    write
        .send(ConnectionSuccess(initiated.clone()).into_message())
        .await?;

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
