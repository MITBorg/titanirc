use sqlx::{Any, Pool};

#[derive(Copy, Clone)]
pub struct Keys {
    pub ip_salt: [u8; 32],
}

impl Keys {
    pub async fn new(pool: &Pool<Any>) -> Result<Self, sqlx::Error> {
        Ok(Self {
            ip_salt: fetch_or_create(pool, "ip_salt").await?.try_into().unwrap(),
        })
    }
}

async fn fetch_or_create(pool: &Pool<Any>, name: &str) -> Result<Vec<u8>, sqlx::Error> {
    sqlx::query_as(
        "INSERT INTO keys (name, enckey)
         VALUES (?, ?)
         ON CONFLICT(name) DO UPDATE SET enckey = enckey
         RETURNING enckey",
    )
    .bind(name)
    .bind(rand::random::<[u8; 32]>().to_vec())
    .fetch_one(pool)
    .await
    .map(|(v,)| v)
}
