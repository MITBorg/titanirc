use std::{net::SocketAddr, str::FromStr};

use clap::Parser;
use serde::Deserialize;

#[derive(Parser)]
#[clap(version = clap::crate_version!(), author = clap::crate_authors!())]
pub struct Args {
    /// Turn debugging information on
    #[clap(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    #[clap(short, long)]
    pub config: Config,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub listen_address: SocketAddr,
    pub motd: Option<String>,
    /// Amount of threads to spawn for processing client commands, set to 0 to spawn clients on the
    /// main server thread.
    #[serde(default = "Config::default_client_threads")]
    pub client_threads: usize,
    /// Amount of threads to spawn for processing channel commands, set to 0 to spawn channels on
    /// the main server thread.
    #[serde(default = "Config::default_channel_threads")]
    pub channel_threads: usize,
}

impl Config {
    #[must_use]
    const fn default_client_threads() -> usize {
        1
    }

    #[must_use]
    const fn default_channel_threads() -> usize {
        1
    }
}

impl FromStr for Config {
    type Err = std::io::Error;

    fn from_str(path: &str) -> Result<Self, Self::Err> {
        let contents = std::fs::read(path)?;
        toml::from_slice(&contents)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}
