use std::net::SocketAddr;

use actix::prelude::*;
use tokio::net::TcpStream;

pub struct Server {}

impl Actor for Server {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Connection(pub TcpStream, pub SocketAddr);

impl Handler<Connection> for Server {
    type Result = ();

    fn handle(&mut self, Connection(_stream, remote): Connection, _: &mut Context<Self>) {
        println!("Accepted connection from {}", remote);
    }
}
