pub mod events;

use std::time::Duration;

use actix::{AsyncContext, Context, Handler, ResponseFuture, WrapFuture};
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

    fn started(&mut self, ctx: &mut Self::Context) {
        // truncate the messages table every 5 minutes for messages all users have seen
        ctx.run_interval(Duration::from_secs(300), |this, ctx| {
            let database = this.database.clone();

            ctx.spawn(truncate_seen_messages(database).into_actor(this));
        });
    }
}

/// Create a new channel in the database, if one doesn't already exist.
impl Handler<ChannelCreated> for Persistence {
    type Result = ResponseFuture<i64>;

    fn handle(&mut self, msg: ChannelCreated, _ctx: &mut Self::Context) -> Self::Result {
        let conn = self.database.clone();

        Box::pin(async move {
            sqlx::query_as(
                "INSERT OR IGNORE INTO channels
                 (name) VALUES (?)
                 ON CONFLICT(name)
                   DO UPDATE SET name = name
                 RETURNING id",
            )
            .bind(msg.name)
            .fetch_one(&conn)
            .await
            .map(|(v,)| v)
            .unwrap()
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
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(channel, user) DO UPDATE SET in_channel = excluded.in_channel",
            )
            .bind(msg.channel_id.0)
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
                 WHERE channel = ?
                   AND user = ?",
            )
            .bind(msg.channel_id.0)
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
                  WHERE user = ?
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
            let (idx,): (i64,) = sqlx::query_as(
                "INSERT INTO channel_messages (channel, idx, sender, message)
                     VALUES (?, COALESCE((SELECT MAX(idx) + 1 FROM channel_messages WHERE channel = ?), 0), ?, ?)
                     RETURNING idx",
            )
            .bind(msg.channel_id.0)
            .bind(msg.channel_id.0)
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

                let mut query = sqlx::query(&query).bind(idx).bind(msg.channel_id.0);
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

/// Remove any messages from the messages table whenever they've been seen by all users.
pub async fn truncate_seen_messages(db: sqlx::Pool<sqlx::Any>) {
    // fetch the minimum last seen message by channel
    let messages = sqlx::query_as::<_, (i64, i64)>(
        "SELECT channel, MIN(last_seen_message_idx)
         FROM channel_users
         GROUP BY channel",
    )
    .fetch_all(&db)
    .await
    .unwrap();

    // delete all messages that have been by all users
    for (channel, min_seen_id) in messages {
        sqlx::query("DELETE FROM channel_messages WHERE channel = ? AND idx < ?")
            .bind(channel)
            .bind(min_seen_id)
            .execute(&db)
            .await
            .unwrap();
    }
}
