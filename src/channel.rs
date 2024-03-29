pub mod permissions;
pub mod response;

use std::collections::HashMap;

use actix::{
    Actor, ActorContext, ActorFutureExt, Addr, AsyncContext, Context, Handler, MessageResult,
    ResponseActFuture, Supervised, WrapFuture,
};
use chrono::{DateTime, Utc};
use futures::future::Either;
use irc_proto::{Command, Message, Mode, Response};
use tracing::{debug, error, info, instrument, warn, Span};

use crate::{
    channel::{
        permissions::Permission,
        response::{
            BanList, ChannelInviteResult, ChannelJoinRejectionReason, ChannelNamesList,
            ChannelTopic, ChannelWhoList, MissingPrivileges, ModeList,
        },
    },
    client::Client,
    connection::{Capability, InitiatedConnection},
    host_mask::{HostMask, HostMaskMap},
    messages::{
        Broadcast, ChannelFetchTopic, ChannelFetchWhoList, ChannelInvite, ChannelJoin,
        ChannelKickUser, ChannelMemberList, ChannelMessage, ChannelPart, ChannelSetMode,
        ChannelUpdateTopic, ClientAway, FetchClientByNick, FetchUserPermission, MessageKind,
        ServerDisconnect, UserKickedFromChannel, UserNickChange,
    },
    persistence::{
        events::{FetchAllUserChannelPermissions, SetUserChannelPermissions},
        Persistence,
    },
    server::{response::IntoProtocol, Server},
};

#[derive(Copy, Clone)]
pub struct ChannelId(pub i64);

/// A channel is an IRC channel (ie. #abc) that multiple users can connect to in order
/// to chat together.
pub struct Channel {
    pub name: String,
    pub server: Addr<Server>,
    pub permissions: HostMaskMap<Permission>,
    pub clients: HashMap<Addr<Client>, InitiatedConnection>,
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
                .then(|res, this, ctx| {
                    match res {
                        Ok(channel_id) => {
                            this.channel_id.0 = channel_id;
                        }
                        Err(error) => {
                            error!(%error, "Failed to create channel in database");
                            ctx.terminate();
                        }
                    }

                    this.persistence
                        .send(FetchAllUserChannelPermissions {
                            channel_id: this.channel_id,
                        })
                        .into_actor(this)
                })
                .map(|res, this, ctx| match res {
                    Ok(permissions) => {
                        this.permissions = permissions;
                    }
                    Err(error) => {
                        error!(%error, "Failed to fetch channel permissions");
                        ctx.terminate();
                    }
                }),
        );
    }
}

impl Supervised for Channel {}

impl Channel {
    /// Grabs the user's permissions from the permission cache, defaulting to `Normal`.
    #[must_use]
    pub fn get_user_permissions(&self, host_mask: &HostMask<'_>) -> Permission {
        self.permissions
            .get(host_mask)
            .into_iter()
            .copied()
            .max()
            .unwrap_or(Permission::Normal)
    }
}

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

impl Handler<ClientAway> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ClientAway, ctx: &mut Self::Context) -> Self::Result {
        if let Some(c) = self.clients.get_mut(&msg.handle) {
            c.away = msg.message;
            ctx.notify(Broadcast {
                message: Message {
                    tags: None,
                    prefix: Some(c.to_nick()),
                    command: Command::AWAY(c.away.clone()),
                },
                span: msg.span,
            });
        }
    }
}

/// Fetches the user's permission for the current channel.
impl Handler<FetchUserPermission> for Channel {
    type Result = MessageResult<FetchUserPermission>;

    fn handle(&mut self, msg: FetchUserPermission, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(self.get_user_permissions(&msg.host_mask))
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

        if !self
            .get_user_permissions(&sender.to_host_mask())
            .can_chatter()
        {
            msg.client.do_send(Broadcast {
                message: Message {
                    tags: None,
                    prefix: None,
                    command: Command::Response(
                        Response::ERR_CANNOTSENDTOCHAN,
                        vec![
                            sender.to_nick().to_string(),
                            self.name.to_string(),
                            "Cannot send to channel".to_string(),
                        ],
                    ),
                },
                span: Span::current(),
            });

            return;
        }

        // build the nick prefix for the message we're about to broadcast
        let nick = sender.to_nick();

        // TODO: implement client msg recv acks
        self.persistence
            .do_send(crate::persistence::events::ChannelMessage {
                channel_id: self.channel_id,
                sender: nick.to_string(),
                message: msg.message.to_string(),
                receivers: self.clients.values().map(|v| v.user_id).collect(),
                kind: msg.kind,
            });

        for client in self.clients.keys() {
            if client == &msg.client {
                // don't echo the message back to the sender
                continue;
            }

            // broadcast the message to `client`
            client.do_send(Broadcast {
                span: Span::current(),
                message: Message {
                    tags: None,
                    prefix: Some(nick.clone()),
                    command: match msg.kind {
                        MessageKind::Normal => {
                            Command::PRIVMSG(self.name.to_string(), msg.message.clone())
                        }
                        MessageKind::Notice => {
                            Command::NOTICE(self.name.to_string(), msg.message.clone())
                        }
                    },
                },
            });
        }
    }
}

