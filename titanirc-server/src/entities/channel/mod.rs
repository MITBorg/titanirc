pub mod events;

use actix::prelude::*;
use std::sync::Arc;

use crate::entities::user::User;

use self::events::JoinBroadcast;

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

    fn broadcast_message<M>(&self, msg: M) -> impl Future<Output = ()>
    where
        M: Message + Send + Sync,
        M::Result: Send,
        Arc<M>: 'static,
        User: Handler<Arc<M>>,
    {
        let mut futures = Vec::with_capacity(self.members.len());

        let msg = Arc::new(msg);

        for member in &self.members {
            futures.push(member.send(msg.clone()));
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

        ctx.spawn(
            self.broadcast_message(JoinBroadcast::from(msg))
                .into_actor(self),
        );

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
        ctx.spawn(self.broadcast_message(msg).into_actor(self));
    }
}
