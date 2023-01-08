use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::rngs::OsRng;
use sqlx::{database::HasArguments, Database, Encode, Executor, FromRow, IntoArguments, Type};

/// Fetches the given user's password from the database.
pub async fn fetch_password_hash<'a, E: Executor<'a>>(
    conn: E,
    username: &'a str,
) -> Result<Option<String>, sqlx::Error>
where
    for<'b> &'b str: Type<E::Database> + Encode<'b, E::Database>,
    <E::Database as HasArguments<'a>>::Arguments: IntoArguments<'a, E::Database>,
    for<'b> (String,): FromRow<'b, <E::Database as Database>::Row>,
{
    let res = sqlx::query_as("SELECT password FROM users WHERE username = ?")
        .bind(username)
        .fetch_optional(conn)
        .await?
        .map(|(v,)| v);

    Ok(res)
}

/// Creates a new user, returning an error if the user already exists.
pub async fn create_user<'a, E: Executor<'a>>(
    conn: E,
    username: &'a str,
    password: &[u8],
) -> Result<(), sqlx::Error>
where
    for<'b> &'b str: Type<E::Database> + Encode<'b, E::Database>,
    for<'b> String: Type<E::Database> + Encode<'b, E::Database>,
    <E::Database as HasArguments<'a>>::Arguments: IntoArguments<'a, E::Database>,
{
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(password, &salt)
        .unwrap()
        .to_string();

    sqlx::query("INSERT INTO users (username, password) VALUES (?, ?)")
        .bind(username)
        .bind(password_hash)
        .execute(conn)
        .await
        .map(|_| ())
}

/// Compares a password to a hash stored in the database.
pub fn verify_password(password: &[u8], hash: &PasswordHash) -> argon2::password_hash::Result<()> {
    Argon2::default().verify_password(password, hash)
}
