pub mod response;

use std::collections::{HashMap, HashSet};

use actix::{Actor, Addr, Context, Handler, ResponseFuture};
use futures::{stream::FuturesOrdered, TryFutureExt};
use irc_proto::{Command, Message, Prefix, Response};
use tokio_stream::StreamExt;
use tracing::{instrument, Span};

use crate::{
    channel::Channel,
    client::Client,
    messages::{
        Broadcast, ChannelFetchTopic, ChannelJoin, ChannelList, ChannelMemberList,
        ServerDisconnect, UserConnected, UserNickChange,
    },
    SERVER_NAME,
};

/// The root actor for arbitration between clients and channels.
#[derive(Default)]
pub struct Server {
    channels: HashMap<String, Addr<Channel>>,
    clients: HashSet<Addr<Client>>,
}

/// Received when a user connects to the server, and sends them the server preamble
impl Handler<UserConnected> for Server {
    type Result = ();

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: UserConnected, _ctx: &mut Self::Context) -> Self::Result {
        // send a welcome to the user
        let responses = [
            (
                Response::RPL_WELCOME,
                vec!["Welcome to the network jordan!jordan@proper.sick.kid"],
            ),
            (Response::RPL_YOURHOST, vec!["Your host is a sick kid"]),
            (
                Response::RPL_CREATED,
                vec!["This server was created at some point"],
            ),
            (
                Response::RPL_MYINFO,
                vec![
                    SERVER_NAME,
                    "0.0.1",
                    "DOQRSZaghilopsuwz",
                    "CFILMPQSbcefgijklmnopqrstuvz",
                    "bkloveqjfI",
                ],
            ),
            (
                Response::RPL_ISUPPORT,
                vec!["D", "are supported by this server"],
            ),
        ];

        for (response, arguments) in responses {
            // fixme: bad perf here with inserting at the front of a vec
            let arguments = std::iter::once(msg.connection.nick.clone())
                .chain(arguments.into_iter().map(ToString::to_string))
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

        self.clients.insert(msg.handle);
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
    fn handle(&mut self, msg: ChannelJoin, _ctx: &mut Self::Context) -> Self::Result {
        let channel = self
            .channels
            .entry(msg.channel_name.clone())
            .or_insert_with(|| {
                Channel {
                    name: msg.channel_name.clone(),
                    clients: HashMap::new(),
                    topic: None,
                }
                .start()
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
        for client in &self.clients {
            client.do_send(msg.clone());
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

impl Actor for Server {
    type Context = Context<Self>;
}
