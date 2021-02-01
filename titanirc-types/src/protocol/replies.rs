#![allow(clippy::wildcard_imports)]

use super::{commands::Command, primitives::*};
use std::fmt::Write;

/// The origin of a message that's about to be returned to the client.
#[derive(Debug)]
pub enum Source<'a> {
    User(Nick<'a>), // change Nick to whatever type nick!user@netmask is..
    Server,
}

impl<'a> From<Nick<'a>> for Source<'a> {
    fn from(other: Nick<'a>) -> Self {
        Self::User(other)
    }
}

macro_rules! define_replies {
    (
        $(
            $name:ident$(($($arg:ident$(<$($gen:tt),+>)?),*))? = $num:expr $(=> $msg:expr)?
        ),* $(,)?
    ) => {
        /// A `RPL_*` or `ERR_*` type as defined in the IRC spec.
        #[derive(Debug)]
        #[allow(clippy::pub_enum_variant_names)]
        pub enum Reply<'a> {
            $(
                $name$(($($arg$(<$($gen),+>)?),*))*,
            )*
        }

        /// Outputs the `RPL_*`/`ERR_*` type for the wire as defined in the IRC spec.
        impl std::fmt::Display for Reply<'_> {
            fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                paste::paste! {
                    match self {
                        $(Self::$name$(($([<$arg:snake>]),*))* => write!(fmt, concat!("", $($msg),*) $(, $([<$arg:snake>]),*)*)),*
                    }
                }
            }
        }

        impl Reply<'_> {
            /// The numeric code for this reply kind.
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

    RplUmodeIs(Mode<'a>) = 221 => "{}",

    ErrNoSuchNick(Nick<'a>) = 401 => "{} :No such nick/channel",
    ErrNoSuchServer(ServerName<'a>) = 402 => "{} :No such server",
    ErrNoSuchChannel(Channel<'a>) = 403 => "{} :No such channel",
    ErrCannotSendToChan(Channel<'a>) = 404 => "{} :Cannot send to channel",
    ErrTooManyChannels(Channel<'a>) = 405 => "{} :You have joined too many channels",
    ErrWasNoSuchNick(Nick<'a>) = 406 => "{} :There was no such nickname",
    ErrTooManyTargets(Target) = 407 => "{} :Duplicate recipients. No message delivered",
    ErrNoOrigin = 409 => ":No origin specified",
    ErrNoRecipient(CommandName) = 411 => ":No recipient given ({})",
    ErrNoTextToSend = 412 => ":No text to send",
    ErrNoTopLevel(Mask) = 413 => "{} :No toplevel domain specified",
    ErrWildTopLevel(Mask) = 414 => "{} :Wildcard in toplevel domain",
    ErrUnknownCommand(CommandName) = 421 => "{} :Unknown command",
    ErrNoMotd = 422 => ":MOTD File is missing",
    ErrNoAdminInfo(ServerName<'a>) = 423 => "{} :No administrative info available",
    ErrFileError(FileOp, File) = 424 => ":File error doing {} on {}",
    ErrNoNickGiven = 431 => ":No nickname given",
    ErrErroneusNick(Nick<'a>) = 432 => "{} :Erroneus nickname",
    ErrNickInUse(Nick<'a>) = 433 => "{} :Nick is already in use",
    ErrNickCollision(Nick<'a>) = 436 => "{} :Nick collision KILL",
    ErrUserNotInChannel(Nick<'a>, Channel<'a>) = 441 => "{} {} :They aren't on that channel",
    ErrNotOnChannel(Channel<'a>) = 442 => "{} :You're not on that channel",
    ErrUserOnChannel(Username<'a>, Channel<'a>) = 443 => "{} {} :is already on channel",
    ErrNoLogin(Username<'a>) = 444 => "{} :User not logged in",
    ErrSummonDisabled = 445 => ":SUMMON has been disabled",
    ErrUsersDisabled = 446 => ":USERS has been disabled",
    ErrNotRegistered = 451 => ":You have not registered",
    ErrNeedMoreParams(CommandName) = 461 => "{} :Not enough parameters",
    ErrAlreadyRegistered = 462 => ":You may not reregister",
    ErrNoPermForHost = 463 => ":Your host isn't among the privileged",
    ErrPasswdMismatch = 464 => ":Password incorrect",
    ErrYoureBannedCreep = 465 => ":You are banned from this server",
    ErrKeySet(Channel<'a>) = 467 => "{} :Channel key already set",
    ErrChannelIsFull(Channel<'a>) = 471 => "{} :Cannot join channel (+l)",
    ErrUnknownMode(Char) = 472 => "{} :is unknown mode char to me",
    ErrInviteOnlyChan(Channel<'a>) = 473 => "{} :Cannot join channel (+i)",
    ErrBannedFromChan(Channel<'a>) = 474 => "{} :Cannot join channel (+b)",
    ErrBadChannelKey(Channel<'a>) = 475 => "{} :Cannot join channel (+k)",
    ErrNoPrivileges = 481 => ":Permission Denied- You're not an IRC operator",
    ErrChanOPrivsNeeded(Channel<'a>) = 482 => "{} :You're not channel operator",
    ErrCantKillServer = 483 => ":You cant kill a server!",
    ErrNoOperHost = 491 => ":No O-lines for your host",
    ErrUmodeUnknownFlag = 501 => ":Unknown MODE flag",
    ErrUsersDontMatch = 502 => ":Cant change mode for other users",
    RplNone = 300,
    RplUserHost(UserHost) = 302 => "{}",
    RplIson(Nicks<'a>) = 303 => "{}",
    RplAway(Nick<'a>, FreeText<'a>) = 301 => "{} :{}",
    RplUnaway = 305 => ":You are no longer marked as being away",
    RplNowAway = 306 => ":You have been marked as being away",
    RplWhoisUser(Nick<'a>, Username<'a>, HostName<'a>, RealName<'a>) = 311 => "{} {} {} * :{}",
    RplWhoisServer(Nick<'a>, ServerName<'a>, ServerInfo) = 312 => "{} {} :{}",
    RplWhoisOperator(Nick<'a>) = 313 => "{} :is an IRC operator",
    RplWhoisIdle(Nick<'a>, Integer) = 317 => "{} {} :seconds idle",
    RplEndOfWhois(Nick<'a>) = 318 => "{} :End of /WHOIS list",
    RplWhoisChannels(Nick<'a>, RightsPrefixedChannels<'a>) = 319 => "{} :{}", // todo
    RplWhoWasUser(Nick<'a>, Username<'a>, HostName<'a>, RealName<'a>) = 314 => "{} {} {} * :{}",
    RplEndOfWhoWas(Nick<'a>) = 369 => "{} :End of WHOWAS",
    RplListStart = 321 => "Channel :Users  RealName",
    RplList(Channel<'a>, AmtVisible, FreeText<'a>) = 322 => "{} {} :{}",
    RplListEnd = 323 => ":End of /LIST",
    RplChannelModeIs(Channel<'a>, Mode<'a>, ModeParams) = 324 => "{} {} {}",
    RplNoTopic(Channel<'a>) = 331 => "{} :No topic is set",
    RplTopic(Channel<'a>, FreeText<'a>) = 332 => "{} :{}",
    RplInviting(Channel<'a>, Nick<'a>) = 341 => "{} {}",
    RplVersion(Version, Debuglevel, ServerName<'a>, FreeText<'a>) = 351 => "{}.{} {} :{}",
    RplWhoReply(Channel<'a>, Username<'a>, HostName<'a>, ServerName<'a>, Nick<'a>, HG, Hopcount, RealName<'a>) = 352 => "{} {} {} {} {} {}[*][@|+] :{} {}",
    RplEndOfWho(Target) = 315 => "{} :End of /WHO list",
    RplNamReply(Channel<'a>, RightsPrefixedNicks<'a>) = 353 => "{} :{}",
    RplEndOfNames(Channel<'a>) = 366 => "{} :End of /NAMES list",
    RplLinks(Mask, ServerName<'a>, Hopcount, ServerInfo) = 364 => "{} {} :{} {}",
    RplEndOfLinks(Mask) = 365 => "{} :End of /LINKS list",
    RplBanList(Channel<'a>, Banid) = 367 => "{} {}",
    RPLEndOfBanList(Channel<'a>) = 368 => "{} :End of channel ban list",
    RplInfo(String) = 371 => ":{}",
    RplEndOfInfo = 374 => ":End of /INFO list",
    RplMotdStart(ServerName<'a>) = 375 => ":- {} Message of the day -",
    RplMotd(FreeText<'a>) = 372 => ":- {}",
    RplEndOfMotd = 376 => ":End of /MOTD command",
    RplYoureOper = 381 => ":You are now an IRC operator",
    RplRehashing(ConfigFile) = 382 => "{} :Rehashing",
    RplTime = 391,
}
