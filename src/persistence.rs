pub mod events;

use actix::{Context, Handler, ResponseFuture};
use tracing::instrument;

use crate::persistence::events::{ChannelCreated, ChannelJoined, ChannelParted};

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
                 VALUES ((SELECT id FROM channels WHERE name = ?), (SELECT id FROM users WHERE username = ?), ?, ?)
                 ON CONFLICT(channel, user) DO UPDATE SET in_channel = excluded.in_channel"
            )
            .bind(msg.channel_name)
            .bind(msg.username)
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
                   AND user = (SELECT id FROM users WHERE username = ?)",
            )
            .bind(msg.channel_name)
            .bind(msg.username)
            .execute(&conn)
            .await
            .unwrap();
        })
    }
}
