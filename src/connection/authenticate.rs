//! An actor to handle the user's initial authentication.
//!
//! The connection initiation process is expected to call the actor with
//! every `AUTHENTICATE` command it receives from the client, and this
//! actor will return back with either a response for the user, or the user's
//! ID, once logged in.
//!
//! The user will be created if the username does not exist.

use std::{
    io::{Error, ErrorKind},
    str::FromStr,
};

use actix::{Actor, ActorContext, Context, Handler, Message, ResponseFuture};
use argon2::PasswordHash;
use base64::{prelude::BASE64_STANDARD, Engine};
use futures::TryFutureExt;
use irc_proto::Command;

use crate::{
    connection::{
        sasl::{AuthStrategy, SaslAborted, SaslFail, SaslStrategyUnsupported},
        UserId,
    },
    database::verify_password,
};

pub struct Authenticate {
    pub selected_strategy: Option<AuthStrategy>,
    pub database: sqlx::Pool<sqlx::Any>,
}

impl Actor for Authenticate {
    type Context = Context<Self>;
}

impl Handler<AuthenticateMessage> for Authenticate {
    type Result = ResponseFuture<Result<AuthenticateResult, std::io::Error>>;

    fn handle(&mut self, msg: AuthenticateMessage, ctx: &mut Self::Context) -> Self::Result {
        let Some(selected_strategy) = self.selected_strategy else {
            let message = match AuthStrategy::from_str(&msg.0) {
                Ok(strategy) => {
                    self.selected_strategy = Some(strategy);

                    // tell the client to go ahead with their authentication
                    irc_proto::Message {
                        tags: None,
                        prefix: None,
                        command: Command::AUTHENTICATE("+".to_string()),
                    }
                }
                Err(_) => SaslStrategyUnsupported::into_message(),
            };

            return Box::pin(futures::future::ok(AuthenticateResult::Reply(Box::new(
                message,
            ))));
        };

        // user has cancelled authentication
        if msg.0 == "*" {
            ctx.stop();
            return Box::pin(futures::future::ok(AuthenticateResult::Reply(Box::new(
                SaslAborted::into_message(),
            ))));
        }

        match selected_strategy {
            AuthStrategy::Plain => Box::pin(
                handle_plain_authentication(msg.0, self.database.clone()).map_ok(|v| {
                    v.map_or_else(
                        || AuthenticateResult::Reply(Box::new(SaslFail::into_message())),
                        |(username, user_id)| AuthenticateResult::Done(username, user_id),
                    )
                }),
            ),
        }
    }
}

/// Attempts to handle an `AUTHENTICATE` command for the `PLAIN` authentication method.
///
/// This will parse the full message, ensure that the identity is correct and compare the hashes
/// to what we have stored in the database.
///
/// This function will return the authenticated user id and username, or None if the password was
/// incorrect.
pub async fn handle_plain_authentication(
    arguments: String,
    database: sqlx::Pool<sqlx::Any>,
) -> Result<Option<(String, UserId)>, Error> {
    // TODO: this needs to deal with AUTHENTICATE spanning more than one message
    let arguments = BASE64_STANDARD
        .decode(&arguments)
        .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

    // split the PLAIN message into its respective parts
    let mut message = arguments.splitn(3, |f| *f == b'\0');
    let (Some(authorization_identity), Some(authentication_identity), Some(password)) =
        (message.next(), message.next(), message.next())
    else {
        return Err(Error::new(ErrorKind::InvalidData, "bad plain message"));
    };

    // we don't want any ambiguity here, so the two identities need to match
    if authorization_identity != authentication_identity {
        return Err(Error::new(ErrorKind::InvalidData, "identity mismatch"));
    }

    let authorization_identity = std::str::from_utf8(authentication_identity)
        .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

    // lookup the user's password based on the USER command they sent earlier
    let (user_id, password_hash) = crate::database::create_user_or_fetch_password_hash(
        &database,
        authorization_identity,
        password,
    )
    .await
    .unwrap();
    let password_hash = PasswordHash::new(&password_hash).unwrap();

    // check the user's password
    match verify_password(password, &password_hash) {
        Ok(()) => Ok(Some((authorization_identity.to_string(), UserId(user_id)))),
        Err(argon2::password_hash::Error::Password) => Ok(None),
        Err(e) => Err(Error::new(ErrorKind::InvalidData, e.to_string())),
    }
}

pub enum AuthenticateResult {
    Reply(Box<irc_proto::Message>),
    Done(String, UserId),
}

#[derive(Message)]
#[rtype(result = "Result<AuthenticateResult, std::io::Error>")]
pub struct AuthenticateMessage(pub String);
