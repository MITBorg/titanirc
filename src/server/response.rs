use irc_proto::{Command, Message, Prefix, Response};

use crate::{server::Server, SERVER_NAME};

#[derive(Default)]
pub struct WhoList {
    pub list: Vec<crate::channel::response::ChannelWhoList>,
    pub query: String,
}

impl WhoList {
    #[must_use]
    pub fn into_messages(self, for_user: &str) -> Vec<Message> {
        let mut out: Vec<_> = self
            .list
            .into_iter()
            .flat_map(|v| v.into_messages(for_user))
            .collect();

        out.push(Message {
            tags: None,
            prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
            command: Command::Response(
                Response::RPL_ENDOFWHO,
                vec![
                    for_user.to_string(),
                    self.query,
                    "End of WHO list".to_string(),
                ],
            ),
        });

        out
    }
}

pub struct AdminInfo {
    pub line1: String,
    pub line2: String,
    pub email: String,
}

impl AdminInfo {
    #[must_use]
    pub fn into_messages(self, for_user: &str) -> Vec<Message> {
        macro_rules! msg {
            ($response:ident, $($payload:expr),*) => {

                Message {
                    tags: None,
                    prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                    command: Command::Response(
                        Response::$response,
                        vec![for_user.to_string(), $($payload),*],
                    ),
                }
            };
        }

        vec![
            msg!(
                RPL_ADMINME,
                SERVER_NAME.to_string(),
                "Administrative info".to_string()
            ),
            msg!(RPL_ADMINLOC1, self.line1),
            msg!(RPL_ADMINLOC2, self.line2),
            msg!(RPL_ADMINEMAIL, self.email),
        ]
    }
}

pub struct ListUsers {
    pub current_clients: usize,
    pub max_clients: usize,
    pub operators_online: usize,
    pub channels_formed: usize,
}

impl ListUsers {
    #[must_use]
    pub fn into_messages(self, for_user: &str) -> Vec<Message> {
        macro_rules! msg {
            ($response:ident, $($payload:expr),*) => {

                Message {
                    tags: None,
                    prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                    command: Command::Response(
                        Response::$response,
                        vec![for_user.to_string(), $($payload),*],
                    ),
                }
            };
        }

        vec![
            msg!(
                RPL_LUSERCLIENT,
                format!(
                    "There are {} users and 0 invisible on 1 servers",
                    self.current_clients
                )
            ),
            msg!(
                RPL_LUSEROP,
                "0".to_string(),
                "operator(s) online".to_string()
            ),
            msg!(
                RPL_LUSERCHANNELS,
                self.channels_formed.to_string(),
                "channels formed".to_string()
            ),
            msg!(
                RPL_LUSERME,
                format!(
                    "I have {} clients and 1 servers",
                    self.current_clients.to_string()
                )
            ),
            msg!(
                RPL_LOCALUSERS,
                self.current_clients.to_string(),
                self.max_clients.to_string(),
                format!(
                    "Current local users {}, max {}",
                    self.current_clients, self.max_clients
                )
            ),
            msg!(
                RPL_GLOBALUSERS,
                self.current_clients.to_string(),
                self.max_clients.to_string(),
                format!(
                    "Current global users {}, max {}",
                    self.current_clients, self.max_clients
                )
            ),
        ]
    }
}

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
