use anyhow::anyhow;
use irc_proto::ChannelMode;

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
    pub fn can_kick(self) -> bool {
        self == Self::Operator
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
