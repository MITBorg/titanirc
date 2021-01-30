#![deny(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

mod entities;
mod error;
mod server;

use std::collections::HashMap;

use crate::{
    error::Result,
    server::{Connection, Server},
};

use actix::{Actor, AsyncContext, System};
use displaydoc::Display;
use thiserror::Error;
use tokio::net::TcpListener;

#[derive(Error, Debug, Display)]
pub enum InitError {
    /// Failed to bind to socket: {0}
    TcpBind(std::io::Error),
}

#[actix_rt::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("0.0.0.0:6667")
        .await
        .map_err(InitError::TcpBind)?;

    let stream = async_stream::stream! {
        loop {
            match listener.accept().await {
                Ok((socket, remote)) => yield Connection(socket, remote),
                Err(e) => eprintln!("Couldn't establish connection: {:?}", e)
            }
        }
    };

    Server::create(move |ctx| {
        ctx.add_message_stream(stream);
        Server {
            channels: HashMap::new(),
        }
    });

    println!("Running IRC server on 0.0.0.0:6667");

    tokio::signal::ctrl_c().await.expect("ctrl-c io");
    System::current().stop();

    Ok(())
}
