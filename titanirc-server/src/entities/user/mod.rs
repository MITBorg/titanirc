mod commands;
pub mod events;

use crate::{entities::channel::events::JoinBroadcast, server::Server};

use std::sync::Arc;

use actix::{
    io::{FramedWrite, WriteHandler},
    prelude::*,
};
use std::time::{Duration, Instant};
use titanirc_types::{Channel, JoinCommand, ServerMessage, Source};
use tokio::{io::WriteHalf, net::TcpStream};

pub struct User {
    pub server: Addr<Server>,
    pub writer: FramedWrite<ServerMessage, WriteHalf<TcpStream>, titanirc_codec::Encoder>,
    pub last_active: Instant,
    pub nick: Option<String>,
}

// TODO: broadcast a leave to all the user's channels on actor shutdown

impl User {
    pub fn new(
        server: Addr<Server>,
        writer: FramedWrite<ServerMessage, WriteHalf<TcpStream>, titanirc_codec::Encoder>,
    ) -> Self {
        Self {
            server,
            writer,
            last_active: Instant::now(),
            nick: None,
        }
    }
}

/// Sends a ping to the user every 30 seconds, if we haven't received a protocol
/// message from the user in 240 seconds then disconnect.
fn schedule_ping(ctx: &mut <User as Actor>::Context) {
    ctx.run_later(Duration::from_secs(30), |act, ctx| {
        if Instant::now().duration_since(act.last_active) > Duration::from_secs(240) {
            // send `QUIT :Ping timeout: 120 seconds` & `ERROR :Closing Link: {ip} (Ping timeout: 120 seconds)`
            ctx.stop();
        }

        act.writer.write(titanirc_types::ServerMessage::Ping);
        schedule_ping(ctx);
    });
}

impl Actor for User {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        schedule_ping(ctx);
    }
}

/// Handles errors from our socket Writer.
impl WriteHandler<std::io::Error> for User {}

/// Handles `JoinBroadcast`s sent by a channel the user is in, and forwards a
/// `JOIN` onto them.
impl actix::Handler<Arc<JoinBroadcast>> for User {
    type Result = ();

    fn handle(&mut self, msg: Arc<JoinBroadcast>, _ctx: &mut Self::Context) -> Self::Result {
        self.writer.write(ServerMessage::Command(
            Source::User(bytes::Bytes::from(msg.nick.as_bytes().to_owned()).into()),
            JoinCommand {
                channel: Channel::from(bytes::Bytes::from(msg.channel_name.as_bytes().to_owned())),
            }
            .into(),
        ));
    }
}
