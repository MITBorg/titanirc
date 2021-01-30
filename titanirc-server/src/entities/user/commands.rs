use std::time::Instant;

use actix::{Actor, AsyncContext, StreamHandler, WrapFuture};
use titanirc_types::{Command, JoinCommand, ModeCommand, MotdCommand, NickCommand, VersionCommand};

pub trait CommandHandler<T>: Actor {
    fn handle_cmd(&mut self, command: T, ctx: &mut Self::Context);
}

impl StreamHandler<Result<Command, std::io::Error>> for super::User {
    fn handle(&mut self, cmd: Result<Command, std::io::Error>, ctx: &mut Self::Context) {
        self.last_active = Instant::now();

        match cmd {
            Ok(Command::Nick(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Join(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Mode(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Motd(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Version(v)) => self.handle_cmd(v, ctx),
            Ok(Command::Pong(_)) => {}
            Ok(cmd) => println!("cmd: {:?}", cmd),
            Err(e) => eprintln!("error decoding: {}", e),
        }
    }
}

// TODO: all the 'raw' writes using byte strings below probably need to
//  be wrapped in something a bit more friendly.

impl CommandHandler<NickCommand> for super::User {
    fn handle_cmd(&mut self, NickCommand { nick }: NickCommand, _ctx: &mut Self::Context) {
        self.writer.write(titanirc_types::Reply::RplWelcome.into());
        self.writer.write(titanirc_types::Reply::RplYourHost.into());
        self.writer.write(titanirc_types::Reply::RplCreated.into());
        self.writer.write(titanirc_types::Reply::RplMyInfo.into());
        self.writer.write(titanirc_types::Reply::RplISupport.into());
        self.nick = Some(std::str::from_utf8(&nick.0[..]).unwrap().to_string());
        // LUSERS
        // RPL_UMODEIS
        // MOTD
    }
}

impl CommandHandler<JoinCommand> for super::User {
    fn handle_cmd(&mut self, JoinCommand { channel }: JoinCommand, ctx: &mut Self::Context) {
        if let Some(ref nick) = self.nick {
            let server_addr = self.server.clone();
            let ctx_addr = ctx.address();
            let nick = nick.clone();

            ctx.spawn(
                async move {
                    server_addr
                        .send(crate::entities::channel::events::Join {
                            channel_name: std::str::from_utf8(&channel.0[..]).unwrap().to_string(),
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
}

impl CommandHandler<ModeCommand> for super::User {
    fn handle_cmd(&mut self, ModeCommand { mode, .. }: ModeCommand, _ctx: &mut Self::Context) {
        self.writer
            .write(titanirc_types::Reply::RplUmodeIs(mode).into())
    }
}

impl CommandHandler<MotdCommand> for super::User {
    fn handle_cmd(&mut self, _command: MotdCommand, _ctx: &mut Self::Context) {
        static SERVER_NAME: bytes::Bytes = bytes::Bytes::from_static(b"my.test.server");
        static MOTD1: bytes::Bytes = bytes::Bytes::from_static(b"Hello, welcome to this server!");
        static MOTD2: bytes::Bytes = bytes::Bytes::from_static(b"it's very cool!");

        self.writer.write(
            titanirc_types::Reply::RplMotdStart(titanirc_types::ServerName(SERVER_NAME.clone()))
                .into(),
        );
        self.writer
            .write(titanirc_types::Reply::RplMotd(titanirc_types::FreeText(MOTD1.clone())).into());
        self.writer
            .write(titanirc_types::Reply::RplMotd(titanirc_types::FreeText(MOTD2.clone())).into());
        self.writer
            .write(titanirc_types::Reply::RplEndOfMotd.into());
    }
}

impl CommandHandler<VersionCommand> for super::User {
    fn handle_cmd(&mut self, _command: VersionCommand, _ctx: &mut Self::Context) {
        static SERVER_NAME: bytes::Bytes = bytes::Bytes::from_static(b"my.test.server");
        static INFO: bytes::Bytes =
            bytes::Bytes::from_static(b"https://github.com/MITBorg/titanirc");

        self.writer.write(
            titanirc_types::Reply::RplVersion(
                clap::crate_version!().to_string(),
                "release".to_string(),
                titanirc_types::ServerName(SERVER_NAME.clone()),
                titanirc_types::FreeText(INFO.clone()),
            )
            .into(),
        )
    }
}
