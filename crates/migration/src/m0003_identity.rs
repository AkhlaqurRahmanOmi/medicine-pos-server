use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            r#"
            -- 1. Platform tables (no tenant_id)
            CREATE TABLE IF NOT EXISTS permissions (
                code VARCHAR(64) PRIMARY KEY,
                module VARCHAR(64) NOT NULL,
                description TEXT NOT NULL,
                is_branch_scoped BOOLEAN NOT NULL DEFAULT false,
                is_tenant_assignable BOOLEAN NOT NULL DEFAULT true,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS role_templates (
                code VARCHAR(64) PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                description TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS role_template_permissions (
                template_code VARCHAR(64) NOT NULL REFERENCES role_templates(code) ON DELETE CASCADE,
                permission_code VARCHAR(64) NOT NULL REFERENCES permissions(code) ON DELETE CASCADE,
                PRIMARY KEY (template_code, permission_code)
            );

            CREATE TABLE IF NOT EXISTS platform_users (
                id UUID PRIMARY KEY,
                email VARCHAR(255) NOT NULL UNIQUE,
                role VARCHAR(32) NOT NULL CHECK (role IN ('super_admin', 'reviewer', 'support', 'billing')),
                password_hash VARCHAR(255) NOT NULL,
                totp_secret_encrypted VARCHAR(255) NOT NULL,
                status VARCHAR(32) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
                last_login_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            -- RLS for platform tables
            ALTER TABLE permissions ENABLE ROW LEVEL SECURITY;
            ALTER TABLE permissions FORCE ROW LEVEL SECURITY;
            CREATE POLICY permissions_select ON permissions FOR SELECT USING (true);
            CREATE POLICY permissions_admin ON permissions FOR ALL
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            ALTER TABLE role_templates ENABLE ROW LEVEL SECURITY;
            ALTER TABLE role_templates FORCE ROW LEVEL SECURITY;
            CREATE POLICY role_templates_select ON role_templates FOR SELECT USING (true);
            CREATE POLICY role_templates_admin ON role_templates FOR ALL
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            ALTER TABLE role_template_permissions ENABLE ROW LEVEL SECURITY;
            ALTER TABLE role_template_permissions FORCE ROW LEVEL SECURITY;
            CREATE POLICY role_template_permissions_select ON role_template_permissions FOR SELECT USING (true);
            CREATE POLICY role_template_permissions_admin ON role_template_permissions FOR ALL
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            ALTER TABLE platform_users ENABLE ROW LEVEL SECURITY;
            ALTER TABLE platform_users FORCE ROW LEVEL SECURITY;
            CREATE POLICY platform_users_admin ON platform_users FOR ALL
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            -- 2. Tenant tables
            CREATE TABLE IF NOT EXISTS users (
                id UUID PRIMARY KEY,
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                email VARCHAR(255) NOT NULL UNIQUE,
                phone VARCHAR(32),
                name VARCHAR(255) NOT NULL,
                password_hash VARCHAR(255) NOT NULL,
                pin_hash VARCHAR(255),
                must_change_password BOOLEAN NOT NULL DEFAULT false,
                status VARCHAR(32) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
                last_login_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT uq_users_tenant_id UNIQUE (tenant_id, id)
            );

            ALTER TABLE users ENABLE ROW LEVEL SECURITY;
            ALTER TABLE users FORCE ROW LEVEL SECURITY;
            CREATE POLICY users_tenant ON users
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            CREATE TABLE IF NOT EXISTS roles (
                id UUID NOT NULL,
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                name VARCHAR(255) NOT NULL,
                is_system BOOLEAN NOT NULL DEFAULT false,
                is_owner BOOLEAN NOT NULL DEFAULT false,
                template_code VARCHAR(64) REFERENCES role_templates(code),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (tenant_id, id),
                CONSTRAINT uq_roles_tenant_name UNIQUE (tenant_id, name)
            );

            CREATE UNIQUE INDEX idx_roles_one_owner ON roles (tenant_id) WHERE is_owner = true;

            ALTER TABLE roles ENABLE ROW LEVEL SECURITY;
            ALTER TABLE roles FORCE ROW LEVEL SECURITY;
            CREATE POLICY roles_tenant ON roles
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            CREATE TABLE IF NOT EXISTS role_permissions (
                tenant_id UUID NOT NULL,
                role_id UUID NOT NULL,
                permission_code VARCHAR(64) NOT NULL REFERENCES permissions(code) ON DELETE CASCADE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (tenant_id, role_id, permission_code),
                FOREIGN KEY (tenant_id, role_id) REFERENCES roles(tenant_id, id) ON DELETE CASCADE
            );

            ALTER TABLE role_permissions ENABLE ROW LEVEL SECURITY;
            ALTER TABLE role_permissions FORCE ROW LEVEL SECURITY;
            CREATE POLICY role_permissions_tenant ON role_permissions
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            CREATE OR REPLACE FUNCTION check_role_permission_assignable() RETURNS TRIGGER AS $$
            BEGIN
                IF NOT EXISTS (
                    SELECT 1 FROM permissions WHERE code = NEW.permission_code AND is_tenant_assignable = true
                ) THEN
                    RAISE EXCEPTION 'Permission % is not tenant assignable', NEW.permission_code;
                END IF;
                RETURN NEW;
            END;
            $$ LANGUAGE plpgsql;

            CREATE TRIGGER trg_check_role_permission_assignable
                BEFORE INSERT OR UPDATE ON role_permissions
                FOR EACH ROW
                EXECUTE FUNCTION check_role_permission_assignable();

            CREATE TABLE IF NOT EXISTS user_branch_roles (
                id UUID NOT NULL,
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                user_id UUID NOT NULL,
                role_id UUID NOT NULL,
                branch_id UUID,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (tenant_id, id),
                FOREIGN KEY (tenant_id, user_id) REFERENCES users(tenant_id, id) ON DELETE CASCADE,
                FOREIGN KEY (tenant_id, role_id) REFERENCES roles(tenant_id, id) ON DELETE CASCADE,
                FOREIGN KEY (tenant_id, branch_id) REFERENCES branches(tenant_id, id) ON DELETE CASCADE,
                CONSTRAINT uq_user_branch_roles UNIQUE NULLS NOT DISTINCT (tenant_id, user_id, role_id, branch_id)
            );

            ALTER TABLE user_branch_roles ENABLE ROW LEVEL SECURITY;
            ALTER TABLE user_branch_roles FORCE ROW LEVEL SECURITY;
            CREATE POLICY user_branch_roles_tenant ON user_branch_roles
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            CREATE TABLE IF NOT EXISTS devices (
                id UUID NOT NULL,
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                branch_id UUID NOT NULL,
                name VARCHAR(255) NOT NULL,
                invoice_prefix VARCHAR(16) NOT NULL,
                status VARCHAR(32) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'revoked')),
                platform VARCHAR(32) NOT NULL DEFAULT 'android' CHECK (platform IN ('android', 'ios', 'web')),
                app_version VARCHAR(64),
                push_token TEXT,
                last_seen_at TIMESTAMPTZ,
                revoked_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (tenant_id, id),
                FOREIGN KEY (tenant_id, branch_id) REFERENCES branches(tenant_id, id) ON DELETE CASCADE,
                CONSTRAINT uq_devices_prefix UNIQUE (tenant_id, invoice_prefix)
            );

            ALTER TABLE devices ENABLE ROW LEVEL SECURITY;
            ALTER TABLE devices FORCE ROW LEVEL SECURITY;
            CREATE POLICY devices_tenant ON devices
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            CREATE TABLE IF NOT EXISTS sessions (
                id UUID NOT NULL,
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                user_id UUID NOT NULL,
                device_id UUID,
                refresh_token_hash VARCHAR(255) NOT NULL UNIQUE,
                ip_address VARCHAR(45),
                user_agent TEXT,
                expires_at TIMESTAMPTZ NOT NULL,
                last_used_at TIMESTAMPTZ,
                revoked_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (tenant_id, id),
                FOREIGN KEY (tenant_id, user_id) REFERENCES users(tenant_id, id) ON DELETE CASCADE,
                FOREIGN KEY (tenant_id, device_id) REFERENCES devices(tenant_id, id) ON DELETE SET NULL
            );

            ALTER TABLE sessions ENABLE ROW LEVEL SECURITY;
            ALTER TABLE sessions FORCE ROW LEVEL SECURITY;
            CREATE POLICY sessions_tenant ON sessions
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            CREATE TABLE IF NOT EXISTS auth_tokens (
                id UUID NOT NULL,
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                user_id UUID NOT NULL,
                token_hash VARCHAR(255) NOT NULL UNIQUE,
                purpose VARCHAR(32) NOT NULL CHECK (purpose IN ('password_reset', 'phone_verify')),
                expires_at TIMESTAMPTZ NOT NULL,
                used_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (tenant_id, id),
                FOREIGN KEY (tenant_id, user_id) REFERENCES users(tenant_id, id) ON DELETE CASCADE
            );

            ALTER TABLE auth_tokens ENABLE ROW LEVEL SECURITY;
            ALTER TABLE auth_tokens FORCE ROW LEVEL SECURITY;
            CREATE POLICY auth_tokens_tenant ON auth_tokens
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
                WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');

            CREATE TABLE IF NOT EXISTS impersonation_sessions (
                id UUID PRIMARY KEY,
                platform_user_id UUID NOT NULL REFERENCES platform_users(id) ON DELETE CASCADE,
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                reason TEXT NOT NULL,
                writes_enabled_at TIMESTAMPTZ,
                expires_at TIMESTAMPTZ NOT NULL,
                ended_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            ALTER TABLE impersonation_sessions ENABLE ROW LEVEL SECURITY;
            ALTER TABLE impersonation_sessions FORCE ROW LEVEL SECURITY;
            CREATE POLICY impersonation_sessions_select ON impersonation_sessions FOR SELECT
                USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');
            CREATE POLICY impersonation_sessions_admin ON impersonation_sessions FOR ALL
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            -- 3. The login lookup function
            CREATE OR REPLACE FUNCTION lookup_user_for_login(p_email TEXT)
            RETURNS TABLE (
                id UUID,
                tenant_id UUID,
                password_hash VARCHAR(255),
                status VARCHAR(32)
            )
            SECURITY DEFINER
            SET search_path = public, pg_temp
            AS $$
            BEGIN
                RETURN QUERY
                SELECT u.id, u.tenant_id, u.password_hash, u.status
                FROM users u
                WHERE lower(u.email) = lower(p_email)
                LIMIT 1;
            END;
            $$ LANGUAGE plpgsql;

            -- 4. Grants
            GRANT SELECT, INSERT, UPDATE, DELETE ON users, roles, role_permissions, user_branch_roles, devices, sessions, auth_tokens TO pos_app;
            GRANT SELECT ON permissions, role_templates, role_template_permissions, impersonation_sessions TO pos_app;
            GRANT EXECUTE ON FUNCTION lookup_user_for_login(TEXT) TO pos_app;
            REVOKE ALL ON platform_users FROM pos_app;

            GRANT SELECT, INSERT, UPDATE, DELETE ON users, roles, role_permissions, user_branch_roles, devices, sessions, auth_tokens, impersonation_sessions TO pos_jobs;

            -- 5. Seed data: Permissions
            INSERT INTO permissions (code, module, description, is_branch_scoped, is_tenant_assignable) VALUES
                ('tenant.view', 'tenancy', 'View tenant pharmacy details', false, true),
                ('tenant.update', 'tenancy', 'Update tenant pharmacy profile', false, true),
                ('tenant.settings.update', 'tenancy', 'Update tenant settings', false, true),
                ('branch.view', 'tenancy', 'View branches', false, true),
                ('branch.create', 'tenancy', 'Create a new branch', false, true),
                ('branch.update', 'tenancy', 'Update branch details', true, true),
                ('subscription.view', 'tenancy', 'View subscription information', false, true),
                ('subscription.pay', 'tenancy', 'Submit subscription payment', false, true),
                ('tenant.export', 'tenancy', 'Export tenant data', false, true),
                ('tenant.close', 'tenancy', 'Close pharmacy account', false, true),

                ('user.view', 'identity', 'View staff users', true, true),
                ('user.create', 'identity', 'Create a new staff user', false, true),
                ('user.update', 'identity', 'Update staff user details', false, true),
                ('user.disable', 'identity', 'Disable staff user', false, true),
                ('user.reset_password', 'identity', 'Reset staff user password', false, true),
                ('role.view', 'identity', 'View roles and permissions', false, true),
                ('role.create', 'identity', 'Create custom roles', false, true),
                ('role.update', 'identity', 'Update custom roles and permissions', false, true),
                ('role.delete', 'identity', 'Delete custom roles', false, true),
                ('role.assign', 'identity', 'Assign roles to staff at branches', true, true),
                ('device.view', 'identity', 'View registered POS devices', true, true),
                ('device.register', 'identity', 'Register a new POS device', true, true),
                ('device.revoke', 'identity', 'Revoke a POS device', true, true),
                ('session.view', 'identity', 'View active sessions', false, true),
                ('session.revoke', 'identity', 'Revoke an active session', false, true),

                ('platform.users', 'platform', 'Manage platform staff users', false, false),
                ('platform.impersonate', 'platform', 'Impersonate a tenant pharmacy', false, false),
                ('platform.review', 'platform', 'Review tenant registrations and approvals', false, false)
            ON CONFLICT (code) DO NOTHING;

            -- Seed role templates
            INSERT INTO role_templates (code, name, description) VALUES
                ('owner', 'Pharmacy Owner', 'Full control over all pharmacy branches, settings, users and finances'),
                ('manager', 'Branch Manager', 'Operations and management at assigned branches'),
                ('cashier', 'Cashier', 'Counter sales and standard POS operation')
            ON CONFLICT (code) DO NOTHING;

            -- Seed role template permissions: Owner gets every assignable permission
            INSERT INTO role_template_permissions (template_code, permission_code)
            SELECT 'owner', code
            FROM permissions
            WHERE is_tenant_assignable = true
            ON CONFLICT (template_code, permission_code) DO NOTHING;

            -- Manager template permissions
            INSERT INTO role_template_permissions (template_code, permission_code) VALUES
                ('manager', 'branch.view'),
                ('manager', 'branch.update'),
                ('manager', 'user.view'),
                ('manager', 'role.view'),
                ('manager', 'role.assign'),
                ('manager', 'device.view'),
                ('manager', 'device.register'),
                ('manager', 'device.revoke'),
                ('manager', 'session.view'),
                ('manager', 'session.revoke')
            ON CONFLICT (template_code, permission_code) DO NOTHING;

            -- Cashier template permissions
            INSERT INTO role_template_permissions (template_code, permission_code) VALUES
                ('cashier', 'device.view')
            ON CONFLICT (template_code, permission_code) DO NOTHING;
            "#
        ).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
