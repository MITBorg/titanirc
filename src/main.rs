#![deny(clippy::nursery, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

use std::{collections::HashMap, sync::Arc};

use actix::{io::FramedWrite, Actor, Addr, AsyncContext, Supervisor};
use actix_rt::{Arbiter, System};
use clap::Parser;
use irc_proto::IrcCodec;
use rand::seq::SliceRandom;
use tokio::{net::TcpListener, time::Instant};
use tokio_util::codec::FramedRead;
use tracing::{error, info, info_span, Instrument};
use tracing_subscriber::EnvFilter;

use crate::{client::Client, config::Args, messages::UserConnected, server::Server};

pub mod channel;
pub mod client;
pub mod config;
pub mod connection;
pub mod messages;
pub mod server;

pub const SERVER_NAME: &str = "my.cool.server";

#[actix_rt::main]
async fn main() -> anyhow::Result<()> {
    // parse CLI arguments
    let opts: Args = Args::parse();

    // overrides the RUST_LOG variable to our own value based on the
    // amount of `-v`s that were passed when calling the service
    std::env::set_var(
        "RUST_LOG",
        match opts.verbose {
            1 => "debug",
            2 => "trace",
            _ => "info",
        },
    );

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .pretty();
    subscriber.init();

    let listen_address = opts.config.listen_address;
    let client_threads = opts.config.client_threads;

    let server = Supervisor::start_in_arbiter(&Arbiter::new().handle(), |_ctx| Server {
        channels: HashMap::default(),
        clients: HashMap::default(),
        channel_arbiters: build_arbiters(opts.config.channel_threads),
        config: opts.config,
    });
    let listener = TcpListener::bind(listen_address).await?;

    actix_rt::spawn(start_tcp_acceptor_loop(listener, server, client_threads));

    info!("Server listening on {}", listen_address);

    tokio::signal::ctrl_c().await?;
    System::current().stop();

    Ok(())
}

/// Start listening for new connections from clients, and create a new client handle for
/// them.
async fn start_tcp_acceptor_loop(
    listener: TcpListener,
    server: Addr<Server>,
    client_threads: usize,
) {
    let client_arbiters = Arc::new(build_arbiters(client_threads));

    while let Ok((stream, addr)) = listener.accept().await {
        let span = info_span!("connection", %addr);
        let _entered = span.clone().entered();

        info!("Accepted connection");

        let server = server.clone();
        let client_arbiters = client_arbiters.clone();

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
            let handle = {
                let server = server.clone();
                let arbiter = client_arbiters.choose(&mut rand::thread_rng()).map_or_else(Arbiter::current, Arbiter::handle);
                let span = span.clone();
                let connection = connection.clone();

                Client::start_in_arbiter(&arbiter, move |ctx| {
                    // setup the writer codec for the user
                    let writer = FramedWrite::new(writer, IrcCodec::new("utf8").unwrap(), ctx);

                    // add the user's incoming tcp stream to the actor, messages over the tcp stream
                    // will be sent to the actor over the `StreamHandler`
                    ctx.add_stream(read);

                    Client {
                        writer,
                        connection,
                        server,
                        channels: HashMap::new(),
                        last_active: Instant::now(),
                        graceful_shutdown: false,
                        server_leave_reason: None,
                        span,
                    }
                })
            };

            // inform the server of the new connection
            server.do_send(UserConnected { handle, connection, span });
        }.instrument(info_span!("negotiation")));
    }
}

#[must_use]
pub fn build_arbiters(count: usize) -> Vec<Arbiter> {
    std::iter::repeat(())
        .take(count)
        .map(|()| Arbiter::new())
        .collect()
}
