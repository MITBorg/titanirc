use actix::{
    io::{FramedWrite, WriteHandler},
    prelude::*,
};
use std::time::{Duration, Instant};
use titanirc_types::Command;
use tokio::{io::WriteHalf, net::TcpStream};

pub struct Session {
    pub writer:
        FramedWrite<titanirc_types::ServerMessage, WriteHalf<TcpStream>, titanirc_codec::Encoder>,
    pub last_active: Instant,
}

fn schedule_ping(ctx: &mut <Session as Actor>::Context) {
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

impl Actor for Session {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        schedule_ping(ctx);
    }
}

impl WriteHandler<std::io::Error> for Session {}

impl StreamHandler<Result<Command, std::io::Error>> for Session {
    /// This is main event loop for client requests
    fn handle(&mut self, cmd: Result<Command, std::io::Error>, _ctx: &mut Self::Context) {
        self.last_active = Instant::now();

        match cmd {
            Ok(Command::Nick(_v)) => {
                self.writer.write(titanirc_types::Reply::RplWelcome.into());
                self.writer.write(titanirc_types::Reply::RplYourHost.into());
                self.writer.write(titanirc_types::Reply::RplCreated.into());
                self.writer.write(titanirc_types::Reply::RplMyInfo.into());
                self.writer.write(titanirc_types::Reply::RplISupport.into());
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
