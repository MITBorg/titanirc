//! Handlers for commands originating from a user.

use std::{sync::Arc, time::Instant};

use actix::{Actor, AsyncContext, StreamHandler, WrapFuture};
use titanirc_types::protocol::{
    commands::{
        Command, JoinCommand, ModeCommand, MotdCommand, NickCommand, PrivmsgCommand, VersionCommand,
    },
    primitives,
    replies::Reply,
};

pub trait CommandHandler<T>: Actor {
    fn handle_cmd(&mut self, command: T, ctx: &mut Self::Context);
}

impl StreamHandler<Result<Command<'static>, std::io::Error>> for super::User {
    fn handle(&mut self, cmd: Result<Command<'static>, std::io::Error>, ctx: &mut Self::Context) {
        self.last_active = Instant::now();

        match cmd {
            Ok(Command::Nick(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Join(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Mode(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Motd(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Privmsg(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Version(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Pong(_)) => {}
            Ok(cmd) => println!("cmd: {:?}", cmd),
            Err(e) => eprintln!("error decoding: {}", e),
        }
    }
}

// TODO: all the 'raw' writes using byte strings below probably need to
//  be wrapped in something a bit more friendly.

impl CommandHandler<NickCommand<'static>> for super::User {
    fn handle_cmd(
        &mut self,
        NickCommand { nick, .. }: NickCommand<'static>,
        _ctx: &mut Self::Context,
    ) {
        self.nick.set(Arc::new(nick.to_bytes()));

        self.writer.write(Reply::RplWelcome.into());
        self.writer.write(Reply::RplYourHost.into());
        self.writer.write(Reply::RplCreated.into());
        self.writer.write(Reply::RplMyInfo.into());
        self.writer.write(Reply::RplISupport.into());
        // LUSERS
        // RPL_UMODEIS
        // MOTD
    }
}

impl CommandHandler<JoinCommand<'static>> for super::User {
    fn handle_cmd(
        &mut self,
        JoinCommand { channel, .. }: JoinCommand<'static>,
        ctx: &mut Self::Context,
    ) {
        // TODO: ensure the user has a nick set before they join a channel!!!

        let server_addr = self.server.clone();
        let ctx_addr = ctx.address();
        let nick = self.nick.clone();
        let user_uuid = self.session_id;

        // TODO: needs to send MODE & NAMES (353, 366)
        ctx.spawn(
            async move {
                server_addr
                    .send(crate::entities::channel::events::Join {
                        channel_name: std::str::from_utf8(&channel.0[..]).unwrap().to_string(),
                        user_uuid,
                        user: ctx_addr,
                        nick,
                    })
                    .await
                    .unwrap()
                    .unwrap();

                println!("joined chan!");
            }
            .into_actor(self),
        );
    }
}

impl CommandHandler<ModeCommand<'static>> for super::User {
    fn handle_cmd(
        &mut self,
        ModeCommand { mode, .. }: ModeCommand<'static>,
        _ctx: &mut Self::Context,
    ) {
        self.writer.write(Reply::RplUmodeIs(mode).into())
    }
}

impl CommandHandler<MotdCommand<'static>> for super::User {
    fn handle_cmd(&mut self, _command: MotdCommand<'static>, _ctx: &mut Self::Context) {
        static SERVER_NAME: bytes::Bytes = bytes::Bytes::from_static(b"my.test.server");
        static MOTD1: bytes::Bytes = bytes::Bytes::from_static(b"Hello, welcome to this server!");
        static MOTD2: bytes::Bytes = bytes::Bytes::from_static(b"it's very cool!");

        self.writer
            .write(Reply::RplMotdStart(primitives::ServerName(SERVER_NAME.clone().into())).into());
        self.writer
            .write(Reply::RplMotd(primitives::FreeText(MOTD1.clone().into())).into());
        self.writer
            .write(Reply::RplMotd(primitives::FreeText(MOTD2.clone().into())).into());
        self.writer.write(Reply::RplEndOfMotd.into());
    }
}

impl CommandHandler<VersionCommand<'static>> for super::User {
    fn handle_cmd(&mut self, _command: VersionCommand<'static>, _ctx: &mut Self::Context) {
        static SERVER_NAME: bytes::Bytes = bytes::Bytes::from_static(b"my.test.server");
        static INFO: bytes::Bytes =
            bytes::Bytes::from_static(b"https://github.com/MITBorg/titanirc");

        self.writer.write(
            titanirc_types::protocol::replies::Reply::RplVersion(
                clap::crate_version!().to_string(),
                "release".to_string(),
                primitives::ServerName(SERVER_NAME.clone().into()),
                primitives::FreeText(INFO.clone().into()),
            )
            .into(),
        )
    }
}

impl CommandHandler<PrivmsgCommand<'static>> for super::User {
    fn handle_cmd(
        &mut self,
        PrivmsgCommand {
            receiver,
            free_text,
            ..
        }: PrivmsgCommand<'static>,
        ctx: &mut Self::Context,
    ) {
        // TODO: ensure the user has a nick before sending messages!!

        let msg = crate::entities::common_events::Message {
            from: self.nick.clone(), // TODO: this need to be a full user string i think
            user_uuid: self.session_id,
            to: receiver,
            message: free_text.to_string(),
        };

        let server_addr = self.server.clone();

        ctx.spawn(
            async move {
                server_addr.send(msg).await.unwrap();
            }
            .into_actor(self),
        );
    }
}
