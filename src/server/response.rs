use irc_proto::{Command, Message, Prefix, Response};

use crate::{server::Server, SERVER_NAME};

#[derive(Default)]
pub struct Motd {
    pub motd: Option<String>,
}

impl Motd {
    #[must_use]
    pub fn new(server: &Server) -> Self {
        Self {
            motd: server.config.motd.clone(),
        }
    }

    #[must_use]
    pub fn into_messages(self, for_user: String) -> Vec<Message> {
        if let Some(motd) = self.motd {
            let mut motd_messages = vec![Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_MOTDSTART,
                    vec![
                        for_user.to_string(),
                        format!("- {SERVER_NAME} Message of the day -"),
                    ],
                ),
            }];

            motd_messages.extend(motd.trim().split('\n').map(|v| Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_MOTD,
                    vec![for_user.to_string(), v.to_string()],
                ),
            }));

            motd_messages.push(Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_ENDOFMOTD,
                    vec![for_user, "End of /MOTD command.".to_string()],
                ),
            });

            motd_messages
        } else {
            vec![Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::ERR_NOMOTD,
                    vec![for_user, "MOTD File is missing".to_string()],
                ),
            }]
        }
    }
}

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
