mod commands;
pub mod events;

use crate::server::Server;

use std::{collections::HashMap, hash::Hash, sync::Arc};

use actix::{
    io::{FramedWrite, WriteHandler},
    prelude::*,
};

use derive_more::Deref;
use std::time::{Duration, Instant};
use titanirc_types::{
    protocol::commands::{JoinCommand, PrivmsgCommand},
    protocol::primitives::{Channel, FreeText, Nick, Receiver},
    protocol::replies::Source,
    protocol::ServerMessage,
    RegisteredNick,
};
use tokio::{io::WriteHalf, net::TcpStream};
use uuid::Uuid;

use super::channel::ChannelName;

#[derive(Debug, Deref, Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[allow(clippy::module_name_repetitions)]
pub struct UserUuid(Uuid);

pub struct User {
    pub session_id: UserUuid,
    pub server: Addr<Server>,
    pub writer: FramedWrite<
        WriteHalf<TcpStream>,
        titanirc_codec::Encoder,
        <titanirc_codec::Encoder as tokio_util::codec::Encoder<ServerMessage<'static>>>::Error,
    >,
    pub last_active: Instant,
    pub nick: RegisteredNick,

    pub password_auth_in_progress: Option<bytes::Bytes>,

    pub channels: HashMap<ChannelName, crate::entities::channel::Handle>,
}

// TODO: broadcast a leave to all the user's channels on actor shutdown

impl User {
    pub fn new(
        server: Addr<Server>,
        writer: FramedWrite<WriteHalf<TcpStream>, titanirc_codec::Encoder>,
        nick: RegisteredNick,
    ) -> Self {
        Self {
            session_id: UserUuid(Uuid::new_v4()),
            server,
            writer,
            password_auth_in_progress: None,
            last_active: Instant::now(),
            nick,
            channels: HashMap::new(),
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

        act.writer
            .write(titanirc_types::protocol::ServerMessage::Ping);
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

impl actix::Handler<crate::server::events::UserAuthResponse> for User {
    type Result = ();

    fn handle(
        &mut self,
        msg: crate::server::events::UserAuthResponse,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        todo!()
    }
}

impl actix::Handler<crate::entities::channel::Handle> for User {
    type Result = ();

    fn handle(
        &mut self,
        msg: crate::entities::channel::Handle,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        self.channels.insert(msg.channel_name.clone(), msg);
    }
}

/// Handles `Join`s sent by a channel the user is in, and forwards a
/// `JOIN` command to the user.
impl actix::Handler<Arc<super::channel::events::Join>> for User {
    type Result = ();

    fn handle(
        &mut self,
        msg: Arc<super::channel::events::Join>,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        self.writer.write(ServerMessage::Command(
            Source::User(Nick((*msg.nick.load().unwrap()).clone().into())),
            JoinCommand {
                _phantom: std::marker::PhantomData,
                channel: Channel((&msg.channel_name[..]).into()),
            }
            .into(),
        ));
    }
}

/// Handles messages that have been forwarded from a channel to this user.
impl actix::Handler<Arc<super::common_events::ChannelMessage>> for User {
    type Result = ();

    fn handle(
        &mut self,
        msg: Arc<super::common_events::ChannelMessage>,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        self.writer.write(ServerMessage::Command(
            Source::User(Nick((*msg.0.from.load().unwrap()).clone().into())),
            PrivmsgCommand {
                _phantom: std::marker::PhantomData,
                free_text: FreeText(msg.0.message.as_bytes().into()),
                receiver: Receiver::Channel(msg.0.to.clone()),
            }
            .into(),
        ));
    }
}

/// Handles messages that have been sent directly to this user.
impl actix::Handler<Arc<super::common_events::UserMessage>> for User {
    type Result = ();

    fn handle(
        &mut self,
        msg: Arc<super::common_events::UserMessage>,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        self.writer.write(ServerMessage::Command(
            Source::User(Nick((*msg.0.from.load().unwrap()).clone().into())),
            PrivmsgCommand {
                _phantom: std::marker::PhantomData,
                free_text: FreeText(msg.0.message.as_bytes().into()),
                receiver: Receiver::User(msg.0.to.clone()),
            }
            .into(),
        ));
    }
}
