use std::{collections::HashMap, time::Duration};

use actix::{
    dev::ToEnvelope, fut::wrap_future, io::WriteHandler, Actor, ActorContext, ActorFuture,
    ActorFutureExt, Addr, AsyncContext, Context, Handler, MessageResult, ResponseActFuture,
    ResponseFuture, Running, StreamHandler, WrapFuture,
};
use chrono::{DateTime, SecondsFormat, Utc};
use clap::{crate_name, crate_version};
use futures::{future, stream::FuturesUnordered, FutureExt, StreamExt};
use irc_proto::{
    error::ProtocolError, message::Tag, ChannelExt, Command, Message, Prefix, Response,
};
use tokio::time::Instant;
use tracing::{debug, error, info, instrument, warn, Instrument, Span};

use crate::{
    channel::Channel,
    connection::{
        sasl::SaslAlreadyAuthenticated, Capability, InitiatedConnection, MessageSink,
        NickNotOwnedByUser, UserMode,
    },
    messages::{
        Broadcast, ChannelFetchTopic, ChannelFetchWhoList, ChannelInvite, ChannelJoin,
        ChannelKickUser, ChannelList, ChannelMemberList, ChannelMessage, ChannelPart,
        ChannelSetMode, ChannelUpdateTopic, ClientAway, ConnectedChannels, FetchClientDetails,
        FetchUserPermission, FetchWhoList, FetchWhois, KillUser, MessageKind, PrivateMessage,
        ServerAdminInfo, ServerDisconnect, ServerFetchMotd, ServerListUsers, UserKickedFromChannel,
        UserNickChange, UserNickChangeInternal, Wallops,
    },
    persistence::{
        events::{
            FetchUnseenChannelMessages, FetchUnseenPrivateMessages, FetchUserChannels,
            FetchUserIdByNick, ReserveNick,
        },
        Persistence,
    },
    server::{response::WhoList, Server},
    SERVER_NAME,
};

/// A client refers to a single connection to the server.
///
/// This client has a handle to the server to inform it of leaves, and to request handles to
/// channels.
pub struct Client {
    /// The TcpStream writer half for sending responses to the client
    pub writer: MessageSink,
    /// Details about the user's connection, including their nick
    pub connection: InitiatedConnection,
    /// A handle to the root actor for arbitration between clients and channels
    pub server: Addr<Server>,
    /// A list of channels the user is currently connected to
    pub channels: HashMap<String, Addr<Channel>>,
    /// The time of the last ping we received from the client
    pub last_active: Instant,
    /// Whether the client is shutting down due to the client calling QUIT, or whether the server
    /// terminated the connection
    pub graceful_shutdown: bool,
    /// The reason the client is leaving the server, whether this is set by the server or the user
    /// is decided by graceful_shutdown
    pub server_leave_reason: Option<String>,
    /// Actor for persisting state to the datastore.
    pub persistence: Addr<Persistence>,
    /// The connection span to group all logs for the same connection
    pub span: Span,
}

impl Client {
    #[must_use]
    pub fn maybe_build_time_tag(&self, time: DateTime<Utc>) -> Option<Tag> {
        if !self
            .connection
            .capabilities
            .contains(Capability::SERVER_TIME)
        {
            return None;
        }

        Some(Tag(
            "time".to_string(),
            Some(time.to_rfc3339_opts(SecondsFormat::Millis, true)),
        ))
    }

    /// Send scheduled pings to the client
    #[instrument(parent = &self.span, skip_all)]
    fn handle_ping_interval(&mut self, ctx: &mut Context<Self>) {
        if Instant::now().duration_since(self.last_active) >= Duration::from_secs(120) {
            self.server_leave_reason = Some("Ping timeout: 120 seconds".to_string());
            ctx.stop();
        }

        self.writer.write(Message {
            tags: None,
            prefix: None,
            command: Command::PING(SERVER_NAME.to_string(), None),
        });
    }

