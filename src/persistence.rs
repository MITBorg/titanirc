pub mod events;

use std::{collections::HashMap, time::Duration};

use actix::{AsyncContext, Context, Handler, ResponseFuture, WrapFuture};
use chrono::Utc;
use itertools::Itertools;
use tracing::instrument;

use crate::{
    channel::permissions::Permission,
    connection::UserId,
    persistence::events::{
        ChannelCreated, ChannelJoined, ChannelMessage, ChannelParted,
        FetchAllUserChannelPermissions, FetchUnseenMessages, FetchUserChannels, ReserveNick,
        SetUserChannelPermissions,
    },
};

/// Takes events destined for other actors and persists them to the database.
pub struct Persistence {
    pub database: sqlx::Pool<sqlx::Any>,
    pub max_message_replay_since: Duration,
    pub last_seen_clock: i64,
}

impl Persistence {
    /// Grabs the current time to use as an ID, preventing against backwards clockskew.
    fn monotonically_increasing_id(&mut self) -> i64 {
        let now = Utc::now().timestamp_nanos();

        self.last_seen_clock = if now <= self.last_seen_clock {
            self.last_seen_clock + 1
        } else {
            now
        };

        self.last_seen_clock
    }
}

impl actix::Supervised for Persistence {}

impl actix::Actor for Persistence {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // truncate the messages table every 5 minutes for messages all users have seen
        ctx.run_interval(Duration::from_secs(300), |this, ctx| {
            let database = this.database.clone();
            let max_message_replay_since = this.max_message_replay_since;

            ctx.spawn(truncate_seen_messages(database, max_message_replay_since).into_actor(this));
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

impl Handler<FetchAllUserChannelPermissions> for Persistence {
    type Result = ResponseFuture<HashMap<UserId, Permission>>;

    fn handle(
        &mut self,
        msg: FetchAllUserChannelPermissions,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        let conn = self.database.clone();

        Box::pin(async move {
            sqlx::query_as::<_, (UserId, Permission)>(
                "SELECT user, permissions
                 FROM channel_users
                 WHERE channel = ?",
            )
            .bind(msg.channel_id.0)
            .fetch_all(&conn)
            .await
            .unwrap()
            .into_iter()
            .collect()
        })
    }
}

impl Handler<SetUserChannelPermissions> for Persistence {
    type Result = ResponseFuture<()>;

    fn handle(&mut self, msg: SetUserChannelPermissions, _ctx: &mut Self::Context) -> Self::Result {
        let conn = self.database.clone();

        Box::pin(async move {
            sqlx::query(
                "UPDATE channel_users
                 SET permissions = ?
                 WHERE user = ?
                   AND channel = ?",
            )
            .bind(msg.permissions)
            .bind(msg.user_id.0)
            .bind(msg.channel_id.0)
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
        let timestamp = self.monotonically_increasing_id();

        Box::pin(async move {
            sqlx::query(
                "INSERT INTO channel_messages (channel, timestamp, sender, message) VALUES (?, ?, ?, ?)",
            )
            .bind(msg.channel_id.0)
            .bind(timestamp)
            .bind(msg.sender)
            .bind(msg.message)
            .execute(&conn)
            .await
            .unwrap();

            if !msg.receivers.is_empty() {
                let query = format!(
                    "UPDATE channel_users
                     SET last_seen_message_timestamp = ?
                     WHERE channel = ?
                       AND user IN ({})",
                    msg.receivers.iter().map(|_| "?").join(",")
                );

                let mut query = sqlx::query(&query).bind(timestamp).bind(msg.channel_id.0);
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
        let max_message_reply_since =
            Utc::now() - chrono::Duration::from_std(self.max_message_replay_since).unwrap();

        Box::pin(async move {
            // select the last 500 messages, or the last message the user saw - whichever dataset
            // is smaller.
            let res = sqlx::query_as(
                "WITH channel AS (SELECT id FROM channels WHERE name = ?)
                 SELECT sender, message
                 FROM channel_messages
                 WHERE channel = (SELECT id FROM channel)
                    AND timestamp > MAX(
                      ?,
                      COALESCE((
                        SELECT last_seen_message_timestamp
                        FROM channel_users
                        WHERE channel = (SELECT id FROM channel)
                          AND user = ?
                      ), 0)
                    )
                 ORDER BY timestamp ASC",
            )
            .bind(&msg.channel_name)
            .bind(max_message_reply_since.timestamp_nanos())
            .bind(msg.user_id.0)
            .fetch_all(&conn)
            .await
            .unwrap();

            res
        })
    }
}

impl Handler<ReserveNick> for Persistence {
    type Result = ResponseFuture<bool>;

    fn handle(&mut self, msg: ReserveNick, _ctx: &mut Self::Context) -> Self::Result {
        let database = self.database.clone();

        Box::pin(async move {
            let (owning_user,): (i64,) = sqlx::query_as(
                "INSERT INTO user_nicks (nick, user)
                 VALUES (?, ?)
                 ON CONFLICT(nick) DO UPDATE SET nick = nick
                 RETURNING user",
            )
            .bind(msg.nick)
            .bind(msg.user_id.0)
            .fetch_one(&database)
            .await
            .unwrap();

            owning_user == msg.user_id.0
        })
    }
}

/// Remove any messages from the messages table whenever they've been seen by all users
/// or have passed their retention period
/// .
pub async fn truncate_seen_messages(db: sqlx::Pool<sqlx::Any>, max_replay_since: Duration) {
    // fetch the minimum last seen message by channel
    let messages = sqlx::query_as::<_, (i64, i64)>(
        "SELECT channel, MIN(last_seen_message_timestamp)
         FROM channel_users
         GROUP BY channel",
    )
    .fetch_all(&db)
    .await
    .unwrap();

    let max_replay_since = Utc::now() - chrono::Duration::from_std(max_replay_since).unwrap();

    // delete all messages that have been by all users or have passed their retention period
    for (channel, min_seen_timestamp) in messages {
        let mut tx = db.begin().await.unwrap();

        let remove_before = std::cmp::max(min_seen_timestamp, max_replay_since.timestamp_nanos());

        sqlx::query(
            "DELETE FROM channel_messages
             WHERE channel = ?
               AND timestamp <= ?",
        )
        .bind(channel)
        .bind(remove_before)
        .execute(&mut tx)
        .await
        .unwrap();

        tx.commit().await.unwrap();
    }
}
