pub mod channel;
pub mod user;

pub mod common_events {
    use actix::prelude::*;

    #[derive(Debug, Message)]
    #[rtype(result = "")]
    pub struct Message {
        pub from: titanirc_types::RegisteredNick,
        pub user_uuid: crate::entities::user::UserUuid,
        pub to: titanirc_types::protocol::primitives::Receiver<'static>,
        pub message: String,
    }
}
