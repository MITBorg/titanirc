mod response;

use std::collections::HashMap;

use actix::{Actor, Addr, AsyncContext, Context, Handler, MessageResult};
use irc_proto::Command;
use tracing::{error, info, instrument, Span};

use crate::{
    channel::response::{ChannelNamesList, ChannelTopic},
    client::Client,
    connection::InitiatedConnection,
    messages::{
        Broadcast, ChannelJoin, ChannelList, ChannelMessage, ChannelPart, ServerDisconnect,
    },
};
use crate::messages::UserNickChange;

/// A channel is an IRC channel (ie. #abc) that multiple users can connect to in order
/// to chat together.
pub struct Channel {
    pub name: String,
    pub clients: HashMap<Addr<Client>, InitiatedConnection>,
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
impl Handler<ChannelList> for Channel {
    type Result = MessageResult<ChannelList>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelList, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(self.clients.values().cloned().collect())
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

/// Recieved when a user changes their nick.
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
        msg.client.do_send(Broadcast {
            message: ChannelTopic::new(self).into_message(self.name.to_string()),
            span: Span::current(),
        });

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
