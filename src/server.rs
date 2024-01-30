pub mod response;

use std::{borrow::Cow, collections::HashMap};

use actix::{
    Actor, Addr, AsyncContext, Context, Handler, MessageResult, ResponseFuture, Supervised,
    Supervisor,
};
use actix_rt::Arbiter;
use clap::crate_version;
use futures::{
    future,
    stream::{FuturesOrdered, FuturesUnordered},
    TryFutureExt,
};
use irc_proto::{Command, Message, Prefix, Response};
use rand::seq::SliceRandom;
use tokio_stream::StreamExt;
use tracing::{debug, instrument, warn, Span};

use crate::{
    channel::{permissions::Permission, Channel, ChannelId},
    client::Client,
    config::Config,
    connection::{InitiatedConnection, UserMode},
    messages::{
        Broadcast, ChannelFetchTopic, ChannelFetchWhoList, ChannelJoin, ChannelList,
        ChannelMemberList, ClientAway, ConnectedChannels, FetchClientByNick, FetchWhoList,
        FetchWhois, KillUser, MessageKind, PrivateMessage, ServerAdminInfo, ServerDisconnect,
        ServerFetchMotd, ServerListUsers, UserConnected, UserNickChange, UserNickChangeInternal,
        Wallops,
    },
    persistence::Persistence,
    server::response::{AdminInfo, ListUsers, Motd, WhoList, Whois},
    SERVER_NAME,
};

/// The root actor for arbitration between clients and channels.
pub struct Server {
    pub channel_arbiters: Vec<Arbiter>,
    pub channels: HashMap<String, Addr<Channel>>,
    pub clients: HashMap<Addr<Client>, InitiatedConnection>,
    pub max_clients: usize,
    pub config: Config,
    pub persistence: Addr<Persistence>,
}

impl Supervised for Server {}

/// Received when an admin SANICKs another user.
impl Handler<UserNickChangeInternal> for Server {
    type Result = ();

    fn handle(&mut self, msg: UserNickChangeInternal, _ctx: &mut Self::Context) -> Self::Result {
        let client = self.clients.iter().find(|(_k, v)| v.nick == msg.old_nick);
        let Some((client, _)) = client else {
            warn!(%msg.old_nick, %msg.new_nick, "User attempted to update nick for unknown user");
            return;
        };

        debug!(%msg.old_nick, %msg.new_nick, "User is updating nick for another user");

        client.do_send(msg);
    }
}

/// Received when a user connects to the server, and sends them the server preamble
impl Handler<UserConnected> for Server {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: UserConnected, _ctx: &mut Self::Context) -> Self::Result {
        let nick = msg.connection.to_nick();

        // send a welcome to the user
        let responses = [
            (
                Response::RPL_WELCOME,
                vec![Cow::Owned(format!("Welcome to the network {nick}",))],
            ),
            (
                Response::RPL_YOURHOST,
                vec![format!(
                    "Your host is {SERVER_NAME}, running version {}",
                    crate_version!()
                )
                .into()],
            ),
            (
                Response::RPL_CREATED,
                vec!["This server was created at some point".into()],
            ),
            (
                Response::RPL_MYINFO,
                vec![
                    SERVER_NAME.into(),
                    crate_version!().into(),
                    "DOQRSZaghilopsuwz".into(),
                    "CFILMPQSbcefgijklmnopqrstuvz".into(),
                    "bkloveqjfI".into(),
                ],
            ),
            (
                Response::RPL_ISUPPORT,
                vec![
                    format!("PREFIX={}", Permission::SUPPORTED_PREFIXES).into(),
                    "are supported by this server".into(),
                ],
            ),
        ];

        for (response, arguments) in responses {
            let arguments = std::iter::once(msg.connection.nick.clone())
                .chain(arguments.into_iter().map(Cow::into_owned))
                .collect();

            msg.handle.do_send(Broadcast {
                span: Span::current(),
                message: Message {
                    tags: None,
                    prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                    command: Command::Response(response, arguments),
                },
            });
        }

        for message in Motd::new(self).into_messages(msg.connection.nick.clone()) {
            msg.handle.do_send(Broadcast {
                span: Span::current(),
                message,
            });
        }

