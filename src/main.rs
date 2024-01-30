#![deny(clippy::nursery, clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc
)]

use std::{collections::HashMap, str::FromStr, sync::Arc};

use actix::{io::FramedWrite, Actor, Addr, AsyncContext, Supervisor};
use actix_rt::{Arbiter, System};
use bytes::BytesMut;
use clap::Parser;
use futures::SinkExt;
use irc_proto::{Command, IrcCodec, Message};
use rand::seq::SliceRandom;
use sqlx::migrate::Migrator;
use tokio::{
    io::WriteHalf,
    net::{TcpListener, TcpStream},
    time::Instant,
};
use tokio_util::codec::FramedRead;
use tracing::{error, info, info_span, Instrument};
use tracing_subscriber::EnvFilter;

use crate::{
    client::Client, config::Args, messages::UserConnected, persistence::Persistence, server::Server,
};

pub mod channel;
pub mod client;
pub mod config;
pub mod connection;
pub mod database;
pub mod messages;
pub mod persistence;
pub mod server;

pub const SERVER_NAME: &str = "my.cool.server";

static MIGRATOR: Migrator = sqlx::migrate!();

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

    sqlx::any::install_default_drivers();
    let database = sqlx::Pool::connect_with(sqlx::any::AnyConnectOptions::from_str(
        &opts.config.database_uri,
    )?)
    .await?;

    MIGRATOR.run(&database).await?;

    let listen_address = opts.config.listen_address;
    let client_threads = opts.config.client_threads;

    let server_arbiter = Arbiter::new();

    let persistence_addr = {
        let database = database.clone();
        let config = opts.config.clone();

        Supervisor::start_in_arbiter(&server_arbiter.handle(), move |_ctx| Persistence {
            database,
            max_message_replay_since: config.max_message_replay_since,
            last_seen_clock: 0,
        })
    };

    let persistence = persistence_addr.clone();
    let server = Supervisor::start_in_arbiter(&server_arbiter.handle(), move |_ctx| Server {
        channels: HashMap::default(),
        clients: HashMap::default(),
        channel_arbiters: build_arbiters(opts.config.channel_threads),
        config: opts.config,
        persistence,
        max_clients: 0,
    });

    let listener = TcpListener::bind(listen_address).await?;

    actix_rt::spawn(start_tcp_acceptor_loop(
        listener,
        database,
        persistence_addr,
        server,
        client_threads,
    ));

    info!("Server listening on {}", listen_address);

    tokio::signal::ctrl_c().await?;
    System::current().stop();

    Ok(())
}

/// Start listening for new connections from clients, and create a new client handle for
/// them.
async fn start_tcp_acceptor_loop(
    listener: TcpListener,
    database: sqlx::Pool<sqlx::Any>,
    persistence: Addr<Persistence>,
    server: Addr<Server>,
    client_threads: usize,
) {
    let client_arbiters = Arc::new(build_arbiters(client_threads));

    while let Ok((stream, addr)) = listener.accept().await {
        let span = info_span!("connection", %addr);
        let _entered = span.clone().entered();

        info!("Accepted connection");

        let database = database.clone();
        let server = server.clone();
        let client_arbiters = client_arbiters.clone();
        let persistence = persistence.clone();

        actix_rt::spawn(async move {
            // split the stream into its read and write halves and setup codecs
            let (read, writer) = tokio::io::split(stream);
            let mut read = FramedRead::new(read, irc_codec());
            let mut write = tokio_util::codec::FramedWrite::new(writer, irc_codec());

            // ensure we have all the details required to actually connect the client to the server
            // (ie. we have a nick, user, etc)
            let connection = match connection::negotiate_client_connection(&mut read, &mut write, addr, &persistence, database).await {
                Ok(Some(v)) => v,
                Ok(None) => {
                    error!("Failed to fully handshake with client, dropping connection");

                    let command = Command::ERROR("You must use SASL to connect to this server".to_string());
                    if let Err(error) = write.send(Message { tags: None, prefix: None, command, }).await {
                        error!(%error, "Failed to send error message to client, forcefully closing connection.");
                    }

                    return;
                }
                Err(error) => {
                    error!(%error, "An error occurred whilst handshaking with client");

                    let command = Command::ERROR(error.to_string());
                    if let Err(error) = write.send(Message { tags: None, prefix: None, command, }).await {
                        error!(%error, "Failed to send error message to client, forcefully closing connection.");
                    }

                    return;
                }
            };

            // spawn the client's actor
            let handle = {
                let server = server.clone();
                let arbiter = client_arbiters.choose(&mut rand::thread_rng()).map_or_else(Arbiter::current, Arbiter::handle);
                let span = span.clone();
                let connection = connection.clone();

                Client::start_in_arbiter(&arbiter, move |ctx| {
                    // setup the writer codec for the user
                    let (stream, codec, buffer) = unpack_writer(write);
                    let writer = FramedWrite::from_buffer(stream, codec, buffer, ctx);

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
                        persistence,
                    }
                })
            };

            // inform the server of the new connection
            server.do_send(UserConnected { handle, connection, span });
        }.instrument(info_span!("negotiation")));
    }
}

/// Unpacks a tokio framed writer, for instantiating an Actix framed writer once connection
/// instantiation is complete.
#[must_use]
pub fn unpack_writer(
    mut writer: tokio_util::codec::FramedWrite<WriteHalf<TcpStream>, IrcCodec>,
) -> (WriteHalf<TcpStream>, IrcCodec, BytesMut) {
    let codec = std::mem::replace(writer.encoder_mut(), irc_codec());
    let bytes = writer.write_buffer_mut().split();
    let stream = writer.into_inner();

    (stream, codec, bytes)
}

#[must_use]
pub fn irc_codec() -> IrcCodec {
    IrcCodec::new("utf8").unwrap()
}

#[must_use]
pub fn build_arbiters(count: usize) -> Vec<Arbiter> {
    std::iter::repeat(())
        .take(count)
        .map(|()| Arbiter::new())
        .collect()
}