    //// Join the user to all the channels they were previously in before disconnecting from
    //// the server
    fn rejoin_channels(&self) -> impl ActorFuture<Self, Output = ()> + 'static {
        self.persistence
            .send(FetchUserChannels {
                user_id: self.connection.user_id,
                span: Span::current(),
            })
            .into_actor(self)
            .map(move |res, this, ctx| {
                ctx.notify(JoinChannelRequest {
                    channels: res.unwrap(),
                    span: this.span.clone(),
                });
            })
    }

    fn build_unseen_message(
        &self,
        sent: DateTime<Utc>,
        sender: &str,
        message: String,
        kind: MessageKind,
    ) -> Message {
        Message {
            tags: TagBuilder::default()
                .insert(self.maybe_build_time_tag(sent))
                .into(),
            prefix: Some(Prefix::new_from_str(sender)),
            command: match kind {
                MessageKind::Normal => Command::PRIVMSG(self.connection.nick.clone(), message),
                MessageKind::Notice => Command::NOTICE(self.connection.nick.clone(), message),
            },
        }
    }

    fn send_unseen_private_messages(&self) -> impl ActorFuture<Self, Output = ()> + 'static {
        self.persistence
            .send(FetchUnseenPrivateMessages {
                user_id: self.connection.user_id,
                span: Span::current(),
            })
            .into_actor(self)
            .map(move |res, this, ctx| {
                for (sent, sender, message, kind) in res.unwrap() {
                    ctx.notify(Broadcast {
                        message: this.build_unseen_message(sent, &sender, message, kind),
                        span: this.span.clone(),
                    });
                }
            })
    }

    fn channel_send_map_write<M>(
        &self,
        ctx: &mut Context<Self>,
        channel: &Addr<Channel>,
        message: M,
        map: impl FnOnce(M::Result, &Self) -> Vec<Message> + 'static,
    ) where
        M: actix::Message + Send + 'static,
        M::Result: Send,
        Channel: Handler<M>,
        <Channel as Actor>::Context: ToEnvelope<Channel, M>,
    {
        let fut = channel
            .send(message)
            .into_actor(self)
            .map(move |result, ref mut this, _ctx| {
                for message in (map)(result.unwrap(), this) {
                    this.writer.write(message);
                }
            });
        ctx.spawn(fut);
    }

    fn server_send_map_write<M>(
        &self,
        ctx: &mut Context<Self>,
        message: M,
        map: impl FnOnce(M::Result, &Self) -> Vec<Message> + 'static,
    ) where
        M: actix::Message + Send + 'static,
        M::Result: Send,
        Server: Handler<M>,
        <Server as Actor>::Context: ToEnvelope<Server, M>,
    {
        let fut =
            self.server
                .send(message)
                .into_actor(self)
                .map(move |result, ref mut this, _ctx| {
                    for message in (map)(result.unwrap(), this) {
                        this.writer.write(message);
                    }
                });
        ctx.spawn(fut);
    }
}

impl Actor for Client {
    type Context = Context<Self>;

    /// Called when the actor is first started (ie. when the client connects).
    ///
    /// We currently just use this to schedule pings towards the client.
    #[instrument(parent = &self.span, skip_all)]
    fn started(&mut self, ctx: &mut Self::Context) {
        info!(?self.connection, "Client has successfully joined to server");

        ctx.run_interval(Duration::from_secs(30), Self::handle_ping_interval);
        ctx.spawn(self.rejoin_channels());
        ctx.spawn(self.send_unseen_private_messages());
    }

    /// Called when the actor is shutting down, either gracefully by the client or forcefully
    /// by the server.
    #[instrument(parent = &self.span, skip_all)]
    fn stopped(&mut self, ctx: &mut Self::Context) {
        let message = self.server_leave_reason.take();

        // inform the server that the user is leaving the server
        self.server.do_send(ServerDisconnect {
            client: ctx.address(),
            message: if self.graceful_shutdown {
                Some(format!("Quit: {}", message.as_deref().unwrap_or("")))
            } else {
                message.clone()
            },
            span: Span::current(),
        });

        // inform all channels the client is connected to of them leaving the server
        for channel in self.channels.values() {
            channel.do_send(ServerDisconnect {
                client: ctx.address(),
                message: message.clone(),
                span: Span::current(),
            });
        }

        // acknowledge the client's quit message by sending an ERROR
        self.writer.write(Message {
            tags: None,
            prefix: None,
            command: Command::ERROR(if self.graceful_shutdown {
                String::new()
            } else {
                format!(
                    "Closing Link: {}",
                    message.as_deref().unwrap_or("Ungraceful shutdown")
                )
            }),
        });
    }
}

