#![deny(clippy::nursery, clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc
)]

pub mod channel;
pub mod client;
pub mod config;
pub mod connection;
pub mod database;
pub mod host_mask;
pub mod keys;
pub mod messages;
pub mod persistence;
pub mod server;

pub const SERVER_NAME: &str = "my.cool.server";
