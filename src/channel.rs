pub mod response;

use std::collections::HashMap;

use actix::{Actor, Addr, AsyncContext, Context, Handler, MessageResult};
use chrono::{DateTime, Utc};
use irc_proto::{Command, Message};
use tracing::{debug, error, info, instrument, Span};

use crate::{
    channel::response::{ChannelNamesList, ChannelTopic},
    client::Client,
    connection::InitiatedConnection,
    messages::{
        Broadcast, ChannelFetchTopic, ChannelJoin, ChannelKickUser, ChannelMemberList,
        ChannelMessage, ChannelPart, ChannelUpdateTopic, ServerDisconnect, UserKickedFromChannel,
        UserNickChange,
    },
};

/// A channel is an IRC channel (ie. #abc) that multiple users can connect to in order
/// to chat together.
pub struct Channel {
    pub name: String,
    pub clients: HashMap<Addr<Client>, InitiatedConnection>,
    pub topic: Option<CurrentChannelTopic>,
}

impl Actor for Channel {
    type Context = Context<Self>;
}

/// Broadcast a raw IRC message to all clients connected to this channel.
impl Handler<Broadcast> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: Broadcast, _ctx: &mut Self::Context) -> Self::Result {
        for client in self.clients.keys() {
            client.try_send(msg.clone()).unwrap();
        }
    }
}

/// Sends back a list of users currently connected to the client
impl Handler<ChannelMemberList> for Channel {
    type Result = MessageResult<ChannelMemberList>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelMemberList, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(ChannelNamesList::new(self))
    }
}

/// Broadcasts a message from a user to all users in the channel
impl Handler<ChannelMessage> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelMessage, _ctx: &mut Self::Context) -> Self::Result {
        // ensure the user is actually in the channel by their handle, and grab their
        // nick & host if they are
        let Some(sender) = self.clients.get(&msg.client) else {
            error!("Received message from user not in channel");
            return;
        };

        // build the nick prefix for the message we're about to broadcast
        let nick = sender.to_nick();

        for client in self.clients.keys() {
            if client == &msg.client {
                // don't echo the message back to the sender
                continue;
            }

            // broadcast the message to `client`
            client.do_send(Broadcast {
                span: Span::current(),
                message: irc_proto::Message {
                    tags: None,
                    prefix: Some(nick.clone()),
                    command: Command::PRIVMSG(self.name.to_string(), msg.message.clone()),
                },
            });
        }
    }
}

/// Received when a user changes their nick.
impl Handler<UserNickChange> for Channel {
    type Result = ();

    fn handle(&mut self, msg: UserNickChange, _ctx: &mut Self::Context) -> Self::Result {
        // grab the user's current info
        let Some(sender) = self.clients.get_mut(&msg.client) else {
            return;
        };

        // update the user's info with the latest `connection` details and new nick
        *sender = msg.connection;
        sender.nick = msg.new_nick;
    }
}

/// Received when a user is attempting to join the channel, broadcasts a message to all clients
/// informing them of the join.
///
/// Sends the current topic & user list, and returns a handle to the channel so the user can
/// start sending us messages.
impl Handler<ChannelJoin> for Channel {
    type Result = MessageResult<ChannelJoin>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelJoin, ctx: &mut Self::Context) -> Self::Result {
        info!(self.name, msg.connection.nick, "User is joining channel");

        self.clients
            .insert(msg.client.clone(), msg.connection.clone());

        // broadcast the user's join to everyone in the channel, including the joining user
        for client in self.clients.keys() {
            client.do_send(Broadcast {
                span: Span::current(),
                message: irc_proto::Message {
                    tags: None,
                    prefix: Some(msg.connection.to_nick()),
                    command: Command::JOIN(self.name.to_string(), None, None),
                },
            });
        }

        // send the channel's topic to the joining user
        for message in ChannelTopic::new(self).into_messages(self.name.to_string(), true) {
            msg.client.do_send(Broadcast {
                message,
                span: Span::current(),
            });
        }

        // send the user list to the user
        for message in ChannelNamesList::new(self).into_messages(msg.connection.nick.to_string()) {
            msg.client.do_send(Broadcast {
                message,
                span: Span::current(),
            });
        }

        MessageResult(Ok(ctx.address()))
    }
}

