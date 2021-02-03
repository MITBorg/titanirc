#[derive(sqlx::FromRow)]
pub struct User {
    id: u64,
    password: String,
    op: bool,
}

impl User {
    pub async fn create_user(
        conn: &sqlx::SqlitePool,
        nick: &[u8],
        password: bytes::Bytes,
    ) -> Result<(), sqlx::Error> {
        let mut tx = conn.begin().await?;

        sqlx::query("INSERT INTO users (password) VALUES (?)")
            .bind(
                tokio::task::spawn_blocking(move || bcrypt::hash(&password, 10).unwrap())
                    .await
                    .unwrap(),
            )
            .execute(&mut tx)
            .await?;

        sqlx::query("INSERT INTO users_aliases (nick, user_id) VALUES (?, LAST_INSERT_ROWID())")
            .bind(nick)
            .execute(&mut tx)
            .await?;

        tx.commit().await?;

        Ok(())
    }

    pub async fn fetch_by_nick(
        conn: &sqlx::SqlitePool,
        nick: &[u8],
    ) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM users WHERE users.id = (SELECT user_id FROM users_aliases WHERE nick = ?)")
            .bind(&nick)
            .fetch_optional(conn)
            .await
    }

    pub fn password_matches(&self, password: &[u8]) -> bool {
        bcrypt::verify(password, &self.password).unwrap()
    }
}
