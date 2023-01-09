use actix::Message;
use tracing::Span;

use crate::{
    channel::{permissions::Permission, ChannelId},
    connection::UserId,
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
#[rtype(result = "Option<Permission>")]
pub struct FetchUserChannelPermissions {
    pub channel_id: ChannelId,
    pub user_id: UserId,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct SetUserChannelPermissions {
    pub channel_id: ChannelId,
    pub user_id: UserId,
    pub permissions: Permission,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelMessage {
    pub channel_id: ChannelId,
    pub sender: String,
    pub message: String,
    pub receivers: Vec<UserId>,
}

#[derive(Message)]
#[rtype(result = "Vec<(String, String)>")]
pub struct FetchUnseenMessages {
    pub channel_name: String,
    pub user_id: UserId,
    pub span: Span,
}
