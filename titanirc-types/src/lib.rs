#![deny(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

use std::{hash::Hash, sync::Arc};

use bytes::Bytes;

pub mod protocol;

#[derive(Debug, Clone)]
pub struct UserIdent {
    nick: RegisteredNick,
    username: Arc<String>,
    host: Arc<String>,
}

/// A reference to a user's nickname. The actual nickname can be loaded using
/// `UserNick::load()`, however this username can be changed fairly quickly,
/// so the loaded value shouldn't be stored.
///
/// The user's nickname can be changed using `UserNick::set()` however this
/// doesn't do any validation that the user is actually allowed to use the
/// nick, nor does it send out any events alerting the users of the server
/// that the user's nick has changed.
#[derive(Debug, Clone)]
#[allow(clippy::clippy::module_name_repetitions)]
pub struct RegisteredNick(Arc<arc_swap::ArcSwapOption<Bytes>>);

impl RegisteredNick {
    #[must_use]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(Arc::new(arc_swap::ArcSwapOption::empty()))
    }

    #[must_use]
    pub fn load(&self) -> Option<Arc<Bytes>> {
        self.0.load().clone()
    }

    pub fn set(&self, nick: Arc<Bytes>) {
        self.0.store(Some(nick))
    }
}

impl Hash for RegisteredNick {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state)
    }
}

impl PartialEq for RegisteredNick {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RegisteredNick {}
