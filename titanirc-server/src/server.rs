use crate::entities::{channel::Channel, user::User};

use std::{collections::HashMap, net::SocketAddr};

use actix::{io::FramedWrite, prelude::*};
use futures_util::future::TryFutureExt;
use tokio::net::TcpStream;
use tokio_util::codec::FramedRead;

pub struct Server {
    pub channels: HashMap<String, Addr<Channel>>,
}

impl Actor for Server {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Connection(pub TcpStream, pub SocketAddr);

impl Handler<Connection> for Server {
    type Result = ();

    fn handle(&mut self, Connection(stream, remote): Connection, server_ctx: &mut Self::Context) {
        println!("Accepted connection from {}", remote);

        User::create(move |ctx| {
            let (read, write) = tokio::io::split(stream);
            User::add_stream(FramedRead::new(read, titanirc_codec::Decoder), ctx);
            User {
                server: server_ctx.address(),
                writer: FramedWrite::new(write, titanirc_codec::Encoder, ctx),
                last_active: std::time::Instant::now(),
                nick: None,
            }
        });
    }
}

impl Handler<crate::entities::channel::events::Join> for Server {
    type Result = ResponseActFuture<Self, crate::entities::channel::events::JoinResult>;

    fn handle(
        &mut self,
        msg: crate::entities::channel::events::Join,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        let channel = if let Some(channel) = self.channels.get(&msg.channel_name) {
            channel
        } else {
            let channel = Channel::create(|ctx| Channel {
                members: Default::default(),
            });

            self.channels
                .entry(msg.channel_name.clone())
                .or_insert(channel)
        };

        Box::pin(
            channel
                .send(msg)
                .into_actor(self)
                .map(|v, _, _| v.map_err(|e| e.into()).and_then(|v| v)),
        )
    }
}