        self.clients.insert(msg.handle, msg.connection);
        self.max_clients = self.clients.len().max(self.max_clients);
    }
}

impl Handler<Wallops> for Server {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: Wallops, _ctx: &mut Self::Context) -> Self::Result {
        for (handle, conn) in &self.clients {
            if !conn.mode.contains(UserMode::WALLOPS) {
                continue;
            }

            handle.do_send(Broadcast {
                message: Message {
                    tags: None,
                    prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                    command: Command::WALLOPS(msg.message.clone()),
                },
                span: msg.span.clone(),
            });
        }
    }
}

/// Returns the MOTD when requested.
impl Handler<ServerFetchMotd> for Server {
    type Result = MessageResult<ServerFetchMotd>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ServerFetchMotd, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(Motd::new(self))
    }
}

/// Received when a client disconnects from the server
impl Handler<ServerDisconnect> for Server {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ServerDisconnect, _ctx: &mut Self::Context) -> Self::Result {
        self.clients.remove(&msg.client);
    }
}

/// Received when a client is attempting to join a channel, and forwards it onto the requested
/// channel for it to handle -- creating it if it doesn't already exist.
impl Handler<ChannelJoin> for Server {
    type Result = ResponseFuture<<ChannelJoin as actix::Message>::Result>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelJoin, ctx: &mut Self::Context) -> Self::Result {
        let channel = self
            .channels
            .entry(msg.channel_name.clone())
            .or_insert_with(|| {
                let arbiter = self
                    .channel_arbiters
                    .choose(&mut rand::thread_rng())
                    .map_or_else(Arbiter::current, Arbiter::handle);

                let channel_name = msg.channel_name.clone();
                let server = ctx.address();
                let persistence = self.persistence.clone();

                Supervisor::start_in_arbiter(&arbiter, move |_ctx| Channel {
                    name: channel_name,
                    permissions: HashMap::new(),
                    clients: HashMap::new(),
                    topic: None,
                    server,
                    persistence,
                    channel_id: ChannelId(0),
                })
            })
            .clone();

        Box::pin(
            channel
                .send(msg)
                .map_err(anyhow::Error::new)
                .and_then(futures::future::ready),
        )
    }
}

/// Received when a client changes their nick and forwards it on to all other users connected to
/// the server.
impl Handler<UserNickChange> for Server {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: UserNickChange, _ctx: &mut Self::Context) -> Self::Result {
        // inform all clients of the nick change
        for client in self.clients.keys() {
            client.do_send(msg.clone());
        }

        if let Some(client) = self.clients.get_mut(&msg.client) {
            *client = msg.connection;
            client.nick = msg.new_nick;
        }
    }
}

/// Looks up a user to disconnect and sends the disconnect notification.
impl Handler<KillUser> for Server {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: KillUser, _ctx: &mut Self::Context) -> Self::Result {
        for (handle, user) in &self.clients {
            if user.nick == msg.killed {
                handle.do_send(msg.clone());
            }
        }
    }
}

impl Handler<ClientAway> for Server {
    type Result = ();

    fn handle(&mut self, msg: ClientAway, _ctx: &mut Self::Context) -> Self::Result {
        if let Some(c) = self.clients.get_mut(&msg.handle) {
            c.away = msg.message;
        }
    }
}

/// Fetches a client's handle by their nick
impl Handler<FetchClientByNick> for Server {
    type Result = MessageResult<FetchClientByNick>;

    fn handle(&mut self, msg: FetchClientByNick, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(
            // TODO: need O(1) lookup here
            self.clients
                .iter()
                .find(|(_handle, connection)| connection.nick == msg.nick)
                .map(|v| v.0.clone()),
        )
    }
}

impl Handler<FetchWhois> for Server {
    type Result = ResponseFuture<<FetchWhois as actix::Message>::Result>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: FetchWhois, _ctx: &mut Self::Context) -> Self::Result {
        let Some((handle, conn)) = self.clients.iter().find(|(_, conn)| conn.nick == msg.query)
        else {
            return Box::pin(future::ready(Whois {
                query: msg.query,
                conn: None,
                channels: vec![],
            }));
        };

        let conn = conn.clone();
        let channels = handle.send(ConnectedChannels {
            span: Span::current(),
        });

        Box::pin(async move {
            Whois {
                query: msg.query,
                conn: Some(conn),
                channels: channels.await.unwrap(),
            }
        })
    }
}

