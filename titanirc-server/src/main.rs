#![deny(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

mod config;
mod entities;
mod error;
mod models;
mod server;

use std::path::PathBuf;

use crate::{
    error::Result,
    server::{Connection, Server},
};

use actix::{Actor, AsyncContext, System};
use clap::Clap;
use displaydoc::Display;
use sqlx::migrate::Migrator;
use thiserror::Error;
use tokio::net::TcpListener;

static MIGRATOR: Migrator = sqlx::migrate!();

#[derive(Error, Debug, Display)]
pub enum InitError {
    /// Failed to bind to socket: {0}
    TcpBind(std::io::Error),
    /// Failed to read config file: {0}
    ConfigRead(std::io::Error),
    /// Failed to parse config file: {0}
    ConfigParse(toml::de::Error),
    /// Failed to connect to database: {0}
    SqlConnect(sqlx::Error),
    /// Failed to run migrations against the database: {0}
    SqlMigrate(sqlx::migrate::MigrateError),
}

#[derive(Clap)]
#[clap(version = clap::crate_version!(), author = clap::crate_authors!())]
struct Opts {
    /// Path to config file
    #[clap(short, long)]
    config: PathBuf,
}

#[actix_rt::main]
async fn main() -> Result<()> {
    let opts: Opts = Opts::parse();

    let config = std::fs::read(&opts.config).map_err(InitError::ConfigRead)?;
    let config: config::Config = toml::from_slice(&config).map_err(InitError::ConfigParse)?;

    let sql_pool = sqlx::SqlitePool::connect(&config.database_uri)
        .await
        .map_err(InitError::SqlConnect)?;
    MIGRATOR
        .run(&sql_pool)
        .await
        .map_err(InitError::SqlMigrate)?;

    let listener = TcpListener::bind(&config.socket_address)
        .await
        .map_err(InitError::TcpBind)?;

    // connection acceptor loop
    let stream = async_stream::stream! {
        loop {
            match listener.accept().await {
                Ok((socket, remote)) => yield Connection(socket, remote),
                Err(e) => eprintln!("Couldn't establish connection: {:?}", e)
            }
        }
    };

    // Spawn the server and pass connections from `stream` to `Handler<Connection>`.
    Server::create(move |ctx| {
        ctx.add_message_stream(stream);
        Server::new(sql_pool)
    });

    println!("Running IRC server on {}", &config.socket_address);

    tokio::signal::ctrl_c().await.expect("ctrl-c io");
    System::current().stop();

    Ok(())
}
