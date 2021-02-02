use crate::entities::user::{User, UserUuid};
use actix::prelude::*;
use titanirc_types::RegisteredNick;



/// Send from `User` to `Channel` via `Server`, the `Channel` then replies back
/// with a direct handle for the `User` to interact with the `Channel`.
///
/// This message is also broadcast to all users in the `Channel`.
#[derive(Message)]
#[rtype(result = "")]
pub struct Join {
    pub channel_name: bytes::Bytes,
    pub user_uuid: UserUuid,
    pub nick: RegisteredNick,
    pub user: Addr<User>,
}
