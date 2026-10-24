use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            r#"
            -- 1. Prerequisite extension for exclusion constraints
            CREATE EXTENSION IF NOT EXISTS btree_gist;

            -- 2. Platform tables (no tenant_id)
            CREATE TABLE IF NOT EXISTS plans (
                id UUID PRIMARY KEY,
                code VARCHAR(64) NOT NULL UNIQUE,
                name VARCHAR(255) NOT NULL,
                grace_days INTEGER NOT NULL DEFAULT 7,
                is_active BOOLEAN NOT NULL DEFAULT true,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS plan_prices (
                plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
                billing_period VARCHAR(32) NOT NULL,
                price NUMERIC(12,2) NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (plan_id, billing_period)
            );

            CREATE TABLE IF NOT EXISTS plan_limit_types (
                key VARCHAR(64) PRIMARY KEY,
                description TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS plan_limits (
                plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
                limit_key VARCHAR(64) NOT NULL REFERENCES plan_limit_types(key) ON DELETE CASCADE,
                max_value INTEGER NULL,
                PRIMARY KEY (plan_id, limit_key)
            );

            -- RLS for platform tables: readable by all authenticated, writable only with app.is_platform = 'true'
            ALTER TABLE plans ENABLE ROW LEVEL SECURITY;
            ALTER TABLE plans FORCE ROW LEVEL SECURITY;
            CREATE POLICY plans_select ON plans FOR SELECT USING (true);
            CREATE POLICY plans_admin ON plans FOR ALL
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            ALTER TABLE plan_prices ENABLE ROW LEVEL SECURITY;
            ALTER TABLE plan_prices FORCE ROW LEVEL SECURITY;
            CREATE POLICY plan_prices_select ON plan_prices FOR SELECT USING (true);
            CREATE POLICY plan_prices_admin ON plan_prices FOR ALL
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            ALTER TABLE plan_limit_types ENABLE ROW LEVEL SECURITY;
            ALTER TABLE plan_limit_types FORCE ROW LEVEL SECURITY;
            CREATE POLICY plan_limit_types_select ON plan_limit_types FOR SELECT USING (true);
            CREATE POLICY plan_limit_types_admin ON plan_limit_types FOR ALL
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            ALTER TABLE plan_limits ENABLE ROW LEVEL SECURITY;
            ALTER TABLE plan_limits FORCE ROW LEVEL SECURITY;
            CREATE POLICY plan_limits_select ON plan_limits FOR SELECT USING (true);
            CREATE POLICY plan_limits_admin ON plan_limits FOR ALL
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            -- 3. Tenant tables
            CREATE TABLE IF NOT EXISTS tenants (
                id UUID PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                legal_name VARCHAR(255),
                vat_bin VARCHAR(64),
                phone VARCHAR(32),
                email VARCHAR(255),
                status VARCHAR(32) NOT NULL DEFAULT 'pending_approval'
                    CHECK (status IN ('pending_approval', 'active', 'suspended', 'closing')),
                suspension_reason TEXT,
                approved_by_platform_user_id UUID,
                approved_at TIMESTAMPTZ,
                deletion_requested_at TIMESTAMPTZ,
                purge_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT chk_tenants_suspension_reason CHECK (status <> 'suspended' OR suspension_reason IS NOT NULL)
            );

            ALTER TABLE tenants ENABLE ROW LEVEL SECURITY;
            ALTER TABLE tenants FORCE ROW LEVEL SECURITY;
            CREATE POLICY tenants_tenant ON tenants
                USING (id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            CREATE TABLE IF NOT EXISTS tenant_settings (
                tenant_id UUID PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
                vat_rate NUMERIC(5,2) NOT NULL DEFAULT 0.00,
                vat_included BOOLEAN NOT NULL DEFAULT false,
                rounding_mode VARCHAR(32) NOT NULL DEFAULT 'nearest_half',
                near_expiry_days INTEGER NOT NULL DEFAULT 90,
                default_payment_terms_days INTEGER NOT NULL DEFAULT 30,
                allow_zero_stock_sale BOOLEAN NOT NULL DEFAULT false,
                receipt_header TEXT,
                receipt_footer TEXT,
                receipt_paper_width INTEGER NOT NULL DEFAULT 80,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            ALTER TABLE tenant_settings ENABLE ROW LEVEL SECURITY;
            ALTER TABLE tenant_settings FORCE ROW LEVEL SECURITY;
            CREATE POLICY tenant_settings_tenant ON tenant_settings
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            CREATE TABLE IF NOT EXISTS branches (
                id UUID PRIMARY KEY,
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                name VARCHAR(255) NOT NULL,
                address TEXT,
                phone VARCHAR(32),
                drug_license_no VARCHAR(64),
                is_default BOOLEAN NOT NULL DEFAULT false,
                is_active BOOLEAN NOT NULL DEFAULT true,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT uq_branches_tenant_id_id UNIQUE (tenant_id, id),
                CONSTRAINT uq_branches_tenant_id_name UNIQUE (tenant_id, name)
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_branches_tenant_default ON branches (tenant_id) WHERE is_default;
            CREATE INDEX IF NOT EXISTS idx_branches_tenant_active ON branches (tenant_id, is_active);

            ALTER TABLE branches ENABLE ROW LEVEL SECURITY;
            ALTER TABLE branches FORCE ROW LEVEL SECURITY;
            CREATE POLICY branches_tenant ON branches
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            -- 4. Subscriptions
            CREATE TABLE IF NOT EXISTS subscriptions (
                id UUID PRIMARY KEY,
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                plan_id UUID NOT NULL,
                billing_period VARCHAR(32) NOT NULL,
                price NUMERIC(12,2) NOT NULL,
                starts_at TIMESTAMPTZ NOT NULL,
                ends_at TIMESTAMPTZ NOT NULL,
                status VARCHAR(32) NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active', 'past_due', 'cancelled', 'expired')),
                is_complimentary BOOLEAN NOT NULL DEFAULT false,
                grant_reason TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT uq_subscriptions_tenant_id_id UNIQUE (tenant_id, id),
                CONSTRAINT fk_subscriptions_plan_price FOREIGN KEY (plan_id, billing_period)
                    REFERENCES plan_prices(plan_id, billing_period),
                CONSTRAINT chk_subscriptions_ends_after_starts CHECK (ends_at > starts_at),
                CONSTRAINT chk_subscriptions_complimentary_grant_reason CHECK (NOT is_complimentary OR grant_reason IS NOT NULL),
                CONSTRAINT ex_subscriptions_no_overlap EXCLUDE USING gist (
                    tenant_id WITH =,
                    tstzrange(starts_at, ends_at) WITH &&
                )
            );

            CREATE INDEX IF NOT EXISTS idx_subscriptions_tenant_status ON subscriptions (tenant_id, status);

            ALTER TABLE subscriptions ENABLE ROW LEVEL SECURITY;
            ALTER TABLE subscriptions FORCE ROW LEVEL SECURITY;
            CREATE POLICY subscriptions_tenant ON subscriptions
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            -- 5. Subscription Payments
            CREATE TABLE IF NOT EXISTS subscription_payments (
                id UUID PRIMARY KEY,
                tenant_id UUID REFERENCES tenants(id) ON DELETE SET NULL,
                subscription_id UUID REFERENCES subscriptions(id) ON DELETE SET NULL,
                amount NUMERIC(12,2) NOT NULL,
                method VARCHAR(64) NOT NULL,
                reference VARCHAR(255) NOT NULL,
                proof_file_key TEXT,
                status VARCHAR(32) NOT NULL DEFAULT 'submitted'
                    CHECK (status IN ('submitted', 'confirmed', 'rejected')),
                review_note TEXT,
                reviewed_by_platform_user_id UUID,
                reviewed_at TIMESTAMPTZ,
                paid_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_subscription_payments_method_ref
                ON subscription_payments (method, reference) WHERE status <> 'rejected';
            CREATE INDEX IF NOT EXISTS idx_subscription_payments_status_paid_at
                ON subscription_payments (status, paid_at);

            ALTER TABLE subscription_payments ENABLE ROW LEVEL SECURITY;
            ALTER TABLE subscription_payments FORCE ROW LEVEL SECURITY;
            CREATE POLICY subscription_payments_tenant ON subscription_payments
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            -- 6. Tenant Exports
            CREATE TABLE IF NOT EXISTS tenant_exports (
                id UUID PRIMARY KEY,
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                requested_by_user_id UUID NOT NULL,
                status VARCHAR(32) NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'processing', 'ready', 'failed', 'expired')),
                file_key TEXT,
                error_message TEXT,
                ready_at TIMESTAMPTZ,
                expires_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT uq_tenant_exports_tenant_id_id UNIQUE (tenant_id, id)
            );

            ALTER TABLE tenant_exports ENABLE ROW LEVEL SECURITY;
            ALTER TABLE tenant_exports FORCE ROW LEVEL SECURITY;
            CREATE POLICY tenant_exports_tenant ON tenant_exports
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            -- 7. Role Grants
            -- Revoke all default privileges on newly created tables from pos_app & pos_jobs
            REVOKE ALL ON plans, plan_prices, plan_limit_types, plan_limits,
                tenants, tenant_settings, branches, subscriptions,
                subscription_payments, tenant_exports FROM pos_app;

            REVOKE ALL ON plans, plan_prices, plan_limit_types, plan_limits,
                tenants, tenant_settings, branches, subscriptions,
                subscription_payments, tenant_exports FROM pos_jobs;

            -- Explicit grants for pos_app:
            -- Read-only on platform tables
            GRANT SELECT ON plans, plan_prices, plan_limit_types, plan_limits TO pos_app;
            -- SELECT, INSERT, UPDATE on tenant-owned entities (NO DELETE)
            GRANT SELECT, INSERT, UPDATE ON tenants, tenant_settings, branches, tenant_exports TO pos_app;
            GRANT SELECT, INSERT, UPDATE ON subscriptions TO pos_app;
            -- SELECT, INSERT, UPDATE on subscription_payments (NO DELETE)
            GRANT SELECT, INSERT, UPDATE ON subscription_payments TO pos_app;

            -- Explicit grants for pos_jobs:
            GRANT SELECT, UPDATE ON tenant_exports, tenants, subscription_payments TO pos_jobs;

            -- 8. Seed: one plan with every limit unlimited [D-89]
            INSERT INTO plans (id, code, name, grace_days)
            VALUES ('0192a000-0000-7000-8000-000000000001', 'standard', 'Standard Plan', 7)
            ON CONFLICT (id) DO NOTHING;

            INSERT INTO plan_prices (plan_id, billing_period, price)
            VALUES
                ('0192a000-0000-7000-8000-000000000001', 'monthly', 1000.00),
                ('0192a000-0000-7000-8000-000000000001', 'yearly', 10000.00)
            ON CONFLICT (plan_id, billing_period) DO NOTHING;

            INSERT INTO plan_limit_types (key, description)
            VALUES
                ('branches', 'Number of active branches'),
                ('users', 'Number of active users'),
                ('devices', 'Number of registered devices')
            ON CONFLICT (key) DO NOTHING;

            INSERT INTO plan_limits (plan_id, limit_key, max_value)
            VALUES
                ('0192a000-0000-7000-8000-000000000001', 'branches', NULL),
                ('0192a000-0000-7000-8000-000000000001', 'users', NULL),
                ('0192a000-0000-7000-8000-000000000001', 'devices', NULL)
            ON CONFLICT (plan_id, limit_key) DO NOTHING;
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            r#"
            DROP TABLE IF EXISTS tenant_exports CASCADE;
            DROP TABLE IF EXISTS subscription_payments CASCADE;
            DROP TABLE IF EXISTS subscriptions CASCADE;
            DROP TABLE IF EXISTS branches CASCADE;
            DROP TABLE IF EXISTS tenant_settings CASCADE;
            DROP TABLE IF EXISTS tenants CASCADE;
            DROP TABLE IF EXISTS plan_limits CASCADE;
            DROP TABLE IF EXISTS plan_limit_types CASCADE;
            DROP TABLE IF EXISTS plan_prices CASCADE;
            DROP TABLE IF EXISTS plans CASCADE;
            "#,
        )
        .await?;

        Ok(())
    }
}