/// TODO: Despite the name of the message, writes a unicast message to the client.
impl Handler<Broadcast> for Client {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: Broadcast, _ctx: &mut Self::Context) -> Self::Result {
        self.writer.write(msg.message);
    }
}

/// Retrieves all the channels the user is connected to.
impl Handler<ConnectedChannels> for Client {
    type Result = ResponseFuture<<ConnectedChannels as actix::Message>::Result>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ConnectedChannels, _ctx: &mut Self::Context) -> Self::Result {
        let span = Span::current();
        let user_id = self.connection.user_id;

        let fut = self.channels.iter().map(move |(channel_name, handle)| {
            let span = span.clone();
            let channel_name = channel_name.to_string();
            let handle = handle.clone();

            async move {
                let permission = handle
                    .send(FetchUserPermission {
                        span,
                        user: user_id,
                    })
                    .await
                    .unwrap();

                (permission, channel_name)
            }
        });

        Box::pin(future::join_all(fut))
    }
}

/// Retrieves the entire WHO list for the user.
impl Handler<FetchWhoList> for Client {
    type Result = ResponseFuture<<FetchWhoList as actix::Message>::Result>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: FetchWhoList, _ctx: &mut Self::Context) -> Self::Result {
        let user_id = self.connection.user_id;

        let futures = self
            .channels
            .values()
            .map(|v| {
                v.send(ChannelFetchWhoList {
                    span: msg.span.clone(),
                })
            })
            .collect::<FuturesUnordered<_>>();
        Box::pin(futures.fold(WhoList::default(), move |mut acc, item| {
            let mut item = item.unwrap();
            item.nick_list.retain(|(_, conn)| conn.user_id == user_id);
            acc.list.push(item);
            future::ready(acc)
        }))
    }
}

/// Returns the client's current nick/connection info.
impl Handler<FetchClientDetails> for Client {
    type Result = MessageResult<FetchClientDetails>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: FetchClientDetails, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(self.connection.clone())
    }
}

impl Handler<SetAway> for Client {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: SetAway, ctx: &mut Self::Context) -> Self::Result {
        self.connection.away = msg.msg.filter(|msg| !msg.is_empty());

        let broadcast = ClientAway {
            span: msg.span,
            handle: ctx.address(),
            message: self.connection.away.clone(),
        };

        self.server.do_send(broadcast.clone());
        for channel in self.channels.values() {
            channel.do_send(broadcast.clone());
        }

        let resp = if self.connection.away.is_some() {
            Command::Response(
                Response::RPL_NOWAWAY,
                vec![
                    self.connection.nick.to_string(),
                    "You have been marked as being away".to_string(),
                ],
            )
        } else {
            Command::Response(
                Response::RPL_UNAWAY,
                vec![
                    self.connection.nick.to_string(),
                    "You are no longer marked as being away".to_string(),
                ],
            )
        };

        self.writer.write(Message {
            tags: None,
            prefix: None,
            command: resp,
        });
    }
}

/// Disconnects the current user from the server as a result of the `KILL` command.
impl Handler<KillUser> for Client {
    type Result = ();

    #[instrument(parent = & msg.span, skip_all)]
    fn handle(&mut self, msg: KillUser, ctx: &mut Self::Context) -> Self::Result {
        self.server_leave_reason = Some(format!("Killed ({} ({}))", msg.killer, msg.comment));
        ctx.stop();
    }
}

