use crate::entities::{
    channel::{Channel, ChannelName},
    user::User,
};

use std::{collections::HashMap, net::SocketAddr};

use actix::{io::FramedWrite, prelude::*};
use titanirc_types::RegisteredNick;
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
    pub channels: HashMap<ChannelName, Addr<Channel>>,
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
    type Result = ();

    // TODO: validate channel name
    fn handle(
        &mut self,
        msg: crate::entities::channel::events::Join,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        // get the channel or create it if it doesn't already exist
        #[allow(clippy::option_if_let_else)]
        let channel = if let Some(channel) = self.channels.get(&msg.channel_name[..]) {
            channel
        } else {
            let channel_name = ChannelName::new(msg.channel_name.clone());

            let channel = Channel::create(|_ctx| Channel::new(channel_name.clone()));

            self.channels.entry(channel_name).or_insert(channel)
        };

        // forward the user's join event onto the channel
        channel.do_send(msg);
    }
}

impl Handler<crate::entities::common_events::UserMessage> for Server {
    type Result = ();

    fn handle(
        &mut self,
        _msg: crate::entities::common_events::UserMessage,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        // TODO: implement this when we have a `users` hashmap
        todo!()
    }
}
