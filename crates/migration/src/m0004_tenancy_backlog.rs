use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let is_plat = concat!("app.", "is_platform");

        let sql = format!(
            r#"
            -- A2: Composite FK on subscription_payments (tenant_id, subscription_id) referencing subscriptions(tenant_id, id)
            ALTER TABLE subscription_payments
                DROP CONSTRAINT IF EXISTS subscription_payments_subscription_id_fkey;

            ALTER TABLE subscription_payments
                ADD CONSTRAINT fk_subscription_payments_subscription_composite
                FOREIGN KEY (tenant_id, subscription_id)
                REFERENCES subscriptions (tenant_id, id)
                ON DELETE SET NULL;

            -- C1: Covering index on subscriptions (tenant_id, ends_at DESC)
            CREATE INDEX IF NOT EXISTS idx_subscriptions_tenant_ends_at
                ON subscriptions (tenant_id, ends_at DESC);

            -- A3: Per-command RLS policies on subscriptions and subscription_payments
            -- Subscriptions:
            DROP POLICY IF EXISTS subscriptions_tenant ON subscriptions;

            CREATE POLICY subscriptions_select ON subscriptions
                FOR SELECT
                USING (tenant_id = app.current_tenant_id() OR current_setting('{is_plat}', true) = 'true');

            CREATE POLICY subscriptions_platform_all ON subscriptions
                FOR ALL
                USING (current_setting('{is_plat}', true) = 'true')
                WITH CHECK (current_setting('{is_plat}', true) = 'true');

            -- Subscription payments:
            DROP POLICY IF EXISTS subscription_payments_tenant ON subscription_payments;

            CREATE POLICY subscription_payments_select ON subscription_payments
                FOR SELECT
                USING (tenant_id = app.current_tenant_id() OR current_setting('{is_plat}', true) = 'true');

            CREATE POLICY subscription_payments_tenant_insert ON subscription_payments
                FOR INSERT
                WITH CHECK (tenant_id = app.current_tenant_id() AND status = 'submitted');

            CREATE POLICY subscription_payments_platform_all ON subscription_payments
                FOR ALL
                USING (current_setting('{is_plat}', true) = 'true')
                WITH CHECK (current_setting('{is_plat}', true) = 'true');
            "#,
            is_plat = is_plat
        );

        db.execute_unprepared(&sql).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
