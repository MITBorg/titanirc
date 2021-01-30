pub mod events;

use actix::prelude::*;
use std::sync::Arc;

use crate::entities::user::User;

/// A handle to this `Channel` that `User` actors can use to communicate with the
/// rest of the channel.
pub struct Handle {
    //pub name_change: actix::Recipient<super::user::events::NameChange>,
    pub message: actix::Recipient<super::common_events::Message>,
}

/// An IRC channel.
pub struct Channel {
    pub members: Vec<Addr<User>>,
}

impl Channel {
    pub fn new() -> Self {
        Self {
            members: Vec::new(),
        }
    }

    /// Announce a user's join event to the rest of the channel.
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
        _msg: super::common_events::Message,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
    }
}
