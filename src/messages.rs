use actix::{Addr, Message};
use anyhow::Result;
use irc_proto::{ChannelMode, Mode};
use tracing::Span;

use crate::{
    channel::Channel,
    client::Client,
    connection::{InitiatedConnection, UserId},
};

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

/// Internal event to update a user's nick.
#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct UserNickChangeInternal {
    pub old_nick: String,
    pub new_nick: String,
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

/// List all the channels a user is connected to
#[derive(Message, Clone)]
#[rtype(result = "Vec<(crate::channel::permissions::Permission, String)>")]
pub struct ConnectedChannels {
    pub span: Span,
}

/// Fetches all the channels visible to the user.
#[derive(Message, Clone)]
#[rtype(result = "super::server::response::ChannelList")]
pub struct ChannelList {
    pub span: Span,
}

/// Fetches the WHO list for the given query.
#[derive(Message, Clone)]
#[rtype(result = "super::server::response::WhoList")]
pub struct FetchWhoList {
    pub span: Span,
    pub query: String,
}

/// Fetches the WHOIS for the given query.
#[derive(Message, Clone)]
#[rtype(result = "super::server::response::Whois")]
pub struct FetchWhois {
    pub span: Span,
    pub query: String,
}

/// Sent when the user attempts to join a channel.
#[derive(Message)]
#[rtype(
    result = "Result<std::result::Result<Addr<Channel>, super::channel::response::ChannelJoinRejectionReason>>"
)]
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
pub struct ChannelMemberList {
    pub span: Span,
}

/// Retrieves the list of users currently in a channel.
#[derive(Message)]
#[rtype(result = "crate::channel::permissions::Permission")]
pub struct FetchUserPermission {
    pub span: Span,
    pub user: UserId,
}

/// Retrieves the current channel topic.
#[derive(Message)]
#[rtype(result = "super::channel::response::ChannelTopic")]
pub struct ChannelFetchTopic {
    pub span: Span,
}

/// Retrieves the WHO list for the channel.
#[derive(Message)]
#[rtype(result = "super::channel::response::ChannelWhoList")]
pub struct ChannelFetchWhoList {
    pub span: Span,
}

/// Sets the given modes on a channel.
#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelSetMode {
    pub span: Span,
    pub client: Addr<Client>,
    pub modes: Vec<Mode<ChannelMode>>,
}

/// Attempts to kick a user from a channel.
#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelKickUser {
    pub span: Span,
    pub client: Addr<Client>,
    pub user: String,
    pub reason: Option<String>,
}

/// Fetch the message of the day from the server.
#[derive(Message)]
#[rtype(result = "super::server::response::Motd")]
pub struct ServerFetchMotd {
    pub span: Span,
}

/// Returns the result of `LUSERS`.
#[derive(Message)]
#[rtype(result = "super::server::response::ListUsers")]
pub struct ServerListUsers {
    pub span: Span,
}

/// Returns the result of `ADMIN`.
#[derive(Message)]
#[rtype(result = "super::server::response::AdminInfo")]
pub struct ServerAdminInfo {
    pub span: Span,
}

/// Sent from channels to users when a user is removed from the channel.
#[derive(Message)]
#[rtype(result = "()")]
pub struct UserKickedFromChannel {
    pub channel: String,
    pub span: Span,
}

/// Sent from a particular user to a channel when the user attempts to update the
/// topic.
#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelUpdateTopic {
    pub topic: String,
    pub client: Addr<Client>,
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

#[derive(Copy, Clone, Debug, sqlx::Type)]
#[repr(i16)]
pub enum MessageKind {
    /// PRIVMSG from a client
    Normal = 0,
    /// NOTICE from a client
    Notice = 1,
}

/// Sends a message to a channel.
#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelMessage {
    pub client: Addr<Client>,
    pub kind: MessageKind,
    pub message: String,
    pub span: Span,
}

/// Invites a user to the channel.
#[derive(Message)]
#[rtype(result = "super::channel::response::ChannelInviteResult")]
pub struct ChannelInvite {
    pub nick: String,
    pub client: Addr<Client>,
    pub span: Span,
}

/// Fetches a client handle by nick from the server.
#[derive(Message)]
#[rtype(result = "Option<Addr<Client>>")]
pub struct FetchClientByNick {
    pub nick: String,
}

/// Sends a private message between two users.
#[derive(Message)]
#[rtype(result = "()")]
pub struct PrivateMessage {
    pub destination: UserId,
    pub message: String,
    pub kind: MessageKind,
    pub from: Addr<Client>,
    pub span: Span,
}
