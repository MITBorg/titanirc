use crate::entities::{channel::Channel, user::User};

use std::{collections::HashMap, net::SocketAddr};

use actix::{io::FramedWrite, prelude::*};
use titanirc_types::{protocol::primitives::Receiver, RegisteredNick, UserIdent};
use tokio::net::TcpStream;
use tokio_util::codec::FramedRead;

/// The core of our server:
///
/// - Handles incoming connections and spawns `User` actors for each.
/// - Handles channel creation, access control, operator config, etc.
///
/// Essentially acts as the middleman for each entity communicating with each other.
pub struct Server {
    /// A list of known channels and the addresses to them.
    pub channels: HashMap<String, Addr<Channel>>,
    // A list of known connected users.
    // pub users: Vec<(UserIdent, Addr<User>)>,    // todo: add this when we know how auth is gonna work
}

impl Server {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
            // users: Vec::new(),
        }
    }
}

impl Actor for Server {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Connection(pub TcpStream, pub SocketAddr);

/// Handles incoming connections from our connection acceptor loop and spawns
/// a `User` actor for each which handles reading from the socket and acting
/// accordingly.
impl Handler<Connection> for Server {
    type Result = ();

    fn handle(&mut self, Connection(stream, remote): Connection, server_ctx: &mut Self::Context) {
        println!("Accepted connection from {}", remote);

        User::create(move |ctx| {
            let nick = RegisteredNick::new();

            let (read, write) = tokio::io::split(stream);
            let read = FramedRead::new(read, titanirc_codec::Decoder);
            let write = FramedWrite::new(
                write,
                titanirc_codec::Encoder::new("my.cool.server", nick.clone()), // TODO: this should take a UserIdent
                ctx,
            );

            // Make our new `User` handle all events from this socket in `StreamHandler<Result<Command, _>>`.
            ctx.add_stream(read);

            // TODO: don't give the user a full server handle until they're authed
            //  ... only add the self.user to `user` then and only then.
            User::new(server_ctx.address(), write, nick)
        });
    }
}

/// Sent by `User` actors to arbitrate access to the requested channel.
impl Handler<crate::entities::channel::events::Join> for Server {
    type Result = ResponseActFuture<Self, crate::entities::channel::events::JoinResult>;

    // TODO: validate channel name
    fn handle(
        &mut self,
        msg: crate::entities::channel::events::Join,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        // get the channel or create it if it doesn't already exist
        #[allow(clippy::option_if_let_else)]
        let channel = if let Some(channel) = self.channels.get(&msg.channel_name) {
            channel
        } else {
            let channel = Channel::create(|_ctx| Channel::new());

            self.channels
                .entry(msg.channel_name.clone())
                .or_insert(channel)
        };

        // forward the user's join event onto the channel
        Box::pin(
            channel
                .send(msg)
                .into_actor(self)
                .map(|v, _, _| v.map_err(|e| e.into()).and_then(|v| v)),
        )
    }
}

impl Handler<crate::entities::common_events::Message> for Server {
    type Result = ();

    fn handle(
        &mut self,
        msg: crate::entities::common_events::Message,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        let dest = MessageDestination::get_destination_from_receiver(&self, &msg.to).unwrap();
        dest.send(ctx, msg);
    }
}

pub enum MessageDestination<'a> {
    User(&'a Server, Addr<User>),
    Channel(&'a Server, Addr<Channel>),
}

impl<'a> MessageDestination<'a> {
    pub fn get_destination_from_receiver<'b>(
        server: &'a Server,
        receiver: &Receiver<'b>,
    ) -> Option<Self> {
        match receiver {
            Receiver::Channel(c) => server
                .channels
                .get(&c.to_string())
                .cloned()
                .map(move |c| Self::Channel(server, c)),
            Receiver::User(_u) => todo!(),
        }
    }

    pub fn send(self, ctx: &mut Context<Server>, msg: crate::entities::common_events::Message) {
        match self {
            Self::Channel(actor, channel) => {
                ctx.spawn(
                    async move {
                        channel.send(msg).await.unwrap();
                    }
                    .into_actor(actor),
                );
            }
            Self::User(_actor, _u) => todo!(),
        }
    }
}
