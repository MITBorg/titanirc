use displaydoc::Display;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug, Display)]
pub enum Error {
    /// Failed to initialise server: {0}
    InitError(#[from] crate::InitError),
}
