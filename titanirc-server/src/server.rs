use crate::entities::{channel::Channel, user::User};

use std::{collections::HashMap, net::SocketAddr};

use actix::{io::FramedWrite, prelude::*};
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
}

impl Server {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
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
            let (read, write) = tokio::io::split(stream);
            let read = FramedRead::new(read, titanirc_codec::Decoder);
            let write = FramedWrite::new(write, titanirc_codec::Encoder, ctx);

            // Make our new `User` handle all events from this socket in `StreamHandler<Result<Command, _>>`.
            ctx.add_stream(read);

            User::new(server_ctx.address(), write)
        });
    }
}

/// Send by `User` actors to arbitrate access to the requested channel.
impl Handler<crate::entities::channel::events::Join> for Server {
    type Result = ResponseActFuture<Self, crate::entities::channel::events::JoinResult>;

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
