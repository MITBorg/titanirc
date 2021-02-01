pub mod events;

use actix::prelude::*;
use std::{collections::HashMap, sync::Arc};

use crate::entities::user::{User, UserUuid};

use self::events::JoinBroadcast;

/// A handle to this `Channel` that `User` actors can use to communicate with the
/// rest of the channel.
pub struct Handle {
    //pub name_change: actix::Recipient<super::user::events::NameChange>,
    pub message: actix::Recipient<super::common_events::Message>,
}

/// An IRC channel.
pub struct Channel {
    pub members: HashMap<UserUuid, Addr<User>>,
}

impl Channel {
    pub fn new() -> Self {
        Self {
            members: HashMap::new(),
        }
    }

    // TODO: add a flag not to broadcast messages to the source so PRIVMSGs dont get duplicated
    fn broadcast_message<M>(
        &self,
        skip_sender: Option<UserUuid>,
        msg: M,
    ) -> impl Future<Output = ()>
    where
        M: Message + Send + Sync,
        M::Result: Send,
        Arc<M>: 'static,
        User: Handler<Arc<M>>,
    {
        let mut futures = Vec::with_capacity(self.members.len());

        let msg = Arc::new(msg);

        for (uuid, member) in &self.members {
            if let Some(skip_sender) = &skip_sender {
                if skip_sender == uuid {
                    continue;
                }
            }

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
        self.members.insert(msg.user_uuid, msg.user.clone());

        ctx.spawn(
            self.broadcast_message(None, JoinBroadcast::from(msg))
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
        // TODO: don't allow messages from unconnected clients
        ctx.spawn(
            self.broadcast_message(Some(msg.user_uuid), msg)
                .into_actor(self),
        );
    }
}
