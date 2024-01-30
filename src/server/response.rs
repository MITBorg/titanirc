use irc_proto::{Command, Message, Prefix, Response};
use itertools::Itertools;

use crate::{
    channel::permissions::Permission, connection::InitiatedConnection, server::Server, SERVER_NAME,
};

pub struct Whois {
    pub query: String,
    pub conn: Option<InitiatedConnection>,
    pub channels: Vec<(Permission, String)>,
}

impl IntoProtocol for Whois {
    fn into_messages(self, for_user: &str) -> Vec<Message> {
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

        let Some(conn) = self.conn else {
            return vec![msg!(ERR_NOSUCHNICK, self.query, "No such nick".to_string())];
        };

        let channels = self
            .channels
            .into_iter()
            .map(|(perm, channel)| format!("{}{channel}", perm.into_prefix()))
            .join(" ");

        // TODO: RPL_WHOISOPERATOR
        // TODO: RPL_WHOISACTUALLY
        // TODO: RPL_WHOISSECURE
        // TODO: fix missing rpl variants
        let mut out = vec![
            // msg!(RPL_WHOISREGNICK, self.conn.nick.to_string(), "has identified for this nick".to_string()),
            msg!(
                RPL_WHOISUSER,
                conn.nick.to_string(),
                conn.user,
                "*".to_string(),
                conn.real_name
            ),
            msg!(
                RPL_WHOISSERVER,
                conn.nick.to_string(),
                SERVER_NAME.to_string(),
                SERVER_NAME.to_string()
            ),
            msg!(
                RPL_WHOISIDLE,
                conn.nick.to_string(),
                "0".to_string(),
                conn.at.timestamp().to_string(),
                "seconds idle, signon time".to_string()
            ), // TODO
            msg!(RPL_WHOISCHANNELS, conn.nick.to_string(), channels),
            // msg!(RPL_WHOISACCOUNT, self.conn.nick.to_string(), self.conn.user.to_string(), "is logged in as".to_string()),
            // msg!(RPL_WHOISHOST, self.conn.nick.to_string(), format!("is connecting from {}@{} {}", self.conn.user, self.conn.host, self.conn.host)),
            // msg!(RPL_WHOISMODES, self.conn.nick.to_string(), format!("is using modes {}", self.conn.mode)),
        ];

        if let Some(msg) = conn.away {
            out.push(msg!(RPL_AWAY, conn.nick.to_string(), msg));
        }

        out.push(msg!(
            RPL_ENDOFWHOIS,
            conn.nick.to_string(),
            "End of /WHOIS list".to_string()
        ));

        out
    }
}

pub struct NoSuchNick {
    pub nick: String,
}

impl IntoProtocol for NoSuchNick {
    fn into_messages(self, for_user: &str) -> Vec<Message> {
        vec![Message {
            tags: None,
            prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
            command: Command::Response(
                Response::ERR_NOSUCHNICK,
                vec![for_user.to_string(), self.nick, "No such nick".to_string()],
            ),
        }]
    }
}

#[derive(Default)]
pub struct WhoList {
    pub list: Vec<crate::channel::response::ChannelWhoList>,
    pub query: String,
}

impl IntoProtocol for WhoList {
    fn into_messages(self, for_user: &str) -> Vec<Message> {
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

impl IntoProtocol for AdminInfo {
    fn into_messages(self, for_user: &str) -> Vec<Message> {
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

impl IntoProtocol for ListUsers {
    #[must_use]
    fn into_messages(self, for_user: &str) -> Vec<Message> {
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
}

impl IntoProtocol for Motd {
    #[must_use]
    fn into_messages(self, for_user: &str) -> Vec<Message> {
        let mut out = Vec::new();

        if let Some(motd) = self.motd {
            out.push(Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_MOTDSTART,
                    vec![
                        for_user.to_string(),
                        format!("- {SERVER_NAME} Message of the day -"),
                    ],
                ),
            });

            out.extend(motd.trim().split('\n').map(|v| Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_MOTD,
                    vec![for_user.to_string(), v.to_string()],
                ),
            }));

            out.push(Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::RPL_ENDOFMOTD,
                    vec![for_user.to_string(), "End of /MOTD command.".to_string()],
                ),
            });
        } else {
            out.push(Message {
                tags: None,
                prefix: Some(Prefix::ServerName(SERVER_NAME.to_string())),
                command: Command::Response(
                    Response::ERR_NOMOTD,
                    vec![for_user.to_string(), "MOTD File is missing".to_string()],
                ),
            });
        }

        out
    }
}

#[derive(Default)]
pub struct ChannelList {
    pub members: Vec<ChannelListItem>,
}

impl IntoProtocol for ChannelList {
    #[must_use]
    fn into_messages(self, for_user: &str) -> Vec<Message> {
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
                vec![for_user.to_string(), "End of /LIST".to_string()],
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

pub trait IntoProtocol {
    #[must_use]
    fn into_messages(self, for_user: &str) -> Vec<Message>;
}

impl IntoProtocol for () {
    fn into_messages(self, _for_user: &str) -> Vec<Message> {
        vec![]
    }
}

impl<T, E> IntoProtocol for Result<T, E>
where
    T: IntoProtocol,
    E: IntoProtocol,
{
    fn into_messages(self, for_user: &str) -> Vec<Message> {
        match self {
            Ok(v) => v.into_messages(for_user),
            Err(e) => e.into_messages(for_user),
        }
    }
}