/// Updates the channel topic, as requested by a connected user.
impl Handler<ChannelUpdateTopic> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelUpdateTopic, _ctx: &mut Self::Context) -> Self::Result {
        let Some(client_info) = self.clients.get(&msg.client) else {
            return;
        };

        debug!(msg.topic, "User is attempting to update channel topic");

        self.topic = Some(CurrentChannelTopic {
            topic: msg.topic,
            set_by: client_info.nick.to_string(),
            set_time: Utc::now(),
        });

        for (client, connection) in &self.clients {
            for message in ChannelTopic::new(self).into_messages(connection.nick.to_string(), false)
            {
                client.do_send(Broadcast {
                    message,
                    span: Span::current(),
                });
            }
        }
    }
}

/// Sent from an oper client to remove a user from the channel.
impl Handler<ChannelKickUser> for Channel {
    type Result = ();

    fn handle(&mut self, msg: ChannelKickUser, _ctx: &mut Self::Context) -> Self::Result {
        let Some(kicker) = self.clients.get(&msg.client) else {
            error!("Kicker is unknown");
            return;
        };
        let kicker = kicker.to_nick();

        let kicked_user = self
            .clients
            .iter()
            .find(|(_handle, client)| client.nick == msg.user)
            .map(|(k, v)| (k.clone(), v));
        let Some((kicked_user_handle, kicked_user_info)) = kicked_user else {
            error!(msg.user, "Attempted to kick unknown user");
            return;
        };

        for client in self.clients.keys() {
            client.do_send(Broadcast {
                message: Message {
                    tags: None,
                    prefix: Some(kicker.clone()),
                    command: Command::KICK(
                        self.name.to_string(),
                        kicked_user_info.nick.to_string(),
                        msg.reason.clone(),
                    ),
                },
                span: Span::current(),
            });
        }

        kicked_user_handle.do_send(UserKickedFromChannel {
            channel: self.name.to_string(),
            span: Span::current(),
        });

        self.clients.remove(&kicked_user_handle);
    }
}

/// Returns the current channel topic to the user.
impl Handler<ChannelFetchTopic> for Channel {
    type Result = MessageResult<ChannelFetchTopic>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelFetchTopic, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(ChannelTopic::new(self))
    }
}

/// Received when a client is parting the channel and broadcasts it to all connected users.
impl Handler<ChannelPart> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelPart, ctx: &mut Self::Context) -> Self::Result {
        let Some(client_info) = self.clients.remove(&msg.client) else {
            return;
        };

        let message = Broadcast {
            message: irc_proto::Message {
                tags: None,
                prefix: Some(client_info.to_nick()),
                command: Command::PART(self.name.to_string(), msg.message),
            },
            span: Span::current(),
        };

        // send the part message to both the parting user and other clients
        msg.client.do_send(message.clone());
        ctx.notify(message);
    }
}

/// Received when a client is disconnecting from the server and broadcasts it to all connected
/// users.
impl Handler<ServerDisconnect> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ServerDisconnect, ctx: &mut Self::Context) -> Self::Result {
        let Some(client_info) = self.clients.remove(&msg.client) else {
            return;
        };

        let message = Broadcast {
            span: Span::current(),
            message: irc_proto::Message {
                tags: None,
                prefix: Some(client_info.to_nick()),
                command: Command::QUIT(msg.message),
            },
        };

        // send the part message to all other clients
        ctx.notify(message);
    }
}

#[derive(Clone)]
pub struct CurrentChannelTopic {
    pub topic: String,
    pub set_by: String,
    pub set_time: DateTime<Utc>,
}
