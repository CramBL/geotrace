/// Elapsed microseconds from the producer's boot-clock origin.
/// Clock selection and suspend behavior are determined by the producer.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BootTime(u64);

impl BootTime {
    /// Accepts `0..=u64::MAX - 1`, rejecting the absent-reading encoding `u64::MAX`.
    pub const fn from_micros(micros: u64) -> Result<Self, BootTimeError> {
        if micros == ABSENT_BOOT_TIME_MICROS {
            Err(BootTimeError::ReservedValue)
        } else {
            Ok(Self(micros))
        }
    }

    /// Returns the exact elapsed microseconds without conversion to floating point.
    pub const fn as_micros(self) -> u64 {
        self.0
    }
}

/// A boot reading outside the range accepted by [`BootTime::from_micros`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum BootTimeError {
    /// The input is the absent-reading encoding `u64::MAX`.
    #[error("boot time u64::MAX is reserved for an absent reading")]
    ReservedValue,
}

pub(crate) const ABSENT_BOOT_TIME_MICROS: u64 = u64::MAX;
