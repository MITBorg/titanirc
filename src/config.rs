use std::{net::SocketAddr, str::FromStr, time::Duration};

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
    pub database_uri: String,
    pub motd: Option<String>,
    /// Maximum amount of messages to replay upon rejoin to a channel, if set to 0 an unlimited
    /// amount of messages will be retained. Defaults to 1 day.
    #[serde(
        default = "Config::default_max_message_replay_since",
        with = "serde_humantime"
    )]
    pub max_message_replay_since: Duration,
    /// Amount of threads to spawn for processing client commands, set to 0 to spawn clients on the
    /// main server thread. Defaults to 1 thread.
    #[serde(default = "Config::default_client_threads")]
    pub client_threads: usize,
    /// Amount of threads to spawn for processing channel commands, set to 0 to spawn channels on
    /// the main server thread. Defaults to 1 thread.
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

    #[must_use]
    const fn default_max_message_replay_since() -> Duration {
        Duration::from_secs(24 * 60 * 60)
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
