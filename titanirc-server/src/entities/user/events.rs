use actix::prelude::*;

#[derive(Message)]
#[rtype(result = "()")]
pub struct NameChange {
    pub old: String,
    pub new: String,
}
