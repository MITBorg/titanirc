use crate::session::Session;

use std::net::SocketAddr;

use actix::{io::FramedWrite, prelude::*};
use tokio::net::TcpStream;
use tokio_util::codec::FramedRead;

pub struct Server {}

impl Actor for Server {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Connection(pub TcpStream, pub SocketAddr);

impl Handler<Connection> for Server {
    type Result = ();

    fn handle(&mut self, Connection(stream, remote): Connection, _ctx: &mut Self::Context) {
        println!("Accepted connection from {}", remote);

        Session::create(move |ctx| {
            let (read, write) = tokio::io::split(stream);
            Session::add_stream(FramedRead::new(read, titanirc_codec::Decoder), ctx);
            Session {
                writer: FramedWrite::new(write, titanirc_codec::Encoder, ctx),
                last_active: std::time::Instant::now(),
            }
        });
    }
}
