//! Used to encode/decode messages to and from the client.

#![deny(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

mod wire;

pub use crate::wire::{Decoder, Encoder};