impl Handler<ChannelFetchWhoList> for Channel {
    type Result = MessageResult<ChannelFetchWhoList>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelFetchWhoList, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(ChannelWhoList::new(self))
    }
}

impl Handler<ChannelSetMode> for Channel {
    type Result = MessageResult<ChannelSetMode>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelSetMode, ctx: &mut Self::Context) -> Self::Result {
        let Some(client) = self.clients.get(&msg.client).cloned() else {
            return MessageResult(None);
        };

        for mode in msg.modes {
            // TODO
            let (add, channel_mode, arg) = match mode.clone() {
                Mode::Plus(mode, arg) => (true, mode, arg),
                Mode::Minus(mode, arg) => (false, mode, arg),
            };

            if let Ok(user_mode) = Permission::try_from(channel_mode) {
                let Some(affected_mask) = arg else {
                    if add && matches!(user_mode, Permission::Ban) {
                        // list is readable and the user didn't supply a mask, so
                        // return the list
                        let bans = BanList {
                            channel: self.name.to_string(),
                            list: self
                                .permissions
                                .iter()
                                .filter(|(_, v)| matches!(v, Permission::Ban))
                                .map(|(k, _)| k)
                                .collect(),
                        };

                        return MessageResult(Some(ModeList::Ban(bans)));
                    }

                    error!("No user given");
                    break;
                };

                let Ok(affected_mask) = HostMask::try_from(affected_mask.as_str()) else {
                    // TODO: return error to caller
                    error!("Invalid mask");
                    continue;
                };

                ctx.notify(SetUserMode {
                    requester: client.clone(),
                    add,
                    affected_mask: affected_mask.into_owned(),
                    user_mode,
                    span: Span::current(),
                });
            } else {
                // TODO
            }
        }

        MessageResult(None)
    }
}

/// Called by other users to set a permission on a user.
///
/// Users are currently only able to set permissions of users that are currently at a lower
/// permission level to themselves, and can only set permissions to levels lower than their
/// own.
impl Handler<SetUserMode> for Channel {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: SetUserMode, ctx: &mut Self::Context) -> Self::Result {
        let permissions = self.get_user_permissions(&msg.requester.to_host_mask());

        // grab the permissions of the user we're trying to affect
        let affected_user_perms = self.get_user_permissions(&msg.affected_mask);

        // calculate the new permissions that should be set on the user
        let new_affected_user_perms = if msg.add {
            msg.user_mode
        } else if affected_user_perms == msg.user_mode {
            Permission::Normal
        } else {
            error!("Removing the given permission would do nothing");
            return;
        };

        // check if the caller can set these permissions on the user
        if !permissions.can_set_permission(new_affected_user_perms, affected_user_perms) {
            error!(
                ?permissions,
                ?new_affected_user_perms,
                ?affected_user_perms,
                "User is not allowed to set permissions for this user"
            );

            return;
        }

        // persist the permissions change both locally and to the database
        self.permissions
            .insert(&msg.affected_mask, new_affected_user_perms);
        self.persistence.do_send(SetUserChannelPermissions {
            channel_id: self.channel_id,
            mask: msg.affected_mask.clone().into_owned(),
            permissions: new_affected_user_perms,
        });

        let Some(mode) = msg
            .user_mode
            .into_mode(msg.add, msg.affected_mask.to_string())
        else {
            return;
        };

