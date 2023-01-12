use std::{collections::HashMap, time::Duration};

use actix::{
    fut::wrap_future, io::WriteHandler, Actor, ActorContext, ActorFutureExt, Addr, AsyncContext,
    Context, Handler, MessageResult, ResponseActFuture, Running, StreamHandler, WrapFuture,
};
use clap::{crate_name, crate_version};
use futures::FutureExt;
use irc_proto::{error::ProtocolError, ChannelExt, Command, Message, Prefix, Response};
use tokio::time::Instant;
use tracing::{debug, error, info_span, instrument, warn, Instrument, Span};

use crate::{
    channel::Channel,
    connection::{sasl::SaslAlreadyAuthenticated, InitiatedConnection, MessageSink},
    messages::{
        Broadcast, ChannelFetchTopic, ChannelInvite, ChannelJoin, ChannelKickUser, ChannelList,
        ChannelMemberList, ChannelMessage, ChannelPart, ChannelSetMode, ChannelUpdateTopic,
        FetchClientDetails, PeerToPeerMessage, ServerDisconnect, ServerFetchMotd,
        UserKickedFromChannel, UserNickChange, UserNickChangeInternal,
    },
    persistence::{
        events::{FetchUnseenMessages, FetchUserChannels, ReserveNick},
        Persistence,
    },
    server::Server,
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

impl Actor for Client {
    type Context = Context<Self>;

    /// Called when the actor is first started (ie. when the client connects).
    ///
    /// We currently just use this to schedule pings towards the client.
    #[instrument(parent = &self.span, skip_all)]
    fn started(&mut self, ctx: &mut Self::Context) {
        // schedule pings to the client
        ctx.run_interval(Duration::from_secs(30), |this, ctx| {
            let _span = info_span!(parent: &this.span, "ping").entered();

            if Instant::now().duration_since(this.last_active) >= Duration::from_secs(120) {
                this.server_leave_reason = Some("Ping timeout: 120 seconds".to_string());
                ctx.stop();
            }

            this.writer.write(Message {
                tags: None,
                prefix: None,
                command: Command::PING(SERVER_NAME.to_string(), None),
            });
        });

        // join the user to all the channels they were previously in before disconnecting from
        // the server
        ctx.spawn(
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
                }),
        );
    }

    /// Called when the actor is shutting down, either gracefully by the client or forcefully
    /// by the server.
    #[instrument(parent = &self.span, skip_all)]
    fn stopped(&mut self, ctx: &mut Self::Context) {
        let message = self.server_leave_reason.take();

        // inform the server that the user is leaving the server
        self.server.do_send(ServerDisconnect {
            client: ctx.address(),
            message: message.clone(),
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

        // send the shutdown message to the client before we terminate the connection on
        // return of this function
        if self.graceful_shutdown {
            self.writer.write(Message {
                tags: None,
                prefix: Some(self.connection.to_nick()),
                command: Command::QUIT(message),
            });
        } else {
            let message = message.unwrap_or_else(|| "Ungraceful shutdown".to_string());

            self.writer.write(Message {
                tags: None,
                prefix: None,
                command: Command::ERROR(message),
            });
        }
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

/// Returns the client's current nick/connection info.
impl Handler<FetchClientDetails> for Client {
    type Result = MessageResult<FetchClientDetails>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: FetchClientDetails, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(self.connection.clone())
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

            let channel_messages_fut = self.persistence.send(FetchUnseenMessages {
                channel_name: channel_name.to_string(),
                user_id: self.connection.user_id,
                span: Span::current(),
            });

            futures.push(
                futures::future::join(channel_handle_fut, channel_messages_fut).map(
                    move |(handle, messages)| {
                        (channel_name, handle.unwrap().unwrap(), messages.unwrap())
                    },
                ),
            );
        }

        // await on all the `ChannelJoin` events to the server, and once we get the channel
        // handles back write them to the server
        let fut = wrap_future::<_, Self>(
            futures::future::join_all(futures.into_iter()).instrument(Span::current()),
        )
        .map(|result, this, _ctx| {
            for (channel_name, handle, messages) in result {
                let handle = match handle {
                    Ok(v) => v,
                    Err(error) => {
                        error!(?error, "Failed to join user to channel");
                        continue;
                    }
                };

                this.channels.insert(channel_name.clone(), handle);

                for (source, message) in messages {
                    this.writer.write(Message {
                        tags: None,
                        prefix: Some(Prefix::new_from_str(&source)),
                        command: Command::PRIVMSG(channel_name.clone(), message),
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
            futures::future::join_all(futures.into_iter()).instrument(Span::current()),
        )
        .map(|result, this, _ctx| {
            for list in result {
                let list = list.unwrap();

                for message in list.into_messages(this.connection.nick.clone()) {
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
                    // TODO: send notification to user to say the nick isn't available
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
            Command::USER(_, _, _) | Command::PASS(_) | Command::CAP(_, _, _, _) => {
                // these were already handled by `negotiate_client_connection`
            }
            Command::NICK(new_nick) => {
                ctx.notify(UserNickChangeInternal {
                    old_nick: self.connection.nick.to_string(),
                    new_nick,
                    span: Span::current(),
                });
            }
            Command::OPER(_, _) => {}
            Command::UserMODE(_, _) => {}
            Command::SERVICE(_, _, _, _, _, _) => {}
            Command::QUIT(message) => {
                // set the user's leave reason and request a shutdown of the actor to close the
                // connection
                self.graceful_shutdown = true;
                self.server_leave_reason = message;
                ctx.stop();
            }
            Command::SQUIT(_, _) => {}
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
                    let fut = channel
                        .send(ChannelFetchTopic { span })
                        .into_actor(self)
                        .map(|result, this, _ctx| {
                            for message in result
                                .unwrap()
                                .into_messages(this.connection.nick.to_string(), false)
                            {
                                this.writer.write(message);
                            }
                        });

                    ctx.spawn(fut);
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
                let fut = self.server.send(ChannelList { span }).into_actor(self).map(
                    |result, this, _ctx| {
                        for message in result
                            .unwrap()
                            .into_messages(this.connection.nick.to_string())
                        {
                            this.writer.write(message);
                        }
                    },
                );

                ctx.spawn(fut);
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
            Command::PRIVMSG(target, message) => {
                if !target.is_channel_name() {
                    // private message to another user
                    self.server.do_send(PeerToPeerMessage {
                        destination: target,
                        message,
                        from: ctx.address(),
                        span: Span::current(),
                    });
                } else if let Some(channel) = self.channels.get(&target) {
                    channel.do_send(ChannelMessage {
                        client: ctx.address(),
                        message,
                        span: Span::current(),
                    });
                } else {
                    // user not connected to channel
                    error!("User not connected to channel");
                }
            }
            Command::NOTICE(_, _) => {}
            Command::MOTD(_) => {
                let span = Span::current();
                let fut = self
                    .server
                    .send(ServerFetchMotd { span })
                    .into_actor(self)
                    .map(|result, this, _ctx| {
                        for message in result
                            .unwrap()
                            .into_messages(this.connection.nick.to_string())
                        {
                            this.writer.write(message);
                        }
                    });

                ctx.spawn(fut);
            }
            Command::LUSERS(_, _) => {}
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
            Command::STATS(_, _) => {}
            Command::LINKS(_, _) => {}
            Command::TIME(_) => {}
            Command::CONNECT(_, _, _) => {}
            Command::TRACE(_) => {}
            Command::ADMIN(_) => {}
            Command::INFO(_) => {}
            Command::SERVLIST(_, _) => {}
            Command::SQUERY(_, _) => {}
            Command::WHO(_, _) => {}
            Command::WHOIS(_, _) => {}
            Command::WHOWAS(_, _, _) => {}
            Command::KILL(_, _) => {}
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
            Command::ERROR(_) => {}
            Command::AWAY(_) => {}
            Command::REHASH => {}
            Command::DIE => {}
            Command::RESTART => {}
            Command::SUMMON(_, _, _) => {}
            Command::USERS(_) => {}
            Command::WALLOPS(_) => {}
            Command::USERHOST(_) => {}
            Command::ISON(_) => {}
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
            Command::NICKSERV(_) => {}
            Command::CHANSERV(_) => {}
            Command::OPERSERV(_) => {}
            Command::BOTSERV(_) => {}
            Command::HOSTSERV(_) => {}
            Command::MEMOSERV(_) => {}
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
            Command::Raw(_, _) => {}
        }
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
