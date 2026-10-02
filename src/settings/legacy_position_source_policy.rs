use serde::{Deserialize as _, Deserializer, Serializer};

use super::InitialPositionSourcePolicy;

pub fn serialize<S: Serializer>(
    policy: &InitialPositionSourcePolicy,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_bool(*policy == InitialPositionSourcePolicy::Ask)
}

pub fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<InitialPositionSourcePolicy, D::Error> {
    bool::deserialize(deserializer).map(|ask| {
        if ask {
            InitialPositionSourcePolicy::Ask
        } else {
            InitialPositionSourcePolicy::AutomaticallyUseUnambiguous
        }
    })
}
