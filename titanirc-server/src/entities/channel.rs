use std::sync::Arc;

use actix::prelude::*;

use crate::{entities::user::User, server::Server};

pub mod events {
    use crate::entities::user::User;
    use actix::prelude::*;

    pub type JoinResult = Result<super::Handle, JoinError>;

    #[derive(Message)]
    #[rtype(result = "JoinResult")]
    pub struct Join {
        pub channel_name: String,
        pub nick: String,
        pub user: Addr<User>,
    }

    #[derive(Debug, thiserror::Error, displaydoc::Display)]
    pub enum JoinError {
        /// Failed to send join request to channel: {0}
        Mailbox(#[from] actix::MailboxError),
    }

    #[derive(Message)]
    #[rtype(result = "")]
    pub struct JoinBroadcast {
        pub channel_name: String,
        pub nick: String,
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
}

pub struct Handle {
    //pub name_change: actix::Recipient<super::user::events::NameChange>,
    pub message: actix::Recipient<super::common_events::Message>,
}

pub struct Channel {
    pub members: Vec<Addr<User>>,
}

impl Channel {
    fn announce_join(&self, join: events::Join) -> impl Future<Output = ()> {
        let mut futures = Vec::new();

        let broadcast = Arc::new(events::JoinBroadcast::from(join));

        for member in &self.members {
            futures.push(member.send(broadcast.clone()));
        }

        async {
            futures_util::future::try_join_all(futures).await.unwrap();
        }
    }
}

impl Actor for Channel {
    type Context = Context<Self>;
}

impl actix::Handler<events::Join> for Channel {
    type Result = events::JoinResult;

    fn handle(&mut self, msg: events::Join, ctx: &mut Self::Context) -> Self::Result {
        self.members.push(msg.user.clone());

        ctx.spawn(self.announce_join(msg).into_actor(self));

        Ok(Handle {
            message: ctx.address().recipient(),
        })
    }
}

impl actix::Handler<super::common_events::Message> for Channel {
    type Result = ();

    fn handle(
        &mut self,
        msg: super::common_events::Message,
        ctx: &mut Self::Context,
    ) -> Self::Result {
    }
}
