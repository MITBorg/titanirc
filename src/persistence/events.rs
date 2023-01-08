use actix::Message;
use tracing::Span;

#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelCreated {
    pub name: String,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelJoined {
    pub channel_name: String,
    pub username: String,
    pub span: Span,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelParted {
    pub channel_name: String,
    pub username: String,
    pub span: Span,
}
