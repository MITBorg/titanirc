use actix::{Addr, Message};
use anyhow::Result;
use tracing::Span;

use crate::{channel::Channel, client::Client, connection::InitiatedConnection};

/// Sent when a user is connecting to the server.
#[derive(Message, Clone)]
#[rtype(message = "()")]
pub struct UserConnected {
    pub handle: Addr<Client>,
    pub connection: InitiatedConnection,
    pub span: Span,
}

/// Sent to both the `Server` and all connected `Channel`s when the user disconnects from
/// the server.
#[derive(Message, Clone)]
#[rtype(message = "()")]
pub struct ServerDisconnect {
    pub client: Addr<Client>,
    pub message: Option<String>,
    pub span: Span,
}

/// Sent when the user changes their nick.
#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct UserNickChange {
    pub client: Addr<Client>,
    pub connection: InitiatedConnection,
    pub new_nick: String,
    pub span: Span,
}

/// Sent when the user attempts to join a channel.
#[derive(Message)]
#[rtype(result = "Result<Addr<Channel>>")]
pub struct ChannelJoin {
    pub channel_name: String,
    pub client: Addr<Client>,
    pub connection: InitiatedConnection,
    pub span: Span,
}

/// Sent when the user parts a channel.
#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelPart {
    pub client: Addr<Client>,
    pub message: Option<String>,
    pub span: Span,
}

/// Retrieves the list of users currently in a channel.
#[derive(Message)]
#[rtype(result = "super::channel::response::ChannelNamesList")]
pub struct ChannelList {
    pub span: Span,
}

/// Sends a raw irc message to a channel/user.
#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct Broadcast {
    pub message: irc_proto::Message,
    pub span: Span,
}

/// Fetches the user's current connection info (nick, host, etc)
#[derive(Message)]
#[rtype(result = "crate::connection::InitiatedConnection")]
pub struct FetchClientDetails {
    pub span: Span,
}

/// Sends a message to a channel.
#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelMessage {
    pub client: Addr<Client>,
    pub message: String,
    pub span: Span,
}
