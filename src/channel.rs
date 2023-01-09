pub mod permissions;
pub mod response;

use std::collections::HashMap;

use actix::{
    Actor, ActorContext, ActorFutureExt, Addr, AsyncContext, Context, Handler, MessageResult,
    ResponseActFuture, Supervised, WrapFuture,
};
use chrono::{DateTime, Utc};
use futures::future::Either;
use irc_proto::{Command, Message, Mode};
use tracing::{debug, error, info, instrument, warn, Span};

use crate::{
    channel::{
        permissions::Permission,
        response::{
            ChannelInviteResult, ChannelJoinRejectionReason, ChannelNamesList, ChannelTopic,
        },
    },
    client::Client,
    connection::InitiatedConnection,
    messages::{
        Broadcast, ChannelFetchTopic, ChannelInvite, ChannelJoin, ChannelKickUser,
        ChannelMemberList, ChannelMessage, ChannelPart, ChannelSetMode, ChannelUpdateTopic,
        FetchClientByNick, ServerDisconnect, UserKickedFromChannel, UserNickChange,
    },
    persistence::{
        events::{FetchUserChannelPermissions, SetUserChannelPermissions},
        Persistence,
    },
    server::Server,
};

#[derive(Copy, Clone)]
pub struct ChannelId(pub i64);

/// A channel is an IRC channel (ie. #abc) that multiple users can connect to in order
/// to chat together.
pub struct Channel {
    pub name: String,
    pub server: Addr<Server>,
    pub clients: HashMap<Addr<Client>, (Permission, InitiatedConnection)>,
    pub topic: Option<CurrentChannelTopic>,
    pub persistence: Addr<Persistence>,
    pub channel_id: ChannelId,
}

impl Actor for Channel {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.wait(
            self.persistence
                .send(crate::persistence::events::ChannelCreated {
                    name: self.name.to_string(),
                })
                .into_actor(self)
                .map(|res, this, ctx| match res {
                    Ok(channel_id) => {
                        this.channel_id.0 = channel_id;
                    }
                    Err(error) => {
                        error!(%error, "Failed to create channel in database");
                        ctx.terminate();
                    }
                }),
        );
    }
}

impl Supervised for Channel {}

/// Broadcast a raw IRC message to all clients connected to this channel.
impl Handler<Broadcast> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: Broadcast, _ctx: &mut Self::Context) -> Self::Result {
        for client in self.clients.keys() {
            client.do_send(msg.clone());
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
        let Some((permissions, sender)) = self.clients.get(&msg.client) else {
            error!("Received message from user not in channel");
            return;
        };

        if !permissions.can_chatter() {
            // TODO
            error!("User cannot send message to channel");
            return;
        }

        // build the nick prefix for the message we're about to broadcast
        let nick = sender.to_nick();

        self.persistence
            .do_send(crate::persistence::events::ChannelMessage {
                channel_id: self.channel_id,
                sender: nick.to_string(),
                message: msg.message.to_string(),
                receivers: self.clients.values().map(|(_, v)| v.user_id).collect(),
            });

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

impl Handler<ChannelSetMode> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelSetMode, ctx: &mut Self::Context) -> Self::Result {
        let Some((permissions, client)) = self.clients.get(&msg.client).cloned() else {
            return;
        };

        for mode in msg.modes {
            // TODO
            let (add, channel_mode, arg) = match mode.clone() {
                Mode::Plus(mode, arg) => (true, mode, arg),
                Mode::Minus(mode, arg) => (false, mode, arg),
            };

            if let Ok(user_mode) = Permission::try_from(channel_mode) {
                let Some(affected_nick) = arg else {
                    error!("No user given");
                    continue;
                };

                // TODO: this should allow setting perms not currently in the channel, this probably
                //  ties into fetching all user permissions on boot of the channel
                let Some((_, (affected_user_perms, affected_user))) =
                    self.clients.iter_mut().find(|(_, (_, connection))| {
                        connection.nick == affected_nick
                    }) else {
                        error!("Unknown user to set perms on");
                        continue;
                    };

                let new_affected_user_perms = if add {
                    user_mode
                } else if *affected_user_perms == user_mode {
                    Permission::Normal
                } else {
                    error!("Removing the given permission would do nothing");
                    continue;
                };

                if !permissions.can_set_permission(new_affected_user_perms, *affected_user_perms) {
                    error!(
                        ?permissions,
                        ?new_affected_user_perms,
                        ?affected_user_perms,
                        "User is not allowed to set permissions for this user"
                    );

                    continue;
                }

                self.persistence.do_send(SetUserChannelPermissions {
                    channel_id: self.channel_id,
                    user_id: affected_user.user_id,
                    permissions: new_affected_user_perms,
                });

                *affected_user_perms = new_affected_user_perms;

                ctx.notify(Broadcast {
                    message: Message {
                        tags: None,
                        prefix: Some(client.to_nick()),
                        command: Command::ChannelMODE(self.name.to_string(), vec![mode]),
                    },
                    span: Span::current(),
                });
            } else {
                // TODO
            }
        }
    }
}

/// Received when a user changes their nick.
impl Handler<UserNickChange> for Channel {
    type Result = ();