/// A self-message from the Client's [`StreamHandler`] implementation when the user
/// sends a join command out.
///
/// This will block the user from performing any actions until they're connected to the
/// channel due to us awaiting on the join handles.
impl Handler<JoinChannelRequest> for Client {
    type Result = ResponseActFuture<Self, ()>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: JoinChannelRequest, ctx: &mut Self::Context) -> Self::Result {
        let mut futures = Vec::with_capacity(msg.channels.len());

        // loop over all the channels and send a channel join notification to the root
        // server actor to get a handle back
        for channel_name in msg.channels {
            if !channel_name.is_channel_name() || self.channels.contains_key(&channel_name) {
                // todo: send message to client informing them of the invalid channel name
                continue;
            }

            let channel_handle_fut = self.server.clone().send(ChannelJoin {
                channel_name: channel_name.to_string(),
                client: ctx.address(),
                connection: self.connection.clone(),
                span: Span::current(),
            });

            let channel_messages_fut = self.persistence.send(FetchUnseenChannelMessages {
                channel_name: channel_name.to_string(),
                user_id: self.connection.user_id,
                span: Span::current(),
            });

            futures.push(future::join(channel_handle_fut, channel_messages_fut).map(
                move |(handle, messages)| {
                    (channel_name, handle.unwrap().unwrap(), messages.unwrap())
                },
            ));
        }

        // await on all the `ChannelJoin` events to the server, and once we get the channel
        // handles back write them to the server
        let fut = wrap_future::<_, Self>(
            future::join_all(futures.into_iter()).instrument(Span::current()),
        )
        .map(|result, this, _ctx| {
            for (channel_name, handle, messages) in result {
                let handle = match handle {
                    Ok(v) => v,
                    Err(error) => {
                        error!(?error, "User failed to join channel");
                        this.writer.write(error.into_message());
                        continue;
                    }
                };

                this.channels.insert(channel_name.clone(), handle);

                for (sent, source, message, kind) in messages {
                    this.writer.write(Message {
                        tags: TagBuilder::default()
                            .insert(this.maybe_build_time_tag(sent))
                            .into(),
                        prefix: Some(Prefix::new_from_str(&source)),
                        command: match kind {
                            MessageKind::Normal => Command::PRIVMSG(channel_name.clone(), message),
                            MessageKind::Notice => Command::NOTICE(channel_name.clone(), message),
                        },
                    });
                }
            }
        });

        Box::pin(fut)
    }
}

/// A self-message from the Client's [`StreamHandler`] implementation when the user
/// sends a request for each channel's member list.
impl Handler<ListChannelMemberRequest> for Client {
    type Result = ResponseActFuture<Self, ()>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ListChannelMemberRequest, _ctx: &mut Self::Context) -> Self::Result {
        let mut futures = Vec::with_capacity(msg.channels.len());

        // loop over all channels the user is connected to and fetch their members
        for (channel_name, handle) in &self.channels {
            if !msg.channels.contains(channel_name) {
                continue;
            }

            futures.push(handle.send(ChannelMemberList {
                span: Span::current(),
            }));
        }

        // await on all the `ChannelMemberList` events to the channels, and once we get the lists back
        // write them to the client
        let fut = wrap_future::<_, Self>(
            future::join_all(futures.into_iter()).instrument(Span::current()),
        )
        .map(|result, this, _ctx| {
            for list in result {
                let list = list.unwrap();

                for message in list.into_messages(
                    this.connection.nick.clone(),
                    this.connection
                        .capabilities
                        .contains(Capability::USERHOST_IN_NAMES),
                ) {
                    this.writer.write(message);
                }
            }
        });

        Box::pin(fut)
    }
}

impl Handler<UserNickChangeInternal> for Client {
    type Result = ResponseActFuture<Self, ()>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: UserNickChangeInternal, _ctx: &mut Self::Context) -> Self::Result {
        self.persistence
            .send(ReserveNick {
                user_id: self.connection.user_id,
                nick: msg.new_nick.clone(),
            })
            .into_actor(self)
            .map(|res, this, ctx| {
                if !res.unwrap() {
                    ctx.notify(Broadcast {
                        message: NickNotOwnedByUser(msg.new_nick).into_message(),
                        span: Span::current(),
                    });
                    return;
                }

                // alert the server to the nick change (we'll receive this event back so the user
                // gets the notification too)
                this.server.do_send(UserNickChange {
                    client: ctx.address(),
                    connection: this.connection.clone(),
                    new_nick: msg.new_nick.clone(),
                    span: Span::current(),
                });

                for channel in this.channels.values() {
                    channel.do_send(UserNickChange {
                        client: ctx.address(),
                        connection: this.connection.clone(),
                        new_nick: msg.new_nick.clone(),
                        span: Span::current(),
                    });
                }

                // updates our nick locally
                this.connection.nick = msg.new_nick;
            })
            .boxed_local()
    }
}

/// A message received from the root server to indicate that another known user has changed their
/// nick
impl Handler<UserNickChange> for Client {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: UserNickChange, _ctx: &mut Self::Context) -> Self::Result {
        self.writer.write(Message {
            tags: None,
            prefix: Some(msg.connection.to_nick()),
            command: Command::NICK(msg.new_nick),
        });
    }
}

