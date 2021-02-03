//! Handlers for commands originating from a user.

use std::{sync::Arc, time::Instant};

use actix::{Actor, AsyncContext, StreamHandler};
use titanirc_types::protocol::{
    commands::{
        Command, JoinCommand, ModeCommand, MotdCommand, NickCommand, PassCommand, PrivmsgCommand,
        VersionCommand,
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
            Ok(Command::Pass(v)) => self.handle_cmd(v, ctx),
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

impl CommandHandler<PassCommand<'static>> for super::User {
    fn handle_cmd(&mut self, command: PassCommand<'static>, _ctx: &mut Self::Context) {
        self.password_auth_in_progress = Some(command.password.to_bytes());
    }
}

impl CommandHandler<NickCommand<'static>> for super::User {
    fn handle_cmd(
        &mut self,
        NickCommand { nick, .. }: NickCommand<'static>,
        ctx: &mut Self::Context,
    ) {
        // TODO: when authenticated, the user should only be allowed to /NICK themselves
        //  to unregistered nicks or aliases.
        self.nick.set(Arc::new(nick.to_bytes()));

        self.server.do_send(crate::server::events::UserAuth {
            nick: nick.to_bytes(),
            user: ctx.address(),
            password: self.password_auth_in_progress.clone().unwrap(),
        });

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
        // TODO: needs to send MODE & NAMES (353, 366)
        self.server.do_send(crate::entities::channel::events::Join {
            channel_name: channel.to_bytes(),
            user_uuid: self.session_id,
            user: ctx.address(),
            nick: self.nick.clone(),
        });
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
        _ctx: &mut Self::Context,
    ) {
        // TODO: ensure the user has a nick before sending messages!!
        match receiver {
            primitives::Receiver::User(nick) => {
                let msg = crate::entities::common_events::UserMessage(
                    crate::entities::common_events::Message {
                        from: self.nick.clone(),
                        user_uuid: self.session_id,
                        to: nick,
                        message: free_text.to_string(),
                    },
                );

                self.server.do_send(msg);
            }
            primitives::Receiver::Channel(channel_name) => {
                if let Some(handle) = self.channels.get(channel_name.as_ref()) {
                    // todo: specific channel event?
                    let msg = crate::entities::common_events::ChannelMessage(
                        crate::entities::common_events::Message {
                            from: self.nick.clone(),
                            user_uuid: self.session_id,
                            to: channel_name,
                            message: free_text.to_string(),
                        },
                    );

                    // todo: should this be do_send?
                    handle.message.do_send(msg).unwrap();
                } else {
                    // todo: error back to user if not in channel
                    panic!("user not in channel")
                }
            }
        }
    }
}
