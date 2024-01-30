#![allow(clippy::iter_without_into_iter)]

mod authenticate;
pub mod sasl;

use std::{
    io::{Error, ErrorKind},
    net::SocketAddr,
    str::FromStr,
};

use actix::{io::FramedWrite, Actor, Addr};
use bitflags::bitflags;
use chrono::Utc;
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
    connection::{
        authenticate::{Authenticate, AuthenticateMessage, AuthenticateResult},
        sasl::{AuthStrategy, ConnectionSuccess, SaslSuccess},
    },
    persistence::{events::ReserveNick, Persistence},
};

pub type MessageStream = FramedRead<ReadHalf<TcpStream>, irc_proto::IrcCodec>;
pub type MessageSink = FramedWrite<Message, WriteHalf<TcpStream>, irc_proto::IrcCodec>;

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, sqlx::Type)]
#[sqlx(transparent)]
pub struct UserId(pub i64);

#[derive(Default)]
pub struct ConnectionRequest {
    host: Option<SocketAddr>,
    nick: Option<String>,
    user: Option<String>,
    real_name: Option<String>,
    user_id: Option<UserId>,
    capabilities: Capability,
}

#[derive(Clone, Debug)]
pub struct InitiatedConnection {
    pub host: SocketAddr,
    pub nick: String,
    pub user: String,
    pub mode: UserMode,
    pub real_name: String,
    pub user_id: UserId,
    pub capabilities: Capability,
    pub away: Option<String>,
    pub at: chrono::DateTime<Utc>,
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
            real_name: Some(real_name),
            user_id: Some(user_id),
            capabilities,
        } = value
        else {
            return Err(value);
        };

        Ok(Self {
            host,
            nick,
            user,
            mode: UserMode::empty(),
            real_name,
            user_id,
            capabilities,
            away: None,
            at: Utc::now(),
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
            Command::USER(_user, _mode, real_name) => {
                // we ignore the user here, as it will be set by the AUTHENTICATE command
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
                            Some(Capability::SUPPORTED.join(" ")),
                        ),
                    })
                    .await
                    .unwrap();
            }
            Command::CAP(_, CapSubCommand::REQ, Some(arguments), None) => {
                let mut acked = true;

                for argument in arguments.split(' ') {
                    acked = if argument == "sasl" {
                        acked
                    } else if let Ok(capability) = Capability::from_str(argument) {
                        request.capabilities |= capability;
                        acked
                    } else {
                        false
                    };
                }

                write
                    .send(AcknowledgedCapabilities(arguments, acked).into_message())
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
        write
            .send(NickNotOwnedByUser(initiated.nick).into_message())
            .await?;

        return Err(ProtocolError::Io(Error::new(
            ErrorKind::InvalidData,
            "nick is already in use by another user",
        )));
    }

    Ok(Some(initiated))
}

pub struct NickNotOwnedByUser(pub String);

impl NickNotOwnedByUser {
    #[must_use]
    pub fn into_message(self) -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::ERR_NICKNAMEINUSE,
                vec![self.0, "Nickname is already in use".to_string()],
            ),
        }
    }
}

/// Return an ACK (or NAK) to the client for their requested capabilities.
pub struct AcknowledgedCapabilities(String, bool);

impl AcknowledgedCapabilities {
    #[must_use]
    pub fn into_message(self) -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::CAP(
                Some("*".to_string()),
                if self.1 {
                    CapSubCommand::ACK
                } else {
                    CapSubCommand::NAK
                },
                None,
                Some(self.0),
            ),
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct Capability: u32 {
        const USERHOST_IN_NAMES = 0b0000_0000_0000_0000_0000_0000_0000_0001;
        const SERVER_TIME       = 0b0000_0000_0000_0000_0000_0000_0000_0010;
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct UserMode: u32 {
        /// w - user receives wallops
        const WALLOPS        = 0b0000_0000_0000_0000_0000_0000_0000_0001;
        /// o - operator flag
        const OPER           = 0b0000_0000_0000_0000_0000_0000_0000_0010;
    }
}

impl Capability {
    pub const SUPPORTED: &'static [&'static str] = &[
        "userhost-in-names",
        "server-time",
        concatcp!("sasl=", AuthStrategy::SUPPORTED),
    ];
}

impl FromStr for Capability {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "userhost-in-names" => Ok(Self::USERHOST_IN_NAMES),
            "server-time" => Ok(Self::SERVER_TIME),
            _ => Err(()),
        }
    }
}
