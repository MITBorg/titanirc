pub mod events;

use actix::{Context, Handler, ResponseFuture};
use itertools::Itertools;
use tracing::instrument;

use crate::persistence::events::{
    ChannelCreated, ChannelJoined, ChannelMessage, ChannelParted, FetchUnseenMessages,
    FetchUserChannels,
};

/// Takes events destined for other actors and persists them to the database.
pub struct Persistence {
    pub database: sqlx::Pool<sqlx::Any>,
}

impl actix::Supervised for Persistence {}

impl actix::Actor for Persistence {
    type Context = Context<Self>;
}

/// Create a new channel in the database, if one doesn't already exist.
impl Handler<ChannelCreated> for Persistence {
    type Result = ResponseFuture<()>;

    fn handle(&mut self, msg: ChannelCreated, _ctx: &mut Self::Context) -> Self::Result {
        let conn = self.database.clone();

        Box::pin(async move {
            sqlx::query("INSERT OR IGNORE INTO channels (name) VALUES (?)")
                .bind(msg.name)
                .execute(&conn)
                .await
                .unwrap();
        })
    }
}

/// Insert a new channel member into the database.
impl Handler<ChannelJoined> for Persistence {
    type Result = ResponseFuture<()>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelJoined, _ctx: &mut Self::Context) -> Self::Result {
        let conn = self.database.clone();

        Box::pin(async move {
            sqlx::query(
                "INSERT INTO channel_users (channel, user, permissions, in_channel)
                 VALUES ((SELECT id FROM channels WHERE name = ?), ?, ?, ?)
                 ON CONFLICT(channel, user) DO UPDATE SET in_channel = excluded.in_channel",
            )
            .bind(msg.channel_name)
            .bind(msg.user_id.0)
            .bind(0i32)
            .bind(true)
            .execute(&conn)
            .await
            .unwrap();
        })
    }
}

/// Update a user to not being in a channel anymore.
impl Handler<ChannelParted> for Persistence {
    type Result = ResponseFuture<()>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: ChannelParted, _ctx: &mut Self::Context) -> Self::Result {
        let conn = self.database.clone();

        Box::pin(async move {
            sqlx::query(
                "UPDATE channel_users
                 SET in_channel = false
                 WHERE channel = (SELECT id FROM channels WHERE name = ?)
                   AND user = ?",
            )
            .bind(msg.channel_name)
            .bind(msg.user_id.0)
            .execute(&conn)
            .await
            .unwrap();
        })
    }
}

impl Handler<FetchUserChannels> for Persistence {
    type Result = ResponseFuture<Vec<String>>;

    fn handle(&mut self, msg: FetchUserChannels, _ctx: &mut Self::Context) -> Self::Result {
        let conn = self.database.clone();

        Box::pin(async move {
            sqlx::query_as(
                "SELECT channels.name
                  FROM channel_users
                  INNER JOIN channels
                    ON channels.id = channel_users.channel
                  WHERE user = (SELECT id FROM users WHERE username = ?)
                    AND in_channel = true",
            )
            .bind(msg.user_id.0)
            .fetch_all(&conn)
            .await
            .unwrap()
            .into_iter()
            .map(|(v,)| v)
            .collect()
        })
    }
}

impl Handler<ChannelMessage> for Persistence {
    type Result = ResponseFuture<()>;

    fn handle(&mut self, msg: ChannelMessage, _ctx: &mut Self::Context) -> Self::Result {
        let conn = self.database.clone();

        Box::pin(async move {
            let (channel, idx): (i64, i64) = sqlx::query_as(
                "WITH channel AS (SELECT id FROM channels WHERE name = ?)
                 INSERT INTO channel_messages (channel, idx, sender, message)
                     SELECT
                        channel.id,
                        COALESCE((SELECT MAX(idx) + 1 FROM channel_messages WHERE channel = channel.id), 0),
                        ?,
                        ?
                    FROM channel
                 RETURNING channel, idx",
            )
            .bind(msg.channel_name)
            .bind(msg.sender)
            .bind(msg.message)
            .fetch_one(&conn)
            .await
            .unwrap();

            if !msg.receivers.is_empty() {
                let query = format!(
                    "UPDATE channel_users
                     SET last_seen_message_idx = ?
                     WHERE channel = ?
                       AND user IN ({})",
                    msg.receivers.iter().map(|_| "?").join(",")
                );

                let mut query = sqlx::query(&query).bind(idx).bind(channel);
                for receiver in msg.receivers {
                    query = query.bind(receiver.0);
                }

                query.execute(&conn).await.unwrap();
            }
        })
    }
}

impl Handler<FetchUnseenMessages> for Persistence {
    type Result = ResponseFuture<Vec<(String, String)>>;

    #[instrument(parent = &msg.span, skip_all)]
    fn handle(&mut self, msg: FetchUnseenMessages, _ctx: &mut Self::Context) -> Self::Result {
        let conn = self.database.clone();

        Box::pin(async move {
            // select the last 500 messages, or the last message the user saw - whichever dataset
            // is smaller.
            let res = sqlx::query_as(
                "WITH channel AS (SELECT id FROM channels WHERE name = ?)
                 SELECT sender, message
                 FROM channel_messages
                 WHERE channel = (SELECT id FROM channel)
                    AND idx > MAX(
                        (
                            SELECT MAX(0, MAX(idx) - 500)
                            FROM channel_messages
                            WHERE channel = (SELECT id FROM channel)
                        ),
                        (
                            SELECT last_seen_message_idx
                            FROM channel_users
                            WHERE channel = (SELECT id FROM channel)
                              AND user = ?
                        )
                    )
                 ORDER BY idx ASC",
            )
            .bind(msg.channel_name.to_string())
            .bind(msg.user_id.0)
            .fetch_all(&conn)
            .await
            .unwrap();

            res
        })
    }
}