/// Sent by channels when the current user is removed from it.
impl Handler<UserKickedFromChannel> for Client {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: UserKickedFromChannel, _ctx: &mut Self::Context) -> Self::Result {
        self.channels.remove(&msg.channel);
    }
}

/// Self-message to send a peer-to-peer message via the server.
impl Handler<SendPrivateMessage> for Client {
    type Result = ResponseActFuture<Self, ()>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: SendPrivateMessage, _ctx: &mut Self::Context) -> Self::Result {
        self.persistence
            .send(FetchUserIdByNick {
                nick: msg.destination,
            })
            .into_actor(self)
            .map(move |res, this, ctx| {
                let Some(destination) = res.unwrap() else {
                    // TODO
                    eprintln!("User attempted to send a message to non-existent user");
                    return;
                };

                this.server.do_send(PrivateMessage {
                    destination,
                    message: msg.message,
                    kind: msg.kind,
                    from: ctx.address(),
                    span: msg.span,
                });
            })
            .boxed_local()
    }
}

/// Receives messages from the user's incoming TCP stream and processes them, passing them onto
/// other actors or self-notifying and calling a [`Handler`].
impl StreamHandler<Result<irc_proto::Message, ProtocolError>> for Client {
    #[instrument(parent = &self.span, skip_all)]
    fn handle(&mut self, item: Result<irc_proto::Message, ProtocolError>, ctx: &mut Self::Context) {
        // unpack the message from the client
        let item = match item {
            Ok(item) => {
                debug!(?item, "Received message from client");
                item
            }
            Err(error) => {
                error!(%error, "Client sent a bad message");
                return;
            }
        };

        // ensure that the message from the client is either a global message (ie. a ping) or
        // has the correct nick (ie. it isn't spoofed or desynced)
        if item
            .source_nickname()
            .map_or(false, |v| v != self.connection.nick)
        {
            warn!("Rejecting message from client due to incorrect nick");
            return;
        }

        // https://modern.ircdocs.horse/
        #[allow(clippy::match_same_arms)]
        match item.command {
            Command::NICK(new_nick) => {
                ctx.notify(UserNickChangeInternal {
                    old_nick: self.connection.nick.to_string(),
                    new_nick,
                    span: Span::current(),
                });
            }
            Command::UserMODE(_, _) => {
                // TODO
            }
            Command::QUIT(message) => {
                // set the user's leave reason and request a shutdown of the actor to close the
                // connection
                self.graceful_shutdown = true;
                self.server_leave_reason = message;
                ctx.stop();
            }
            Command::JOIN(channel_names, _passwords, _real_name) => {
                // split the list of channel names...
                let channels = parse_channel_name_list(&channel_names);

                // ...and send a self-notification to schedule those joins
                ctx.notify(JoinChannelRequest {
                    channels,
                    span: Span::current(),
                });
            }
            Command::PART(channel, message) => {
                // remove the handle from the users locally connected channels
                let Some(channel) = self.channels.remove(&channel) else {
                    return;
                };

                // alert the channel to our leave
                channel.do_send(ChannelPart {
                    client: ctx.address(),
                    message,
                    span: Span::current(),
                });
            }
            Command::ChannelMODE(channel, modes) => {
                let Some(channel) = self.channels.get(&channel) else {
                    return;
                };

                channel.do_send(ChannelSetMode {
                    span: Span::current(),
                    client: ctx.address(),
                    modes,
                });
            }
            Command::TOPIC(channel, topic) => {
                let Some(channel) = self.channels.get(&channel) else {
                    return;
                };

                #[allow(clippy::option_if_let_else)]
                if let Some(topic) = topic {
                    channel.do_send(ChannelUpdateTopic {
                        topic,
                        client: ctx.address(),
                        span: Span::current(),
                    });
                } else {
                    let span = Span::current();
                    self.channel_send_map_write(
                        ctx,
                        channel,
                        ChannelFetchTopic { span },
                        |res, this| res.into_messages(this.connection.nick.to_string(), false),
                    );
                }
            }
            Command::NAMES(channel_names, _) => {
                // split the list of channel names...
                let channels = parse_channel_name_list(channel_names.as_deref().unwrap_or(""));

                if channels.is_empty() {
                    warn!("Client didn't request names for a particular channel");
                    return;
                }

                // ...and send a self-notification to request each channel for their list
                ctx.notify(ListChannelMemberRequest {
                    channels,
                    span: Span::current(),
                });
            }
            Command::LIST(_, _) => {
                let span = Span::current();
                self.server_send_map_write(ctx, ChannelList { span }, |res, this| {
                    res.into_messages(this.connection.nick.to_string())
                });
            }
            Command::INVITE(nick, channel) => {
                let Some(channel) = self.channels.get(&channel) else {
                    error!(%channel, "User not connected to channel");
                    return;
                };

                channel.do_send(ChannelInvite {
                    nick,
                    client: ctx.address(),
                    span: Span::current(),
                });
            }
            Command::KICK(channel, users, reason) => {
                let Some(channel) = self.channels.get(&channel) else {
                    error!(%channel, "User not connected to channel");
                    return;
                };

                for user in parse_channel_name_list(&users) {
                    channel.do_send(ChannelKickUser {
                        span: Span::current(),
                        client: ctx.address(),
                        user,
                        reason: reason.clone(),
                    });
                }
            }
            command @ (Command::NOTICE(_, _) | Command::PRIVMSG(_, _)) => {
                let (target, message, kind) = match command {
                    Command::PRIVMSG(target, message) => (target, message, MessageKind::Normal),
                    Command::NOTICE(target, message) => (target, message, MessageKind::Notice),
                    _ => unreachable!(),
                };

                if !target.is_channel_name() {
                    // private message to another user
                    ctx.notify(SendPrivateMessage {
                        destination: target,
                        message,
                        kind,
                        span: Span::current(),
                    });
                } else if let Some(channel) = self.channels.get(&target) {
                    channel.do_send(ChannelMessage {
                        client: ctx.address(),
                        message,
                        kind,
                        span: Span::current(),
                    });
                } else {
                    // user not connected to channel
                    error!("User not connected to channel");
                }
            }
            Command::MOTD(_) => {
                let span = Span::current();
                self.server_send_map_write(ctx, ServerFetchMotd { span }, |res, this| {
                    res.into_messages(this.connection.nick.to_string())
                });
            }
            Command::LUSERS(_, _) => {
                let span = Span::current();
                self.server_send_map_write(ctx, ServerListUsers { span }, |res, this| {
                    res.into_messages(&this.connection.nick)
                });
            }
            Command::VERSION(_) => {
                self.writer.write(Message {
                    tags: None,
                    prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                    command: Command::Response(
                        Response::RPL_VERSION,
                        vec![
                            self.connection.nick.to_string(),
                            format!("{}-{}", crate_name!(), crate_version!()),
                            SERVER_NAME.to_string(),
                        ],
                    ),
                });
            }
            Command::TIME(_) => {
                let time = chrono::Utc::now();

                self.writer.write(Message {
                    tags: None,
                    prefix: None,
                    command: Command::Response(
                        Response::RPL_TIME,
                        vec![
                            self.connection.nick.to_string(),
                            SERVER_NAME.to_string(),
                            time.timestamp().to_string(),
                            time.format("%a %b %d %Y %T").to_string(),
                        ],
                    ),
                });
            }
            Command::ADMIN(_) => {
                let span = Span::current();
                self.server_send_map_write(ctx, ServerAdminInfo { span }, |res, this| {
                    res.into_messages(&this.connection.nick)
                });
            }
            Command::INFO(_) => {
                static INFO_STR: &str = include_str!("../text/info.txt");
                for line in INFO_STR.trim().split('\n') {
                    self.writer.write(Message {
                        tags: None,
                        prefix: None,
                        command: Command::Response(
                            Response::RPL_INFO,
                            vec![self.connection.nick.to_string(), line.to_string()],
                        ),
                    });
                }

                self.writer.write(Message {
                    tags: None,
                    prefix: None,
                    command: Command::Response(
                        Response::RPL_ENDOFINFO,
                        vec![
                            self.connection.nick.to_string(),
                            "End of INFO list".to_string(),
                        ],
                    ),
                });
            }
            Command::WHO(Some(query), _) => {
                let span = Span::current();
                self.server_send_map_write(ctx, FetchWhoList { span, query }, |res, this| {
                    res.into_messages(&this.connection.nick)
                });
            }
            Command::WHOIS(Some(query), _) => {
                let span = Span::current();
                self.server_send_map_write(ctx, FetchWhois { span, query }, |res, this| {
                    res.into_messages(&this.connection.nick)
                });
            }
            Command::WHOWAS(_, _, _) => {}
            Command::KILL(nick, comment) => {
                self.server.do_send(KillUser {
                    span: Span::current(),
                    killer: self.connection.nick.to_string(),
                    comment,
                    killed: nick,
                });
            }
            Command::PING(v, _) => {
                self.writer.write(Message {
                    tags: None,
                    prefix: None,
                    command: Command::PONG(v, None),
                });
            }
            Command::PONG(_, _) => {
                self.last_active = Instant::now();
            }
            Command::AWAY(msg) => {
                ctx.notify(SetAway {
                    span: Span::current(),
                    msg,
                });
            }
            Command::REHASH => {}
            Command::DIE => {}
            Command::RESTART => {}
            Command::USERS(_) => {}
            Command::WALLOPS(message) if self.connection.mode.contains(UserMode::OPER) => {
                self.server.do_send(Wallops {
                    span: Span::current(),
                    message,
                });
            }
            Command::USERHOST(_) => {}
            Command::SAJOIN(_, _) => {}
            Command::SAMODE(_, _, _) => {}
            Command::SANICK(old_nick, new_nick) => {
                // TODO: permission checks
                self.server.do_send(UserNickChangeInternal {
                    old_nick,
                    new_nick,
                    span: Span::current(),
                });
            }
            Command::SAPART(_, _) => {}
            Command::SAQUIT(_, _) => {}
            Command::AUTHENTICATE(_) => {
                self.writer.write(
                    SaslAlreadyAuthenticated(self.connection.nick.to_string()).into_message(),
                );
            }
            Command::ACCOUNT(_) => {}
            Command::METADATA(_, _, _) => {}
            Command::MONITOR(_, _) => {}
            Command::BATCH(_, _, _) => {}
            Command::CHGHOST(_, _) => {}
            Command::Response(_, _) => {}
            v => self.writer.write(Message {
                tags: None,
                prefix: Some(Prefix::new_from_str(&self.connection.nick)),
                command: Command::Response(
                    Response::ERR_UNKNOWNCOMMAND,
                    vec![
                        String::from(&v)
                            .split_whitespace()
                            .next()
                            .unwrap_or_default()
                            .to_string(),
                        "Unknown command".to_string(),
                    ],
                ),
            }),
        }
    }
}

