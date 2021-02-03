use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub socket_address: SocketAddr,
    pub database_uri: String,
}
