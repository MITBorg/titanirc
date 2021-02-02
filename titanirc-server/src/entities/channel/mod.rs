pub mod events;

use actix::prelude::*;
use bytes::Bytes;
use derive_more::Deref;
use std::{collections::HashMap, sync::Arc};

use crate::entities::user::{User, UserUuid};

#[derive(Clone, Debug, Deref, Eq, PartialEq, Hash)]
#[allow(clippy::module_name_repetitions)]
pub struct ChannelName(Bytes);

impl ChannelName {
    pub fn new(name: Bytes) -> Self {
        Self(name)
    }
}

impl std::borrow::Borrow<[u8]> for ChannelName {
    fn borrow(&self) -> &[u8] {
        &self.0[..]
    }
}

/// A handle to this `Channel` that `User` actors can use to communicate with
/// the rest of the channel. This is sent back to the `User` when it sends a
/// `events::Join`.
#[derive(Message)]
#[rtype(result = "")]
pub struct Handle {
    pub channel_name: ChannelName,
    //pub name_change: actix::Recipient<super::user::events::NameChange>,
    pub message: actix::Recipient<super::common_events::ChannelMessage>,
}

/// An IRC channel.
pub struct Channel {
    pub channel_name: ChannelName,
    pub members: HashMap<UserUuid, Addr<User>>,
}

impl Channel {
    pub fn new(channel_name: ChannelName) -> Self {
        Self {
            channel_name,
            members: HashMap::new(),
        }
    }

    // TODO: add a flag not to broadcast messages to the source so PRIVMSGs dont get duplicated
    fn broadcast_message<M>(&self, skip_sender: Option<UserUuid>, msg: Arc<M>)
    where
        M: Message + Send + Sync,
        M::Result: Send,
        Arc<M>: 'static,
        User: Handler<Arc<M>>,
    {
        for (uuid, member) in &self.members {
            if let Some(skip_sender) = &skip_sender {
                if skip_sender == uuid {
                    continue;
                }
            }

            // TODO: should this just be `send`?
            member.do_send(msg.clone());
        }
    }
}

impl Actor for Channel {
    type Context = Context<Self>;
}

impl actix::Handler<events::Join> for Channel {
    type Result = ();

    fn handle(&mut self, event: events::Join, ctx: &mut Self::Context) -> Self::Result {
        self.members.insert(event.user_uuid, event.user.clone());

        let event = Arc::new(event);

        self.broadcast_message(None, Arc::clone(&event));

        event.user.do_send(Handle {
            channel_name: self.channel_name.clone(),
            message: ctx.address().recipient(),
        })
    }
}

impl actix::Handler<super::common_events::ChannelMessage> for Channel {
    type Result = ();

    fn handle(
        &mut self,
        msg: super::common_events::ChannelMessage,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        // TODO: don't allow messages from unconnected clients
        self.broadcast_message(Some(msg.0.user_uuid), Arc::new(msg));
    }
}