impl Handler<FetchWhoList> for Server {
    type Result = ResponseFuture<<FetchWhoList as actix::Message>::Result>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: FetchWhoList, _ctx: &mut Self::Context) -> Self::Result {
        if let Some(channel) = self.channels.get(&msg.query).cloned() {
            Box::pin(async move {
                WhoList {
                    list: vec![channel
                        .send(ChannelFetchWhoList { span: msg.span })
                        .await
                        .unwrap()],
                    query: msg.query,
                }
            })
        } else {
            let futures = self
                .clients
                .iter()
                .filter(|(_, conn)| conn.nick == msg.query)
                .map(|(client, _)| {
                    client.send(FetchWhoList {
                        span: msg.span.clone(),
                        query: String::new(),
                    })
                })
                .collect::<FuturesUnordered<_>>();

            let init = WhoList {
                query: msg.query,
                list: Vec::new(),
            };
            Box::pin(futures.fold(init, |mut acc, item| {
                acc.list.extend(item.unwrap().list);
                acc
            }))
        }
    }
}

impl Handler<ChannelList> for Server {
    type Result = ResponseFuture<<ChannelList as actix::Message>::Result>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelList, _ctx: &mut Self::Context) -> Self::Result {
        let fut = self
            .channels
            .values()
            .map(|channel| {
                let fetch_topic = channel.send(ChannelFetchTopic {
                    span: Span::current(),
                });

                let fetch_members = channel.send(ChannelMemberList {
                    span: Span::current(),
                });

                futures::future::try_join(fetch_topic, fetch_members)
            })
            .collect::<FuturesOrdered<_>>()
            .map(|res| {
                let (topic, members) = res.unwrap();

                response::ChannelListItem {
                    channel_name: topic.channel_name,
                    client_count: members.nick_list.len(),
                    topic: topic.topic.map(|v| v.topic),
                }
            })
            .fold(response::ChannelList::default(), |mut acc, v| {
                acc.members.push(v);
                acc
            });

        Box::pin(fut)
    }
}

impl Handler<ServerListUsers> for Server {
    type Result = MessageResult<ServerListUsers>;

    fn handle(&mut self, _msg: ServerListUsers, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(ListUsers {
            current_clients: self.clients.len(),
            max_clients: self.max_clients,
            operators_online: 0,
            channels_formed: self.channels.len(),
        })
    }
}

impl Handler<ServerAdminInfo> for Server {
    type Result = MessageResult<ServerAdminInfo>;

    fn handle(&mut self, _msg: ServerAdminInfo, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(AdminInfo {
            line1: "Name: example name".to_string(),
            line2: "Nickname: examplenick".to_string(),
            email: "Email: me@example.com".to_string(),
        })
    }
}

impl Handler<PrivateMessage> for Server {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: PrivateMessage, _ctx: &mut Self::Context) -> Self::Result {
        let Some(source) = self.clients.get(&msg.from) else {
            // user is not yet registered with the server
            return;
        };

        let mut seen_by_user = false;

        // TODO: O(1) lookup of users by id
        for (target, target_conn) in self.clients.iter().filter(|(handle, connection)| {
            connection.user_id == msg.destination && msg.from != **handle
        }) {
            target.do_send(Broadcast {
                message: Message {
                    tags: None,
                    prefix: Some(source.to_nick()),
                    command: match msg.kind {
                        MessageKind::Normal => {
                            Command::PRIVMSG(target_conn.nick.clone(), msg.message.clone())
                        }
                        MessageKind::Notice => {
                            Command::NOTICE(target_conn.nick.clone(), msg.message.clone())
                        }
                    },
                },
                span: msg.span.clone(),
            });

            seen_by_user = true;
        }

        if !seen_by_user {
            self.persistence
                .do_send(crate::persistence::events::PrivateMessage {
                    sender: source.to_nick().to_string(),
                    receiver: msg.destination,
                    message: msg.message,
                    kind: msg.kind,
                });
        }
    }
}

impl Actor for Server {
    type Context = Context<Self>;
}
