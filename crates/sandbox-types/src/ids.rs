use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use ulid::Ulid;

macro_rules! ulid_newtype {
    ($name:ident) => {
        #[derive(
            Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default
        )]
        #[serde(transparent)]
        pub struct $name(pub Ulid);

        impl $name {
            pub fn new() -> Self {
                Self(Ulid::new())
            }

            pub fn from_ulid(ulid: Ulid) -> Self {
                Self(ulid)
            }

            pub fn as_ulid(&self) -> Ulid {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = <Ulid as FromStr>::Err;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Ulid::from_str(s)?))
            }
        }

        impl From<Ulid> for $name {
            fn from(ulid: Ulid) -> Self {
                Self(ulid)
            }
        }

        impl From<$name> for Ulid {
            fn from(id: $name) -> Ulid {
                id.0
            }
        }
    };
}

ulid_newtype!(SessionId);
ulid_newtype!(SnapshotId);
ulid_newtype!(TenantId);

impl SessionId {
    pub const fn nil() -> Self {
        Self(Ulid::nil())
    }
}

impl SnapshotId {
    pub const fn nil() -> Self {
        Self(Ulid::nil())
    }
}

impl TenantId {
    pub const fn nil() -> Self {
        Self(Ulid::nil())
    }
}