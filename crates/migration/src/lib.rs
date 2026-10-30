pub use sea_orm_migration::prelude::*;

pub mod m0001_foundation;
pub mod m0002_tenancy;
pub mod m0003_identity;
pub mod m0004_tenancy_backlog;
pub mod m0005_catalog;
pub mod m0006_catalog_permissions;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_foundation::Migration),
            Box::new(m0002_tenancy::Migration),
            Box::new(m0003_identity::Migration),
            Box::new(m0004_tenancy_backlog::Migration),
            Box::new(m0005_catalog::Migration),
            Box::new(m0006_catalog_permissions::Migration),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    #[test]
    fn bootstrap_app_role_has_no_bypassrls() {
        let path = Path::new("../../docs/db/bootstrap.sql");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read bootstrap.sql")
        } else {
            fs::read_to_string("docs/db/bootstrap.sql").expect("read bootstrap.sql from workspace")
        };

        // Find the line creating pos_app
        let pos_app_line = content
            .lines()
            .find(|l| l.contains("CREATE ROLE pos_app"))
            .expect("pos_app creation exists in bootstrap.sql");

        assert!(
            pos_app_line.contains("NOBYPASSRLS") || !pos_app_line.contains("BYPASSRLS"),
            "pos_app must not have BYPASSRLS"
        );
        assert!(
            pos_app_line.contains("NOSUPERUSER") || !pos_app_line.contains("SUPERUSER"),
            "pos_app must not be superuser"
        );

        // Verify pos_jobs has BYPASSRLS for cross-tenant maintenance
        let pos_jobs_line = content
            .lines()
            .find(|l| l.contains("CREATE ROLE pos_jobs"))
            .expect("pos_jobs creation exists in bootstrap.sql");

        assert!(
            pos_jobs_line.contains("BYPASSRLS") && !pos_jobs_line.contains("NOBYPASSRLS"),
            "pos_jobs must have BYPASSRLS"
        );
    }

    #[test]
    fn app_role_cannot_delete_by_default() {
        let path = Path::new("src/m0001_foundation.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0001_foundation.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0001_foundation.rs")
                .expect("read m0001_foundation.rs from workspace")
        };

        let default_privs = content
            .lines()
            .filter(|l| l.contains("ALTER DEFAULT PRIVILEGES") && l.contains("pos_app"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            default_privs.contains("SELECT, INSERT, UPDATE"),
            "pos_app should be granted SELECT, INSERT, UPDATE on tables"
        );
        assert!(
            !default_privs.contains("GRANT ALL") && !default_privs.contains("GRANT DELETE"),
            "pos_app must never be granted DELETE by default"
        );
    }

    #[test]
    fn plan_tables_have_no_tenant_id() {
        let path = Path::new("src/m0002_tenancy.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0002_tenancy.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0002_tenancy.rs")
                .expect("read m0002_tenancy.rs from workspace")
        };

        let plan_tables = ["plans", "plan_prices", "plan_limit_types", "plan_limits"];
        for table in plan_tables {
            // Find CREATE TABLE block for this table
            let create_stmt = content
                .split("CREATE TABLE")
                .find(|block| block.contains(&format!("IF NOT EXISTS {table} ")) || block.contains(&format!(" {table} ")))
                .unwrap_or_else(|| panic!("Table {table} not found in CREATE TABLE statements"));

            let table_def = create_stmt
                .split(';')
                .next()
                .unwrap_or_else(|| panic!("Malformed statement for {table}"));

            assert!(
                !table_def.contains("tenant_id"),
                "Plan table {table} must have no tenant_id column"
            );

            assert!(
                content.contains(&format!("ALTER TABLE {table} ENABLE ROW LEVEL SECURITY;")),
                "Table {table} must enable RLS"
            );
            assert!(
                content.contains(&format!("ALTER TABLE {table} FORCE ROW LEVEL SECURITY;")),
                "Table {table} must force RLS"
            );
            let is_platform = concat!("app.", "is_platform");
            assert!(
                content.contains(&format!("CREATE POLICY {table}_admin ON {table}"))
                    && content.contains(is_platform),
                "Table {table} must have platform write policy"
            );
        }
    }

    #[test]
    fn every_tenancy_table_forces_rls() {
        let path = Path::new("src/m0002_tenancy.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0002_tenancy.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0002_tenancy.rs")
                .expect("read m0002_tenancy.rs from workspace")
        };

        let all_tables = [
            "plans",
            "plan_prices",
            "plan_limit_types",
            "plan_limits",
            "tenants",
            "tenant_settings",
            "branches",
            "subscriptions",
            "subscription_payments",
            "tenant_exports",
        ];

        for table in all_tables {
            assert!(
                content.contains(&format!("ALTER TABLE {table} ENABLE ROW LEVEL SECURITY;")),
                "Table {table} must enable RLS"
            );
            assert!(
                content.contains(&format!("ALTER TABLE {table} FORCE ROW LEVEL SECURITY;")),
                "Table {table} must force RLS"
            );
        }

        let tenant_owned_tables = [
            "tenants",
            "tenant_settings",
            "branches",
            "subscriptions",
            "subscription_payments",
            "tenant_exports",
        ];

        for table in tenant_owned_tables {
            assert!(
                content.contains(&format!("CREATE POLICY {table}_tenant ON {table}")),
                "Tenant table {table} must have tenant policy"
            );
            assert!(
                content.contains("app.current_tenant_id()"),
                "Tenant table {table} must check app.current_tenant_id()"
            );
        }
    }

    #[test]
    fn every_identity_table_forces_rls() {
        let path = Path::new("src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        let all_tables = [
            "permissions",
            "role_templates",
            "role_template_permissions",
            "platform_users",
            "users",
            "roles",
            "role_permissions",
            "user_branch_roles",
            "devices",
            "sessions",
            "auth_tokens",
            "impersonation_sessions",
        ];

        for table in all_tables {
            assert!(
                content.contains(&format!("ALTER TABLE {table} ENABLE ROW LEVEL SECURITY;")),
                "Table {table} must enable RLS"
            );
            assert!(
                content.contains(&format!("ALTER TABLE {table} FORCE ROW LEVEL SECURITY;")),
                "Table {table} must force RLS"
            );
        }

        let tenant_owned_tables = [
            "users",
            "roles",
            "role_permissions",
            "user_branch_roles",
            "devices",
            "sessions",
            "auth_tokens",
        ];

        for table in tenant_owned_tables {
            assert!(
                content.contains(&format!("CREATE POLICY {table}_tenant ON {table}")),
                "Tenant table {table} must have tenant policy"
            );
            assert!(
                content.contains("app.current_tenant_id()"),
                "Tenant table {table} must check app.current_tenant_id()"
            );
        }
    }

    #[test]
    fn user_email_is_globally_unique() {
        let path = Path::new("src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        assert!(
            content.contains("email VARCHAR(255) NOT NULL UNIQUE")
                || content.contains("email TEXT NOT NULL UNIQUE")
                || content.contains("UNIQUE (email)"),
            "users.email must be globally unique"
        );
    }

    #[test]
    fn one_owner_role_per_tenant() {
        let path = Path::new("src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        assert!(
            content.contains("CREATE UNIQUE INDEX idx_roles_one_owner ON roles (tenant_id) WHERE is_owner = true")
                || content.contains("CREATE UNIQUE INDEX idx_roles_one_owner ON roles(tenant_id) WHERE is_owner = true"),
            "roles must have partial unique index for is_owner per tenant"
        );
    }

    #[test]
    fn a_pharmacy_wide_grant_cannot_be_inserted_twice() {
        let path = Path::new("src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        assert!(
            content.contains("UNIQUE NULLS NOT DISTINCT (tenant_id, user_id, role_id, branch_id)"),
            "user_branch_roles must use UNIQUE NULLS NOT DISTINCT"
        );
    }

    #[test]
    fn device_invoice_prefix_is_unique_per_tenant() {
        let path = Path::new("src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        assert!(
            content.contains("UNIQUE (tenant_id, invoice_prefix)")
                || content.contains("UNIQUE NULLS NOT DISTINCT (tenant_id, invoice_prefix)"),
            "devices invoice prefix must be unique per tenant"
        );
    }

    #[test]
    fn the_database_refuses_a_platform_only_permission() {
        let path = Path::new("src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        assert!(
            content.contains("check_role_permission_assignable")
                && content.contains("is_tenant_assignable = true"),
            "role_permissions must have trigger or check enforcing is_tenant_assignable = true"
        );
    }

    #[test]
    fn permission_catalogue_has_no_tenant_id() {
        let path = Path::new("src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        let platform_tables = ["permissions", "role_templates", "role_template_permissions"];
        for table in platform_tables {
            let create_stmt = content
                .split("CREATE TABLE")
                .find(|block| block.contains(&format!("IF NOT EXISTS {table} ")) || block.contains(&format!(" {table} ")))
                .unwrap_or_else(|| panic!("Table {table} not found"));

            let table_def = create_stmt
                .split(';')
                .next()
                .unwrap_or_else(|| panic!("Malformed statement for {table}"));

            assert!(
                !table_def.contains("tenant_id"),
                "Platform table {table} must have no tenant_id column"
            );
        }
    }

    #[test]
    fn the_app_role_cannot_read_platform_users() {
        let path = Path::new("src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        assert!(
            content.contains("REVOKE ALL ON platform_users FROM pos_app;"),
            "pos_app must have all permissions revoked on platform_users"
        );
        assert!(
            !content.contains("GRANT SELECT ON platform_users TO pos_app")
                && !content.contains("GRANT ALL ON platform_users TO pos_app"),
            "pos_app must never be granted access to platform_users"
        );
    }

    #[test]
    fn the_login_lookup_returns_four_columns() {
        let path = Path::new("src/m0003_identity.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0003_identity.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0003_identity.rs")
                .expect("read m0003_identity.rs from workspace")
        };

        let fn_def = content
            .split("CREATE OR REPLACE FUNCTION lookup_user_for_login")
            .nth(1)
            .expect("lookup_user_for_login function defined");

        let returns_table = fn_def
            .split("RETURNS TABLE")
            .nth(1)
            .expect("RETURNS TABLE block")
            .split("SECURITY DEFINER")
            .next()
            .expect("column list")
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')');

        let cols: Vec<&str> = returns_table
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        assert_eq!(
            cols.len(),
            4,
            "lookup_user_for_login must return exactly four columns, got: {cols:?}"
        );
        assert!(cols.iter().any(|c| c.starts_with("id ")));
        assert!(cols.iter().any(|c| c.starts_with("tenant_id ")));
        assert!(cols.iter().any(|c| c.starts_with("password_hash ")));
        assert!(cols.iter().any(|c| c.starts_with("status ")));
    }

    #[test]
    fn no_catalog_table_carries_a_tenant_id() {
        let path = Path::new("src/m0005_catalog.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0005_catalog.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0005_catalog.rs")
                .expect("read m0005_catalog.rs from workspace")
        };

        let shared_tables = [
            "pack_units",
            "dosage_forms",
            "ingredients",
            "therapeutic_classes",
            "companies",
            "generics",
            "generic_monographs",
            "generic_ingredients",
            "generic_therapeutic_classes",
            "products",
            "product_packs",
        ];

        for table in shared_tables {
            let create_stmt = content
                .split("CREATE TABLE")
                .find(|block| {
                    block.contains(&format!("IF NOT EXISTS {table} "))
                        || block.contains(&format!(" {table} "))
                })
                .unwrap_or_else(|| panic!("Table {table} not found in CREATE TABLE statements"));

            let table_def = create_stmt
                .split(';')
                .next()
                .unwrap_or_else(|| panic!("Malformed statement for {table}"));

            let has_direct_tenant_id = table_def.lines().any(|l| {
                let trimmed = l.trim();
                trimmed.starts_with("tenant_id ") || trimmed.starts_with("tenant_id,")
            });

            assert!(
                !has_direct_tenant_id,
                "Catalogue shared table {table} must have no tenant_id column"
            );
        }

        // catalog_issue_reports is the single exception and MUST have tenant_id
        let issue_stmt = content
            .split("CREATE TABLE")
            .find(|block| {
                block.contains("IF NOT EXISTS catalog_issue_reports ")
                    || block.contains(" catalog_issue_reports ")
            })
            .expect("catalog_issue_reports exists");
        let issue_def = issue_stmt.split(';').next().expect("issue report def");
        assert!(
            issue_def.lines().any(|l| l.trim().starts_with("tenant_id ")),
            "catalog_issue_reports must have tenant_id column"
        );
    }

    #[test]
    fn every_catalog_table_forces_rls() {
        let path = Path::new("src/m0005_catalog.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0005_catalog.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0005_catalog.rs")
                .expect("read m0005_catalog.rs from workspace")
        };

        let all_tables = [
            "pack_units",
            "dosage_forms",
            "ingredients",
            "therapeutic_classes",
            "companies",
            "generics",
            "generic_monographs",
            "generic_ingredients",
            "generic_therapeutic_classes",
            "products",
            "product_packs",
            "catalog_issue_reports",
        ];

        for table in all_tables {
            assert!(
                content.contains(&format!("ALTER TABLE {table} ENABLE ROW LEVEL SECURITY;")),
                "Table {table} must enable RLS"
            );
            assert!(
                content.contains(&format!("ALTER TABLE {table} FORCE ROW LEVEL SECURITY;")),
                "Table {table} must force RLS"
            );
        }

        assert!(
            !content.contains("FOR ALL"),
            "All policies in catalog must be per command rather than one FOR ALL"
        );
    }

    #[test]
    fn therapeutic_class_hangs_off_the_generic() {
        let path = Path::new("src/m0005_catalog.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0005_catalog.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0005_catalog.rs")
                .expect("read m0005_catalog.rs from workspace")
        };

        // Table generic_therapeutic_classes connects generic_id and therapeutic_class_id
        assert!(
            content.contains("CREATE TABLE IF NOT EXISTS generic_therapeutic_classes"),
            "generic_therapeutic_classes table must exist"
        );
        let gtc_stmt = content
            .split("CREATE TABLE")
            .find(|block| block.contains("generic_therapeutic_classes"))
            .expect("gtc stmt");
        assert!(
            gtc_stmt.contains("generic_id UUID NOT NULL REFERENCES generics(id)"),
            "gtc must reference generics(id)"
        );
        assert!(
            gtc_stmt.contains("therapeutic_class_id UUID NOT NULL REFERENCES therapeutic_classes(id)"),
            "gtc must reference therapeutic_classes(id)"
        );

        // Products table must NOT have therapeutic_class_id
        let prod_stmt = content
            .split("CREATE TABLE")
            .find(|block| {
                block.contains("IF NOT EXISTS products ") || block.contains(" products ")
            })
            .expect("products table exists");
        let prod_def = prod_stmt.split(';').next().expect("products def");
        assert!(
            !prod_def.contains("therapeutic_class"),
            "products must not have therapeutic class column"
        );
        assert!(
            !content.contains("product_therapeutic_classes"),
            "no product-level therapeutic class table may exist"
        );
    }

    #[test]
    fn catalog_has_both_trigram_indexes() {
        let path = Path::new("src/m0005_catalog.rs");
        let content = if path.exists() {
            fs::read_to_string(path).expect("read m0005_catalog.rs")
        } else {
            fs::read_to_string("crates/migration/src/m0005_catalog.rs")
                .expect("read m0005_catalog.rs from workspace")
        };

        assert!(
            content.contains("CREATE INDEX IF NOT EXISTS idx_products_brand_name_trgm ON products USING gin (brand_name gin_trgm_ops);")
                || content.contains("ON products USING gin (brand_name gin_trgm_ops)"),
            "products.brand_name must have GIN trgm index"
        );
        assert!(
            content.contains("CREATE INDEX IF NOT EXISTS idx_generics_name_trgm ON generics USING gin (name gin_trgm_ops);")
                || content.contains("ON generics USING gin (name gin_trgm_ops)"),
            "generics.name must have GIN trgm index"
        );
    }
}
