use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! define_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[repr(transparent)]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new_v7() -> Self {
                Self(Uuid::now_v7())
            }

            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            #[must_use]
            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

define_id!(TenantId, "Unique identifier for a tenant pharmacy.");
define_id!(BranchId, "Unique identifier for a pharmacy branch.");
define_id!(UserId, "Unique identifier for an authenticated user.");
define_id!(DeviceId, "Unique identifier for a POS device.");
define_id!(RoleId, "Unique identifier for a role.");
define_id!(SessionId, "Unique identifier for an authenticated session.");

define_id!(CompanyId, "Unique identifier for a pharmaceutical company/manufacturer.");
define_id!(GenericId, "Unique identifier for an active ingredient generic.");
define_id!(ProductId, "Unique identifier for a medicine or non-medicine product.");
define_id!(IngredientId, "Unique identifier for a chemical ingredient molecule.");
define_id!(TherapeuticClassId, "Unique identifier for a therapeutic class.");
define_id!(DosageFormId, "Unique identifier for a dosage form.");
define_id!(PackUnitId, "Unique identifier for a packaging unit.");
define_id!(IssueReportId, "Unique identifier for a catalogue issue report.");

#[repr(transparent)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(String);

impl RequestId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7().to_string())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_uuid_v7() {
        let tenant_id = TenantId::new_v7();
        let branch_id = BranchId::new_v7();
        let user_id = UserId::new_v7();
        let device_id = DeviceId::new_v7();

        assert_eq!(tenant_id.as_uuid().get_version_num(), 7);
        assert_eq!(branch_id.as_uuid().get_version_num(), 7);
        assert_eq!(user_id.as_uuid().get_version_num(), 7);
        assert_eq!(device_id.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn the_new_identity_ids_are_uuid_v7() {
        let role_id = RoleId::new_v7();
        let device_id = DeviceId::new_v7();
        let session_id = SessionId::new_v7();

        assert_eq!(role_id.as_uuid().get_version_num(), 7);
        assert_eq!(device_id.as_uuid().get_version_num(), 7);
        assert_eq!(session_id.as_uuid().get_version_num(), 7);
    }
}
