pub use sea_orm_migration::prelude::*;

pub mod m0001_foundation;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m0001_foundation::Migration)]
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
}
