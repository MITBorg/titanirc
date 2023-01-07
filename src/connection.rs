use actix::io::FramedWrite;
use futures::TryStreamExt;
use irc_proto::{error::ProtocolError, Command, Message, Prefix};
use tokio::{
    io::{ReadHalf, WriteHalf},
    net::TcpStream,
};
use tokio_util::codec::FramedRead;
use tracing::{instrument, warn};

pub type MessageStream = FramedRead<ReadHalf<TcpStream>, irc_proto::IrcCodec>;
pub type MessageSink = FramedWrite<Message, WriteHalf<TcpStream>, irc_proto::IrcCodec>;

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
) -> Result<Option<InitiatedConnection>, ProtocolError> {
    let mut request = ConnectionRequest::default();

    while let Some(msg) = s.try_next().await? {
        #[allow(clippy::match_same_arms)]
        match msg.command {
            Command::PASS(_) => {}
            Command::NICK(nick) => request.nick = Some(nick),
            Command::USER(user, mode, real_name) => {
                request.user = Some(user);
                request.mode = Some(mode);
                request.real_name = Some(real_name);
            }
            Command::CAP(_, _, _, _) => {}
            _ => {
                warn!(?msg, "Client sent unknown command during negotiation");
            }
        }

        match InitiatedConnection::try_from(std::mem::take(&mut request)) {
            Ok(v) => return Ok(Some(v)),
            Err(v) => {
                // connection isn't fully initiated yet...
                request = v;
            }
        }
    }

    Ok(None)
}
