pub mod channel;
pub mod user;

pub mod common_events {
    use actix::prelude::*;

    #[derive(Debug, Message)]
    #[rtype(result = "")]
    pub struct Message {
        pub from: String,
        pub to: String,
        pub message: String,
    }
}
