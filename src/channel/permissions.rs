use std::cmp::Ordering;

use anyhow::anyhow;
use irc_proto::{ChannelMode, Mode};

#[derive(Copy, Clone, Debug, Eq, PartialEq, sqlx::Type)]
#[repr(i16)]
pub enum Permission {
    Ban = -1,
    Normal = 0,
    Voice = 1,
    HalfOperator = i16::MAX - 2,
    Operator = i16::MAX - 1,
    Founder = i16::MAX,
}

impl PartialOrd for Permission {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Permission {
    fn cmp(&self, other: &Self) -> Ordering {
        // order `ban` ahead of `normal`
        match (*self as i16, *other as i16) {
            (-1, 0) => Ordering::Less,
            (0, -1) => Ordering::Greater,
            _ => (*self as i16).cmp(&(*other as i16)),
        }
    }
}

impl TryFrom<ChannelMode> for Permission {
    type Error = anyhow::Error;

    fn try_from(value: ChannelMode) -> Result<Self, Self::Error> {
        match value {
            ChannelMode::Ban => Ok(Self::Ban),
            ChannelMode::Voice => Ok(Self::Voice),
            ChannelMode::Halfop => Ok(Self::HalfOperator),
            ChannelMode::Oper => Ok(Self::Operator),
            ChannelMode::Founder => Ok(Self::Founder),
            _ => Err(anyhow!("unknown user access level: {value:?}")),
        }
    }
}

impl Permission {
    /// A list of (mode)prefix used to inform clients of which modes set which prefixes.
    pub const SUPPORTED_PREFIXES: &'static str = "(qohv)~@%+";

    /// Builds the mode message that's used to set (or unset) this permission.
    #[must_use]
    pub fn into_mode(self, add: bool, mask: String) -> Option<Mode<ChannelMode>> {
        <Option<ChannelMode>>::from(self).map(|v| {
            if add {
                Mode::Plus(v, Some(mask))
            } else {
                Mode::Minus(v, Some(mask))
            }
        })
    }

    /// Grabs the prefix that is used to represent a permission.
    #[must_use]
    pub const fn into_prefix(self) -> &'static str {
        match self {
            Self::Ban | Self::Normal => "",
            Self::Voice => "+",
            Self::HalfOperator => "%",
            Self::Operator => "@",
            Self::Founder => "~",
        }
    }
}

impl From<Permission> for Option<ChannelMode> {
    fn from(value: Permission) -> Self {
        match value {
            Permission::Ban => Some(ChannelMode::Ban),
            Permission::Normal => None,
            Permission::Voice => Some(ChannelMode::Voice),
            Permission::HalfOperator => Some(ChannelMode::Halfop),
            Permission::Operator => Some(ChannelMode::Oper),
            Permission::Founder => Some(ChannelMode::Founder),
        }
    }
}

impl Permission {
    /// Returns true, if the user is allowed to chat in the channel.
    #[must_use]
    pub fn can_chatter(self) -> bool {
        self != Self::Ban
    }

    /// Returns true, if the user is allowed to join the channel.
    #[must_use]
    pub fn can_join(self) -> bool {
        self != Self::Ban
    }

    /// Returns true, if the user is allowed to set the channel topic.
    #[must_use]
    pub const fn can_set_topic(self) -> bool {
        (self as i16) >= (Self::HalfOperator as i16)
    }

    /// Returns true, if the user is allowed to kick people from the channel.
    #[must_use]
    pub const fn can_kick(self) -> bool {
        (self as i16) >= (Self::HalfOperator as i16)
    }

    /// Returns true, if the user is allowed to set the given permission on another
    /// user.
    #[must_use]
    pub const fn can_set_permission(self, new: Self, old: Self) -> bool {
        (self as i16) >= (Self::HalfOperator as i16)
            && (self as i16) > (new as i16)
            && (self as i16) > (old as i16)
    }
}
