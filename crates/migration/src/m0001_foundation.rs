use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            r#"
            CREATE SCHEMA IF NOT EXISTS app;

            CREATE OR REPLACE FUNCTION app.current_tenant_id() RETURNS uuid AS $$
                SELECT NULLIF(current_setting('app.tenant_id', true), '')::uuid;
            $$ LANGUAGE sql STABLE;

            CREATE OR REPLACE FUNCTION app.current_user_id() RETURNS uuid AS $$
                SELECT NULLIF(current_setting('app.user_id', true), '')::uuid;
            $$ LANGUAGE sql STABLE;

            -- Default grants: pos_app gets SELECT, INSERT, UPDATE on future tables in public, never DELETE
            GRANT USAGE ON SCHEMA public TO pos_app;
            GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO pos_app;
            ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO pos_app;
            ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE ON TABLES TO pos_app;
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            r#"
            ALTER DEFAULT PRIVILEGES IN SCHEMA public REVOKE SELECT, INSERT, UPDATE ON TABLES FROM pos_app;
            DROP FUNCTION IF EXISTS app.current_user_id();
            DROP FUNCTION IF EXISTS app.current_tenant_id();
            "#,
        )
        .await?;

        Ok(())
    }
}
