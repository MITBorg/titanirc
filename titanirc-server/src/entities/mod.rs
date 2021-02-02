pub mod channel;
pub mod user;

pub mod common_events {
    use std::fmt::Debug;

    use actix::prelude::*;

    #[derive(Debug, Message)]
    #[rtype(result = "")]
    pub struct ChannelMessage(pub Message<titanirc_types::protocol::primitives::Channel<'static>>);

    #[derive(Debug, Message)]
    #[rtype(result = "")]
    pub struct UserMessage(pub Message<titanirc_types::protocol::primitives::Nick<'static>>);

    #[derive(Debug)]
    pub struct Message<T: Debug> {
        pub from: titanirc_types::RegisteredNick,
        pub user_uuid: crate::entities::user::UserUuid,
        pub to: T,
        pub message: String,
    }
}
