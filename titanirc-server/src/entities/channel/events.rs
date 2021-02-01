use crate::entities::user::{User, UserUuid};
use actix::prelude::*;
use titanirc_types::RegisteredNick;

pub type JoinResult = Result<super::Handle, JoinError>;

/// Send from `User` to `Channel` via `Server`, the `Channel` then replies back
/// with a direct handle for the `User` to interact with the `Channel`.
#[derive(Message)]
#[rtype(result = "JoinResult")]
pub struct Join {
    pub channel_name: String,
    pub user_uuid: UserUuid,
    pub nick: RegisteredNick,
    pub user: Addr<User>,
}

#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum JoinError {
    /// Failed to send join request to channel: {0}
    Mailbox(#[from] actix::MailboxError),
}

/// Sent directly to every `User` when another `User` joins the channel.
#[derive(Message)]
#[rtype(result = "")]
pub struct JoinBroadcast {
    pub channel_name: String,
    pub nick: RegisteredNick,
}

impl From<Join> for JoinBroadcast {
    fn from(
        Join {
            channel_name, nick, ..
        }: Join,
    ) -> Self {
        Self { channel_name, nick }
    }
}
