use actix::prelude::*;
use titanirc_types::Command;

pub struct Session {}

impl Actor for Session {
    type Context = Context<Self>;
}

impl StreamHandler<Result<Command, std::io::Error>> for Session {
    /// This is main event loop for client requests
    fn handle(&mut self, cmd: Result<Command, std::io::Error>, _ctx: &mut Self::Context) {
        match cmd {
            Ok(cmd) => println!("cmd: {:?}", cmd),
            Err(e) => eprintln!("error decoding: {}", e),
        }
    }
}
