#[derive(Copy, Clone, Debug, Eq, PartialEq, sqlx::Type)]
#[repr(i16)]
pub enum Permission {
    Ban = -1,
    Normal = 0,
    Voice = 1,
    HalfOperator = 2,
    Operator = i16::MAX,
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
}
