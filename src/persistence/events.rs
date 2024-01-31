use actix::Message;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use tracing::Span;

use crate::{
    channel::{permissions::Permission, ChannelId},
    connection::UserId,
    host_mask::{HostMask, HostMaskMap},
    messages::MessageKind,
};

#[derive(Message)]
#[rtype(result = "i64")]
pub struct ChannelCreated {
    pub name: String,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelJoined {
    pub channel_id: ChannelId,
    pub user_id: UserId,
    pub span: Span,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelParted {
    pub channel_id: ChannelId,
    pub user_id: UserId,
    pub span: Span,
}

#[derive(Message)]
#[rtype(result = "Vec<String>")]
pub struct FetchUserChannels {
    pub user_id: UserId,
    pub span: Span,
}

#[derive(Message)]
#[rtype(result = "HostMaskMap<Permission>")]
pub struct FetchAllUserChannelPermissions {
    pub channel_id: ChannelId,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct SetUserChannelPermissions {
    pub channel_id: ChannelId,
    pub mask: HostMask<'static>,
    pub permissions: Permission,
}

#[derive(Message)]
#[rtype(result = "Option<UserId>")]
pub struct FetchUserIdByNick {
    pub nick: String,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelMessage {
    pub channel_id: ChannelId,
    pub sender: String,
    pub message: String,
    pub receivers: Vec<UserId>,
    pub kind: MessageKind,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PrivateMessage {
    pub sender: String,
    pub receiver: UserId,
    pub message: String,
    pub kind: MessageKind,
}

#[derive(Message)]
#[rtype(result = "Vec<(DateTime<Utc>, String, String, MessageKind)>")]
pub struct FetchUnseenPrivateMessages {
    pub user_id: UserId,
    pub span: Span,
}

#[derive(Message)]
#[rtype(result = "Vec<(DateTime<Utc>, String, String, MessageKind)>")]
pub struct FetchUnseenChannelMessages {
    pub channel_name: String,
    pub user_id: UserId,
    pub span: Span,
}

#[derive(Message)]
#[rtype(result = "bool")]
pub struct ReserveNick {
    pub user_id: UserId,
    pub nick: String,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ServerBan {
    pub mask: HostMask<'static>,
    pub requester: UserId,
    pub reason: String,
    pub created: DateTime<Utc>,
    pub expires: Option<DateTime<Utc>>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ServerRemoveBan {
    pub mask: HostMask<'static>,
}

#[derive(Message)]
#[rtype(result = "Vec<ServerListBanEntry>")]
pub struct ServerListBan;

#[derive(Message, FromRow)]
#[rtype(result = "()")]
pub struct ServerListBanEntry {
    pub mask: HostMask<'static>,
    pub requester: String,
    pub reason: String,
    // timestamp in nanos. todo: sqlx datetime<utc>
    pub created_timestamp: i64,
    pub expires_timestamp: Option<i64>,
}
