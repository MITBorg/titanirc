use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::rngs::OsRng;

/// Attempts creation of a new user, returning the password of the user.
///
/// The returned password _is not_ guaranteed to be the password that was just set.
pub async fn create_user_or_fetch_password_hash(
    conn: &sqlx::Pool<sqlx::Any>,
    username: &str,
    password: &[u8],
) -> Result<(i64, String), sqlx::Error> {
    let password_hash = Argon2::default()
        .hash_password(password, &SaltString::generate(&mut OsRng))
        .unwrap()
        .to_string();

    sqlx::query_as(
        "INSERT INTO users (username, password)
         VALUES (?, ?)
         ON CONFLICT(username) DO UPDATE SET username = username
         RETURNING id, password",
    )
    .bind(username)
    .bind(password_hash)
    .fetch_one(conn)
    .await
}

/// Compares a password to a hash stored in the database.
pub fn verify_password(password: &[u8], hash: &PasswordHash) -> argon2::password_hash::Result<()> {
    Argon2::default().verify_password(password, hash)
}
