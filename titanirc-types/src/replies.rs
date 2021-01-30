#![allow(clippy::wildcard_imports)]

use crate::{primitives::*, Command};
use std::fmt::Write;

#[derive(Debug)]
pub enum Source {
    User(Nick), // change Nick to whatever type nick!user@netmask is..
    Server,
}

#[derive(Debug, derive_more::From)]
pub enum ServerMessage {
    Reply(Reply),
    Command(Source, Command), // change Nick to whatever type nick!user@netmask is..
    Ping,
    Pong,
}

impl ServerMessage {
    pub fn write(self, server_name: &str, client_username: &str, dst: &mut bytes::BytesMut) {
        match self {
            Self::Reply(reply) => write!(
                dst,
                ":{} {} {} {}",
                server_name,
                reply.code(),
                client_username,
                reply,
            ),
            Self::Ping => write!(dst, "PING :{}", server_name),
            Self::Pong => write!(dst, "PONG :{}", server_name),
            Self::Command(source, command) => {
                let source = match &source {
                    Source::User(nick) => std::str::from_utf8(nick).unwrap(),
                    Source::Server => server_name,
                };
                write!(dst, ":{} {}", source, command)
            }
        }
        .unwrap()
    }
}

macro_rules! define_replies {
    (
        $(
            $name:ident$(($($arg:ty),*))? = $num:expr $(=> $msg:expr)?
        ),* $(,)?
    ) => {
        #[derive(Debug)]
        #[allow(clippy::pub_enum_variant_names)]
        pub enum Reply {
            $(
                $name$(($($arg),*))*,
            )*
        }

        impl std::fmt::Display for Reply {
            fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                paste::paste! {
                    match self {
                        $(Self::$name$(($([<$arg:snake>]),*))* => write!(fmt, concat!("", $($msg),*) $(, $([<$arg:snake>]),*)*)),*
                    }
                }
            }
        }

        impl Reply {
            #[must_use]
            pub fn code(&self) -> &'static str {
                paste::paste! {
                    match self {
                        $(Self::$name$(($([<_ $arg:snake>]),*))* => stringify!($num)),*
                    }
                }
            }
        }
    };
}

type Target = String;
type CommandName = String;
type Mask = String;
type Banid = String;
type ConfigFile = String;
type Hopcount = String;
type ServerInfo = String;
type HG = String;
type Debuglevel = String;
type ModeParams = String;
type Version = String;
type AmtVisible = String;
type Integer = String;
type File = String;
type FileOp = String;
type Char = String;

// TODO: fix these
type UserHost = String;

