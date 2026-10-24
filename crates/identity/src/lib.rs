mod entity;
pub mod commands;
pub mod login;
pub mod password;
pub mod platform;
pub mod queries;
pub mod rate_limit;
pub mod revocation;
pub mod roles;
pub mod rest;
pub mod settings;
pub mod tokens;
pub mod types;

pub use settings::TokenSettings;
pub use types::*;

pub const PERMISSIONS: &[&str] = &[
    "user.view",
    "user.create",
    "user.update",
    "user.disable",
    "user.reset_password",
    "role.view",
    "role.create",
    "role.update",
    "role.delete",
    "role.assign",
    "device.view",
    "device.register",
    "device.revoke",
    "session.view",
    "session.revoke",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn a_secret_cannot_be_serialized_or_printed() {
        let secret = Secret::new("my-secret-password-123".to_string());
        let debug_output = format!("{secret:?}");
        assert_eq!(debug_output, "[redacted]");
        assert!(!debug_output.contains("my-secret-password-123"));
    }

    #[test]
    fn every_declared_permission_slug_has_a_row() {
        let path = Path::new("../migration/src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        // All identity permissions
        for slug in PERMISSIONS {
            assert!(
                content.contains(&format!("'{slug}'")),
                "Permission slug '{slug}' must exist as a seeded row in m0003_identity"
            );
        }

        // All tenancy permissions
        let tenancy_slugs = [
            "tenant.view",
            "tenant.update",
            "tenant.settings.update",
            "branch.view",
            "branch.create",
            "branch.update",
            "subscription.view",
            "subscription.pay",
            "tenant.export",
            "tenant.close",
        ];
        for slug in tenancy_slugs {
            assert!(
                content.contains(&format!("'{slug}'")),
                "Tenancy permission slug '{slug}' must exist as a seeded row in m0003_identity"
            );
        }
    }

    #[test]
    fn the_owner_template_holds_every_assignable_permission() {
        let path = Path::new("../migration/src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        assert!(
            content.contains("INSERT INTO role_template_permissions (template_code, permission_code)")
                && content.contains("SELECT 'owner', code")
                && content.contains("WHERE is_tenant_assignable = true"),
            "Owner template must hold every tenant-assignable permission via seed query"
        );
    }
}
