#![deny(clippy::nursery, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

use std::collections::HashMap;

use actix::{io::FramedWrite, Actor, Addr, AsyncContext};
use actix_rt::System;
use irc_proto::IrcCodec;
use tokio::{net::TcpListener, time::Instant};
use tokio_util::codec::FramedRead;
use tracing::{error, info, info_span, Instrument};
use tracing_subscriber::EnvFilter;

use crate::{client::Client, messages::UserConnected, server::Server};

pub mod channel;
pub mod client;
pub mod connection;
pub mod messages;
pub mod server;

pub const SERVER_NAME: &str = "my.cool.server";

#[actix_rt::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .pretty();
    subscriber.init();

    let server = Server::default().start();
    let listener = TcpListener::bind("127.0.0.1:6697").await?;

    actix_rt::spawn(start_tcp_acceptor_loop(listener, server));

    info!("Server listening on 127.0.0.1:6697");

    tokio::signal::ctrl_c().await?;
    System::current().stop();

    Ok(())
}

/// Start listening for new connections from clients, and create a new client handle for
/// them.
async fn start_tcp_acceptor_loop(listener: TcpListener, server: Addr<Server>) {
    while let Ok((stream, addr)) = listener.accept().await {
        let span = info_span!("connection", %addr);
        let _entered = span.clone().entered();

        info!("Accepted connection");

        let server = server.clone();

        actix_rt::spawn(async move {
            // split the stream into its read and write halves and setup codecs
            let (read, writer) = tokio::io::split(stream);
            let mut read = FramedRead::new(read, IrcCodec::new("utf8").unwrap());

            // ensure we have all the details required to actually connect the client to the server
            // (ie. we have a nick, user, etc)
            let Some(connection) = connection::negotiate_client_connection(&mut read).await.unwrap() else {
                error!("Failed to fully handshake with client, dropping connection");
                return;
            };

            // spawn the client's actor
            let handle = Client::create(|ctx| {
                // setup the writer codec for the user
                let writer = FramedWrite::new(writer, IrcCodec::new("utf8").unwrap(), ctx);

                // add the user's incoming tcp stream to the actor, messages over the tcp stream
                // will be sent to the actor over the `StreamHandler`
                ctx.add_stream(read);

                Client {
                    writer,
                    connection: connection.clone(),
                    server: server.clone(),
                    channels: HashMap::new(),
                    last_active: Instant::now(),
                    graceful_shutdown: false,
                    server_leave_reason: None,
                    span: span.clone(),
                }
            });

            // inform the server of the new connection
            server.do_send(UserConnected { handle, connection, span });
        }.instrument(info_span!("negotiation")));
    }
}
