use crate::SERVER_NAME;
use irc_proto::{Command, Message, Prefix, Response};

#[derive(Default)]
pub struct ChannelList {
    pub members: Vec<ChannelListItem>,
}

impl ChannelList {
    #[must_use]
    pub fn into_messages(self, for_user: String) -> Vec<Message> {
        let mut messages = Vec::with_capacity(self.members.len() + 2);

        messages.push(Message {
            tags: None,
            prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
            command: Command::Response(
                Response::RPL_LISTSTART,
                vec![
                    for_user.to_string(),
                    "Channel".to_string(),
                    "Users  Name".to_string(),
                ],
            ),
        });

        for item in self.members {
            messages.push(Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_LIST,
                    vec![
                        for_user.to_string(),
                        item.channel_name,
                        item.client_count.to_string(),
                        item.topic.unwrap_or_default(),
                    ],
                ),
            });
        }

        messages.push(Message {
            tags: None,
            prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
            command: Command::Response(
                Response::RPL_LISTEND,
                vec![for_user, "End of /LIST".to_string()],
            ),
        });

        messages
    }
}

pub struct ChannelListItem {
    pub channel_name: String,
    pub client_count: usize,
    pub topic: Option<String>,
}
