use irc_proto::{Command, Message, Prefix, Response};
use itertools::Itertools;

use crate::{
    channel::{permissions::Permission, Channel, CurrentChannelTopic},
    connection::InitiatedConnection,
    server::response::IntoProtocol,
    SERVER_NAME,
};

pub struct ChannelTopic {
    pub channel_name: String,
    pub topic: Option<CurrentChannelTopic>,
    pub skip_on_none: bool,
}

impl ChannelTopic {
    #[must_use]
    pub fn new(channel: &Channel, skip_on_none: bool) -> Self {
        Self {
            channel_name: channel.name.to_string(),
            topic: channel.topic.clone(),
            skip_on_none,
        }
    }
}

impl IntoProtocol for ChannelTopic {
    fn into_messages(self, for_user: &str) -> Vec<Message> {
        if let Some(topic) = self.topic {
            vec![
                Message {
                    tags: None,
                    prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                    command: Command::Response(
                        Response::RPL_TOPIC,
                        vec![
                            for_user.to_string(),
                            self.channel_name.to_string(),
                            topic.topic,
                        ],
                    ),
                },
                Message {
                    tags: None,
                    prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                    command: Command::Response(
                        Response::RPL_TOPICWHOTIME,
                        vec![
                            for_user.to_string(),
                            self.channel_name.to_string(),
                            topic.set_by,
                            topic.set_time.timestamp().to_string(),
                        ],
                    ),
                },
            ]
        } else if !self.skip_on_none {
            vec![Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_NOTOPIC,
                    vec![
                        for_user.to_string(),
                        self.channel_name,
                        "No topic is set".to_string(),
                    ],
                ),
            }]
        } else {
            vec![]
        }
    }
}

pub struct ChannelWhoList {
    pub channel_name: String,
    pub nick_list: Vec<(Permission, InitiatedConnection)>,
}

impl ChannelWhoList {
    #[must_use]
    pub fn new(channel: &Channel) -> Self {
        Self {
            channel_name: channel.name.to_string(),
            nick_list: channel
                .clients
                .values()
                .map(|v| (channel.get_user_permissions(&v.to_host_mask()), v.clone()))
                .collect(),
        }
    }
}

impl IntoProtocol for ChannelWhoList {
    fn into_messages(self, for_user: &str) -> Vec<Message> {
        let mut out = Vec::with_capacity(self.nick_list.len());

        for (perm, conn) in self.nick_list {
            let presence = if conn.away.is_some() { "G" } else { "H" };

            out.push(Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_WHOREPLY,
                    vec![
                        for_user.to_string(),
                        self.channel_name.to_string(),
                        conn.user,
                        conn.cloak.to_string(),
                        SERVER_NAME.to_string(),
                        conn.nick,
                        format!("{presence}{}", perm.into_prefix()), // TODO: user modes & server operator
                        "0".to_string(),
                        conn.real_name,
                    ],
                ),
            });
        }

        out
    }
}

pub struct ChannelNamesList {
    pub channel_name: String,
    pub nick_list: Vec<(Permission, InitiatedConnection)>,
}

impl ChannelNamesList {
    #[must_use]
    pub fn new(channel: &Channel) -> Self {
        Self {
            channel_name: channel.name.to_string(),
            nick_list: channel
                .clients
                .values()
                .map(|v| (channel.get_user_permissions(&v.to_host_mask()), v.clone()))
                .collect(),
        }
    }

    #[must_use]
    pub const fn empty(channel_name: String) -> Self {
        Self {
            channel_name,
            nick_list: vec![],
        }
    }

    #[must_use]
    pub fn into_messages(self, for_user: String, with_hostnames: bool) -> Vec<Message> {
        let nick_list = self
            .nick_list
            .into_iter()
            .map(|(permission, connection)| {
                let permission = permission.into_prefix();

                if with_hostnames {
                    format!("{permission}{}", connection.to_nick())
                } else {
                    format!("{permission}{}", connection.nick)
                }
            })
            .join(" ");

        vec![
            Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_NAMREPLY,
                    vec![
                        for_user.to_string(),
                        "=".to_string(),
                        self.channel_name,
                        nick_list,
                    ],
                ),
            },
            Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_ENDOFNAMES,
                    vec![for_user, "End of /NAMES list".to_string()],
                ),
            },
        ]
    }
}

#[derive(Copy, Clone)]
pub enum ChannelInviteResult {
    Successful,
    NoSuchUser,
    UserAlreadyOnChannel,
    NotOnChannel,
}

impl ChannelInviteResult {
    #[must_use]
    pub fn into_message(
        self,
        invited_user: String,
        channel: String,
        for_user: String,
    ) -> Option<Message> {
        let command = match self {
            Self::Successful => Command::Response(
                Response::RPL_INVITING,
                vec![for_user, invited_user, channel],
            ),
            Self::NoSuchUser => return None,
            Self::UserAlreadyOnChannel => Command::Response(
                Response::ERR_USERONCHANNEL,
                vec![
                    for_user,
                    invited_user,
                    channel,
                    "is already on channel".to_string(),
                ],
            ),
            Self::NotOnChannel => Command::Response(
                Response::ERR_NOTONCHANNEL,
                vec![for_user, channel, "You're not on that channel".to_string()],
            ),
        };

        Some(Message {
            tags: None,
            prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
            command,
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub enum ChannelJoinRejectionReason {
    Banned,
}

impl IntoProtocol for ChannelJoinRejectionReason {
    fn into_messages(self, for_user: &str) -> Vec<Message> {
        match self {
            Self::Banned => vec![Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::ERR_BANNEDFROMCHAN,
                    vec![for_user.to_string(), "Cannot join channel (+b)".to_string()],
                ),
            }],
        }
    }
}

pub struct MissingPrivileges(pub Prefix, pub String);

impl MissingPrivileges {
    #[must_use]
    pub fn into_message(self) -> Message {
        Message {
            tags: None,
            prefix: None,
            command: Command::Response(
                Response::ERR_CHANOPRIVSNEEDED,
                vec![
                    self.0.to_string(),
                    self.1,
                    "You're not channel operator".to_string(),
                ],
            ),
        }
    }
}
