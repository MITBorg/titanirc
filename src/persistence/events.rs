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

#[derive(Message)]
#[rtype(result = "Vec<String>")]
pub struct FetchUserChannels {
    pub username: String,
    pub span: Span,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelMessage {
    pub channel_name: String,
    pub sender: String,
    pub message: String,
    pub receivers: Vec<String>,
}

#[derive(Message)]
#[rtype(result = "Vec<(String, String)>")]
pub struct FetchUnseenMessages {
    pub channel_name: String,
    pub username: String,
    pub span: Span,
}