    fn handle(&mut self, msg: UserNickChange, _ctx: &mut Self::Context) -> Self::Result {
        // grab the user's current info
        let Some((_, sender)) = self.clients.get_mut(&msg.client) else {
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
///
/// This will return a `ChannelJoinRejectionReason` if the channel couldn't be joined.
impl Handler<ChannelJoin> for Channel {
    type Result = ResponseActFuture<
        Self,
        Result<Result<Addr<Self>, ChannelJoinRejectionReason>, anyhow::Error>,
    >;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelJoin, _ctx: &mut Self::Context) -> Self::Result {
        info!(self.name, msg.connection.nick, "User is joining channel");

        // TODO: maybe do the lookup at channel `started` so we dont have to do a query every time
        //  a user attempts to join the channel
        self.persistence
            .send(FetchUserChannelPermissions {
                channel_id: self.channel_id,
                user_id: msg.connection.user_id,
            })
            .into_actor(self)
            .map(move |res, this, ctx| {
                let permissions: Permission = res.unwrap().unwrap_or(Permission::Normal);

                if !permissions.can_join() {
                    return Ok(Err(ChannelJoinRejectionReason::Banned));
                }

                // persist the user's join to the database
                this.persistence
                    .do_send(crate::persistence::events::ChannelJoined {
                        channel_id: this.channel_id,
                        user_id: msg.connection.user_id,
                        span: msg.span.clone(),
                    });

                this.clients
                    .insert(msg.client.clone(), (permissions, msg.connection.clone()));

                // broadcast the user's join to everyone in the channel, including the joining user
                for client in this.clients.keys() {
                    client.do_send(Broadcast {
                        span: Span::current(),
                        message: irc_proto::Message {
                            tags: None,
                            prefix: Some(msg.connection.to_nick()),
                            command: Command::JOIN(this.name.to_string(), None, None),
                        },
                    });
                }

                // send the channel's topic to the joining user
                for message in ChannelTopic::new(this).into_messages(this.name.to_string(), true) {
                    msg.client.do_send(Broadcast {
                        message,
                        span: Span::current(),
                    });
                }

                // send the user list to the user
                for message in
                    ChannelNamesList::new(this).into_messages(msg.connection.nick.to_string())
                {
                    msg.client.do_send(Broadcast {
                        message,
                        span: Span::current(),
                    });
                }

                Ok(Ok(ctx.address()))
            })
            .boxed_local()
    }
}

/// Updates the channel topic, as requested by a connected user.
impl Handler<ChannelUpdateTopic> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelUpdateTopic, _ctx: &mut Self::Context) -> Self::Result {
        let Some((permissions, client_info)) = self.clients.get(&msg.client) else {
            return;
        };

        debug!(msg.topic, "User is attempting to update channel topic");

        if !permissions.can_set_topic() {
            // TODO
            error!("User cannot set channel topic");
            return;
        }

        self.topic = Some(CurrentChannelTopic {
            topic: msg.topic,
            set_by: client_info.nick.to_string(),
            set_time: Utc::now(),
        });

        for (client, (_, connection)) in &self.clients {
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
        let Some((permissions, kicker)) = self.clients.get(&msg.client) else {
            error!("Kicker is unknown");
            return;
        };

        if !permissions.can_kick() {
            // TODO
            error!("Kicker can not kick people from the channel");
            return;
        }

        let kicker = kicker.to_nick();

        let kicked_user = self
            .clients
            .iter()
            .find(|(_handle, (_, client))| client.nick == msg.user)
            .map(|(k, v)| (k.clone(), v));
        let Some((kicked_user_handle, (_, kicked_user_info))) = kicked_user else {
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
        let Some((_, client_info)) = self.clients.remove(&msg.client) else {
            return;
        };

        // update the client's state in the database
        self.persistence
            .do_send(crate::persistence::events::ChannelParted {
                channel_id: self.channel_id,
                user_id: client_info.user_id,
                span: msg.span.clone(),
            });

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

impl Handler<ChannelInvite> for Channel {
    type Result = ResponseActFuture<Self, ChannelInviteResult>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelInvite, _ctx: &mut Self::Context) -> Self::Result {
        let Some((_, source)) = self.clients.get(&msg.client) else {
            return Box::pin(futures::future::ready(ChannelInviteResult::NotOnChannel));
        };

        let source = source.to_nick();

        let fut = self
            .server
            .send(FetchClientByNick {
                nick: msg.nick.clone(),
            })
            .into_actor(self)
            .then(|client, this, _ctx| {
                let client = match client.unwrap() {
                    Some(v) if this.clients.contains_key(&v) => {
                        return Either::Left(futures::future::ready(
                            ChannelInviteResult::UserAlreadyOnChannel,
                        ))
                        .into_actor(this);
                    }
                    Some(v) => v,
                    None => {
                        return Either::Left(futures::future::ready(
                            ChannelInviteResult::NoSuchUser,
                        ))
                        .into_actor(this)
                    }
                };

                let channel_name = this.name.to_string();

                Either::Right(async move {
                    client
                        .send(Broadcast {
                            message: Message {
                                tags: None,
                                prefix: Some(source),
                                command: Command::INVITE(msg.nick, channel_name),
                            },
                            span: msg.span,
                        })
                        .await
                        .unwrap();

                    ChannelInviteResult::Successful
                })
                .into_actor(this)
            });

        Box::pin(fut)
    }
}

/// Received when a client is disconnecting from the server and broadcasts it to all connected
/// users.
impl Handler<ServerDisconnect> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ServerDisconnect, ctx: &mut Self::Context) -> Self::Result {
        let Some((_, client_info)) = self.clients.remove(&msg.client) else {
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
