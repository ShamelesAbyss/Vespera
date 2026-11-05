use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use uuid::Uuid;

use crate::DomainError;

macro_rules! define_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[doc = "\nThe UUID is never nil. Serialization uses a canonical UUID string."]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Generate an independent version-4 UUID identity.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Confirm the nonnil invariant; parsing already enforces it.
            pub fn validate(&self) -> Result<(), DomainError> {
                if self.0.is_nil() {
                    Err(DomainError::InvalidId {
                        kind: stringify!($name),
                        value: self.to_string(),
                        reason: "nil UUID is not an identity".into(),
                    })
                } else {
                    Ok(())
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let uuid = Uuid::parse_str(value).map_err(|error| DomainError::InvalidId {
                    kind: stringify!($name),
                    value: value.into(),
                    reason: error.to_string(),
                })?;
                let id = Self(uuid);
                id.validate()?;
                Ok(id)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(de::Error::custom)
            }
        }
    };
}

define_id!(
    AgentId,
    "Model-independent agent identity; also determines its memory namespace."
);
define_id!(
    ModelId,
    "Identity of a registered model, separate from provider and agent identities."
);
define_id!(
    ProviderId,
    "Identity of a provider configuration; does not establish a connection."
);
define_id!(
    TaskId,
    "Identity of a task, independent of its execution state."
);
define_id!(
    EventId,
    "Identity of an event envelope; uniqueness does not prove an event occurred."
);
define_id!(
    MemoryId,
    "Identity of a memory record; creating one does not persist anything."
);
