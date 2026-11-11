use chrono::Duration;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssociationWindow(Duration);

impl AssociationWindow {
    pub const MAX: Self = Self(Duration::days(365));
    pub const ZERO: Self = Self(Duration::zero());
}

impl Serialize for AssociationWindow {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let nanoseconds = self
            .0
            .num_nanoseconds()
            .ok_or_else(|| serde::ser::Error::custom(InvalidAssociationWindow))?;
        serializer.serialize_i64(nanoseconds)
    }
}

impl<'de> Deserialize<'de> for AssociationWindow {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let nanoseconds = i64::deserialize(deserializer)?;
        Self::try_from(Duration::nanoseconds(nanoseconds)).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<Duration> for AssociationWindow {
    type Error = InvalidAssociationWindow;

    fn try_from(duration: Duration) -> Result<Self, Self::Error> {
        if duration < Self::ZERO.0 || duration > Self::MAX.0 {
            return Err(InvalidAssociationWindow);
        }
        Ok(Self(duration))
    }
}

impl From<AssociationWindow> for Duration {
    fn from(window: AssociationWindow) -> Self {
        window.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("association window must be between zero and 365 days")]
pub struct InvalidAssociationWindow;

#[cfg(test)]
mod tests;