        ctx.notify(Broadcast {
            message: Message {
                tags: None,
                prefix: Some(msg.requester.to_nick()),
                command: Command::ChannelMODE(self.name.to_string(), vec![mode]),
            },
            span: Span::current(),
        });
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
///
/// This will return a `ChannelJoinRejectionReason` if the channel couldn't be joined.
impl Handler<ChannelJoin> for Channel {
    type Result = MessageResult<ChannelJoin>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelJoin, ctx: &mut Self::Context) -> Self::Result {
        info!(self.name, msg.connection.nick, "User is joining channel");

        let mut permissions = self
            .permissions
            .get(&msg.connection.to_host_mask())
            .into_iter()
            .copied()
            .max()
            .unwrap_or(Permission::Normal);

        if !permissions.can_join() {
            return MessageResult(Ok(Err(ChannelJoinRejectionReason::Banned)));
        }

        // persist the user's join to the database
        self.persistence
            .do_send(crate::persistence::events::ChannelJoined {
                channel_id: self.channel_id,
                user_id: msg.connection.user_id,
                span: msg.span.clone(),
            });

        // we need to send out the set user channel permissions after the channel joined persistence
        // event has been sent so the user's row exists
        if self.permissions.is_empty() {
            // the first person to ever join the channel should get founder permissions
            permissions = Permission::Founder;

            let username_mask = HostMask::new("*", &msg.connection.user, "*");

            self.permissions.insert(&username_mask, permissions);

            self.persistence.do_send(SetUserChannelPermissions {
                channel_id: self.channel_id,
                mask: username_mask.into_owned(),
                permissions,
            });
        }

        self.clients
            .insert(msg.client.clone(), msg.connection.clone());

        // broadcast the user's join to everyone in the channel, including the joining user
        for client in self.clients.keys() {
            client.do_send(Broadcast {
                span: Span::current(),
                message: Message {
                    tags: None,
                    prefix: Some(msg.connection.to_nick()),
                    command: Command::JOIN(self.name.to_string(), None, None),
                },
            });

            if let Some(mode) = permissions.into_mode(true, msg.connection.nick.to_string()) {
                client.do_send(Broadcast {
                    span: Span::current(),
                    message: Message {
                        tags: None,
                        prefix: Some(msg.connection.to_nick()),
                        command: Command::ChannelMODE(self.name.to_string(), vec![mode]),
                    },
                });
            }
        }

        // send the channel's topic to the joining user
        for message in ChannelTopic::new(self, true).into_messages(&self.name) {
            msg.client.do_send(Broadcast {
                message,
                span: Span::current(),
            });
        }

        // send the user list to the user
        for message in ChannelNamesList::new(self).into_messages(
            msg.connection.nick.to_string(),
            msg.connection
                .capabilities
                .contains(Capability::USERHOST_IN_NAMES),
        ) {
            msg.client.do_send(Broadcast {
                message,
                span: Span::current(),
            });
        }

        MessageResult(Ok(Ok(ctx.address())))
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

        if !self
            .get_user_permissions(&client_info.to_host_mask())
            .can_set_topic()
        {
            error!("User attempted to set channel topic without privileges");
            msg.client.do_send(Broadcast {
                message: MissingPrivileges(client_info.to_nick(), self.name.to_string())
                    .into_message(),
                span: Span::current(),
            });
            return;
        }

        self.topic = Some(CurrentChannelTopic {
            topic: msg.topic,
            set_by: client_info.nick.to_string(),
            set_time: Utc::now(),
        });

        for (client, connection) in &self.clients {
            for message in ChannelTopic::new(self, false).into_messages(&connection.nick) {
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

        if !self.get_user_permissions(&kicker.to_host_mask()).can_kick() {
            error!("Kicker can not kick people from the channel");
            msg.client.do_send(Broadcast {
                message: MissingPrivileges(kicker.to_nick(), self.name.to_string()).into_message(),
                span: Span::current(),
            });
            return;
        }

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
        MessageResult(ChannelTopic::new(self, msg.skip_on_none))
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

        // update the client's state in the database
        self.persistence
            .do_send(crate::persistence::events::ChannelParted {
                channel_id: self.channel_id,
                user_id: client_info.user_id,
                span: msg.span.clone(),
            });

        let message = Broadcast {
            message: Message {
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
        let Some(source) = self.clients.get(&msg.client) else {
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
        let Some(client_info) = self.clients.remove(&msg.client) else {
            return;
        };

        let message = Broadcast {
            span: Span::current(),
            message: Message {
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

#[derive(actix::Message)]
#[rtype(result = "()")]
pub struct SetUserMode {
    requester: InitiatedConnection,
    add: bool,
    affected_mask: HostMask<'static>,
    user_mode: Permission,
    span: Span,
}
