use actix::{
    io::{FramedWrite, WriteHandler},
    prelude::*,
};
use std::time::{Duration, Instant};
use titanirc_types::Command;
use tokio::{io::WriteHalf, net::TcpStream};

pub mod events {
    use actix::prelude::*;

    #[derive(Message)]
    #[rtype(result = "()")]
    pub struct NameChange {
        pub old: String,
        pub new: String,
    }
}

pub struct User {
    pub server: Addr<crate::server::Server>,
    pub writer:
        FramedWrite<titanirc_types::ServerMessage, WriteHalf<TcpStream>, titanirc_codec::Encoder>,
    pub last_active: Instant,
    pub nick: Option<String>, // should probably be an arc so we dont have to keep cloning the string
}

fn schedule_ping(ctx: &mut <User as Actor>::Context) {
    ctx.run_later(Duration::from_secs(30), |act, ctx| {
        if Instant::now().duration_since(act.last_active) > Duration::from_secs(240) {
            // send `QUIT :Ping timeout: 120 seconds` & `ERROR :Closing Link: {ip} (Ping timeout: 120 seconds)`
            eprintln!("ping timeout");
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

impl WriteHandler<std::io::Error> for User {}

impl StreamHandler<Result<Command, std::io::Error>> for User {
    /// This is main event loop for client requests
    fn handle(&mut self, cmd: Result<Command, std::io::Error>, ctx: &mut Self::Context) {
        self.last_active = Instant::now();

        match cmd {
            Ok(Command::Nick(titanirc_types::NickCommand { nick })) => {
                self.writer.write(titanirc_types::Reply::RplWelcome.into());
                self.writer.write(titanirc_types::Reply::RplYourHost.into());
                self.writer.write(titanirc_types::Reply::RplCreated.into());
                self.writer.write(titanirc_types::Reply::RplMyInfo.into());
                self.writer.write(titanirc_types::Reply::RplISupport.into());
                self.nick = Some(nick.0);
                // LUSERS
                // RPL_UMODEIS
                // MOTD
            }
            Ok(Command::Join(titanirc_types::JoinCommand { channel })) => {
                if let Some(ref nick) = self.nick {
                    let server_addr = self.server.clone();
                    let ctx_addr = ctx.address();
                    let nick = nick.clone();

                    ctx.spawn(
                        async move {
                            server_addr
                                .send(crate::entities::channel::events::Join {
                                    channel_name: channel.0,
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
            Ok(Command::Mode(titanirc_types::ModeCommand { mode, .. })) => self
                .writer
                .write(titanirc_types::Reply::RplUmodeIs(mode).into()),
            Ok(Command::Motd(_)) => {
                self.writer.write(
                    titanirc_types::Reply::RplMotdStart(titanirc_types::ServerName(
                        "my.test.server".to_string(),
                    ))
                    .into(),
                );
                self.writer.write(
                    titanirc_types::Reply::RplMotd(titanirc_types::FreeText(
                        "Hello, welcome to this server!".to_string(),
                    ))
                    .into(),
                );
                self.writer.write(
                    titanirc_types::Reply::RplMotd(titanirc_types::FreeText(
                        "it's very cool!".to_string(),
                    ))
                    .into(),
                );
                self.writer
                    .write(titanirc_types::Reply::RplEndOfMotd.into());
            }
            Ok(Command::Version(_)) => self.writer.write(
                titanirc_types::Reply::RplVersion(
                    clap::crate_version!().to_string(),
                    "release".to_string(),
                    titanirc_types::ServerName("my.test.server".to_string()),
                    titanirc_types::FreeText("https://github.com/MITBorg/titanirc".to_string()),
                )
                .into(),
            ),
            Ok(Command::Pong(_)) => {}
            Ok(cmd) => println!("cmd: {:?}", cmd),
            Err(e) => eprintln!("error decoding: {}", e),
        }
    }
}

impl actix::Handler<std::sync::Arc<crate::entities::channel::events::JoinBroadcast>> for User {
    type Result = ();

    fn handle(
        &mut self,
        msg: std::sync::Arc<crate::entities::channel::events::JoinBroadcast>,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        self.writer.write(titanirc_types::ServerMessage::Command(
            titanirc_types::Source::User(titanirc_types::Nick(msg.nick.clone())),
            titanirc_types::Command::Join(titanirc_types::JoinCommand {
                channel: titanirc_types::Channel::from(msg.channel_name.clone()),
            }),
        ));
    }
}
