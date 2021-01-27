#![deny(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

mod wire;

pub use crate::wire::{Decoder, Encoder};