#[derive(Default)]
pub struct TagBuilder {
    inner: Vec<Tag>,
}

impl TagBuilder {
    #[must_use]
    pub fn insert(mut self, tag: impl Into<Option<Tag>>) -> Self {
        if let Some(tag) = tag.into() {
            self.inner.push(tag);
        }

        self
    }
}

impl From<TagBuilder> for Option<Vec<Tag>> {
    fn from(value: TagBuilder) -> Self {
        Some(value.inner).filter(|v| !v.is_empty())
    }
}

#[must_use]
pub fn parse_channel_name_list(s: &str) -> Vec<String> {
    s.split(',')
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// Sent to us by actix whenever we fail to write a message to the client's outgoing tcp stream
impl WriteHandler<ProtocolError> for Client {
    #[instrument(parent = &self.span, skip_all)]
    fn error(&mut self, error: ProtocolError, _ctx: &mut Self::Context) -> Running {
        error!(%error, "Failed to write message to client");
        Running::Continue
    }
}

/// A [`Client`] internal self-notification to send a peer-to-peer message to another user
#[derive(actix::Message, Debug)]
#[rtype(result = "()")]
struct SendPrivateMessage {
    destination: String,
    message: String,
    kind: MessageKind,
    span: Span,
}

/// A [`Client`] internal self-notification to grab a list of users in each channel
#[derive(actix::Message, Debug)]
#[rtype(result = "()")]
struct ListChannelMemberRequest {
    channels: Vec<String>,
    span: Span,
}

/// A [`Client`] internal self-notification to schedule channel joining
#[derive(actix::Message, Debug)]
#[rtype(result = "()")]
struct JoinChannelRequest {
    channels: Vec<String>,
    span: Span,
}

/// A [`Client`] internal self-notification to set away status
#[derive(actix::Message, Debug)]
#[rtype(result = "()")]
struct SetAway {
    msg: Option<String>,
    span: Span,
}