define_replies! {
    RplWelcome = 001 => ":Welcome to the network jordan!jordan@proper.sick.kid",
    RplYourHost = 002 => ":Your host is a sick kid",
    RplCreated = 003 => ":This server was created at some point",
    RplMyInfo = 004 => ":my.test.server 0.0.1 DOQRSZaghilopsuwz CFILMPQSbcefgijklmnopqrstuvz bkloveqjfI",
    RplISupport = 005 => "D :are supported by this server",

    RplUmodeIs(Mode) = 221 => "{}",

    ErrNoSuchNick(Nick) = 401 => "{} :No such nick/channel",
    ErrNoSuchServer(ServerName) = 402 => "{} :No such server",
    ErrNoSuchChannel(Channel) = 403 => "{} :No such channel",
    ErrCannotSendToChan(Channel) = 404 => "{} :Cannot send to channel",
    ErrTooManyChannels(Channel) = 405 => "{} :You have joined too many channels",
    ErrWasNoSuchNick(Nick) = 406 => "{} :There was no such nickname",
    ErrTooManyTargets(Target) = 407 => "{} :Duplicate recipients. No message delivered",
    ErrNoOrigin = 409 => ":No origin specified",
    ErrNoRecipient(CommandName) = 411 => ":No recipient given ({})",
    ErrNoTextToSend = 412 => ":No text to send",
    ErrNoTopLevel(Mask) = 413 => "{} :No toplevel domain specified",
    ErrWildTopLevel(Mask) = 414 => "{} :Wildcard in toplevel domain",
    ErrUnknownCommand(CommandName) = 421 => "{} :Unknown command",
    ErrNoMotd = 422 => ":MOTD File is missing",
    ErrNoAdminInfo(ServerName) = 423 => "{} :No administrative info available",
    ErrFileError(FileOp, File) = 424 => ":File error doing {} on {}",
    ErrNoNickGiven = 431 => ":No nickname given",
    ErrErroneusNick(Nick) = 432 => "{} :Erroneus nickname",
    ErrNickInUse(Nick) = 433 => "{} :Nick is already in use",
    ErrNickCollision(Nick) = 436 => "{} :Nick collision KILL",
    ErrUserNotInChannel(Nick, Channel) = 441 => "{} {} :They aren't on that channel",
    ErrNotOnChannel(Channel) = 442 => "{} :You're not on that channel",
    ErrUserOnChannel(Username, Channel) = 443 => "{} {} :is already on channel",
    ErrNoLogin(Username) = 444 => "{} :User not logged in",
    ErrSummonDisabled = 445 => ":SUMMON has been disabled",
    ErrUsersDisabled = 446 => ":USERS has been disabled",
    ErrNotRegistered = 451 => ":You have not registered",
    ErrNeedMoreParams(CommandName) = 461 => "{} :Not enough parameters",
    ErrAlreadyRegistered = 462 => ":You may not reregister",
    ErrNoPermForHost = 463 => ":Your host isn't among the privileged",
    ErrPasswdMismatch = 464 => ":Password incorrect",
    ErrYoureBannedCreep = 465 => ":You are banned from this server",
    ErrKeySet(Channel) = 467 => "{} :Channel key already set",
    ErrChannelIsFull(Channel) = 471 => "{} :Cannot join channel (+l)",
    ErrUnknownMode(Char) = 472 => "{} :is unknown mode char to me",
    ErrInviteOnlyChan(Channel) = 473 => "{} :Cannot join channel (+i)",
    ErrBannedFromChan(Channel) = 474 => "{} :Cannot join channel (+b)",
    ErrBadChannelKey(Channel) = 475 => "{} :Cannot join channel (+k)",
    ErrNoPrivileges = 481 => ":Permission Denied- You're not an IRC operator",
    ErrChanOPrivsNeeded(Channel) = 482 => "{} :You're not channel operator",
    ErrCantKillServer = 483 => ":You cant kill a server!",
    ErrNoOperHost = 491 => ":No O-lines for your host",
    ErrUmodeUnknownFlag = 501 => ":Unknown MODE flag",
    ErrUsersDontMatch = 502 => ":Cant change mode for other users",
    RplNone = 300,
    RplUserHost(UserHost) = 302 => "{}",
    RplIson(Nicks) = 303 => "{}",
    RplAway(Nick, FreeText) = 301 => "{} :{}",
    RplUnaway = 305 => ":You are no longer marked as being away",
    RplNowAway = 306 => ":You have been marked as being away",
    RplWhoisUser(Nick, Username, HostName, RealName) = 311 => "{} {} {} * :{}",
    RplWhoisServer(Nick, ServerName, ServerInfo) = 312 => "{} {} :{}",
    RplWhoisOperator(Nick) = 313 => "{} :is an IRC operator",
    RplWhoisIdle(Nick, Integer) = 317 => "{} {} :seconds idle",
    RplEndOfWhois(Nick) = 318 => "{} :End of /WHOIS list",
    RplWhoisChannels(Nick, RightsPrefixedChannels) = 319 => "{} :{}", // todo
    RplWhoWasUser(Nick, Username, HostName, RealName) = 314 => "{} {} {} * :{}",
    RplEndOfWhoWas(Nick) = 369 => "{} :End of WHOWAS",
    RplListStart = 321 => "Channel :Users  RealName",
    RplList(Channel, AmtVisible, FreeText) = 322 => "{} {} :{}",
    RplListEnd = 323 => ":End of /LIST",
    RplChannelModeIs(Channel, Mode, ModeParams) = 324 => "{} {} {}",
    RplNoTopic(Channel) = 331 => "{} :No topic is set",
    RplTopic(Channel, FreeText) = 332 => "{} :{}",
    RplInviting(Channel, Nick) = 341 => "{} {}",
    RplVersion(Version, Debuglevel, ServerName, FreeText) = 351 => "{}.{} {} :{}",
    RplWhoReply(Channel, Username, HostName, ServerName, Nick, HG, Hopcount, RealName) = 352 => "{} {} {} {} {} {}[*][@|+] :{} {}",
    RplEndOfWho(Target) = 315 => "{} :End of /WHO list",
    RplNamReply(Channel, RightsPrefixedNicks) = 353 => "{} :{}",
    RplEndOfNames(Channel) = 366 => "{} :End of /NAMES list",
    RplLinks(Mask, ServerName, Hopcount, ServerInfo) = 364 => "{} {} :{} {}",
    RplEndOfLinks(Mask) = 365 => "{} :End of /LINKS list",
    RplBanList(Channel, Banid) = 367 => "{} {}",
    RPLEndOfBanList(Channel) = 368 => "{} :End of channel ban list",
    RplInfo(String) = 371 => ":{}",
    RplEndOfInfo = 374 => ":End of /INFO list",
    RplMotdStart(ServerName) = 375 => ":- {} Message of the day -",
    RplMotd(FreeText) = 372 => ":- {}",
    RplEndOfMotd = 376 => ":End of /MOTD command",
    RplYoureOper = 381 => ":You are now an IRC operator",
    RplRehashing(ConfigFile) = 382 => "{} :Rehashing",
    RplTime = 391,
}
