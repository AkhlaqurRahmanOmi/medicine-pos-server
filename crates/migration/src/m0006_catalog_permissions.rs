use sea_orm_migration::prelude::*;

/// Seeds module 01's five permission slugs, which no migration ever created.
///
/// `crates/catalog/src/perms.rs` declares them and `rest.rs` enforces four of them, but
/// `m0005_catalog` contains no `INSERT INTO permissions`, so the rows did not exist. Because
/// `role_permissions` carries an FK to `permissions(code)` and the
/// `check_role_permission_assignable()` trigger, the slugs could not be granted to any role —
/// the insert failed with *"Permission catalog.view is not tenant assignable"*. Every
/// permission-gated catalogue endpoint was therefore unreachable for every user, the owner
/// included. Found 2026-10-25, recorded as `specs/01-catalog/BACKLOG.md` A1.
///
/// A new migration rather than an edit to `m0005_catalog`: Article V.4 — a migration already
/// applied is immutable.
///
/// Three slugs are tenant-assignable; `catalog.review` and `catalog.lookup.manage` are
/// platform-only `[D-78]`, so they are seeded with `is_tenant_assignable = false` and are
/// hidden from the role editor and refused if submitted directly. Nothing in the catalogue is
/// branch-scoped — it is one shared central catalogue, so `is_branch_scoped` is false
/// throughout.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        let sql = r#"
            -- The five slugs `catalog::perms` declares. `module` groups them in the role
            -- editor; `is_tenant_assignable = false` is what hides the platform-only pair
            -- [D-78], a column rather than a list in the editor.
            INSERT INTO permissions (code, module, description, is_branch_scoped, is_tenant_assignable) VALUES
                ('catalog.view',          'catalog', 'Read the central medicine catalogue',                false, true),
                ('catalog.submit',        'catalog', 'Submit a new product, company or generic for review', false, true),
                ('catalog.issue.report',  'catalog', 'Report a problem with a catalogue entry',            false, true),
                ('catalog.review',        'catalog', 'Review, approve, reject and merge submissions',      false, false),
                ('catalog.lookup.manage', 'catalog', 'Maintain the catalogue lookup tables',               false, false)
            ON CONFLICT (code) DO NOTHING;

            -- The owner template is seeded in m0003 from `is_tenant_assignable = true` as it
            -- stood then, so a permission added later is absent from it. [D-79] says the owner
            -- role holds every assignable permission, so the template gains the three.
            INSERT INTO role_template_permissions (template_code, permission_code)
            SELECT 'owner', code
            FROM permissions
            WHERE code IN ('catalog.view', 'catalog.submit', 'catalog.issue.report')
            ON CONFLICT (template_code, permission_code) DO NOTHING;

            -- The template only seeds a pharmacy at signup, so pharmacies that already exist
            -- have owner roles without these rows. Backfill them, or [D-79] holds for new
            -- pharmacies and silently fails for every current one.
            INSERT INTO role_permissions (tenant_id, role_id, permission_code, created_at)
            SELECT r.tenant_id, r.id, p.code, now()
            FROM roles r
            CROSS JOIN permissions p
            WHERE r.is_owner = true
              AND p.code IN ('catalog.view', 'catalog.submit', 'catalog.issue.report')
            ON CONFLICT (tenant_id, role_id, permission_code) DO NOTHING;

            -- Not a judgement call: `01-catalog/SPEC.md` §4 settles it — "Everything readable
            -- here is readable by a cashier. Submission is not: Product Spec §3 limits it to
            -- owner and manager, which 03 Identity expresses as `catalog.submit` on those two
            -- seeded roles."
            INSERT INTO role_template_permissions (template_code, permission_code) VALUES
                ('manager', 'catalog.view'),
                ('manager', 'catalog.submit'),
                ('cashier', 'catalog.view')
            ON CONFLICT (template_code, permission_code) DO NOTHING;

            INSERT INTO role_permissions (tenant_id, role_id, permission_code, created_at)
            SELECT r.tenant_id, r.id, 'catalog.view', now()
            FROM roles r
            WHERE r.is_system = true AND r.template_code IN ('manager', 'cashier')
            ON CONFLICT (tenant_id, role_id, permission_code) DO NOTHING;

            INSERT INTO role_permissions (tenant_id, role_id, permission_code, created_at)
            SELECT r.tenant_id, r.id, 'catalog.submit', now()
            FROM roles r
            WHERE r.is_system = true AND r.template_code = 'manager'
            ON CONFLICT (tenant_id, role_id, permission_code) DO NOTHING;
        "#;

        db.execute_unprepared(sql).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
