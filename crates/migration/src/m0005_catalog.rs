use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            r#"
            -- 1. Extension
            CREATE EXTENSION IF NOT EXISTS pg_trgm;

            -- 2. Lookup tables (platform-owned, no tenant_id)
            CREATE TABLE IF NOT EXISTS pack_units (
                id UUID PRIMARY KEY,
                name VARCHAR(50) NOT NULL UNIQUE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS dosage_forms (
                id UUID PRIMARY KEY,
                name VARCHAR(100) NOT NULL UNIQUE,
                base_unit_id UUID NOT NULL REFERENCES pack_units(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS ingredients (
                id UUID PRIMARY KEY,
                name VARCHAR(255) NOT NULL UNIQUE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS therapeutic_classes (
                id UUID PRIMARY KEY,
                name VARCHAR(255) NOT NULL UNIQUE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            -- 3. Submittables (companies, generics)
            CREATE TABLE IF NOT EXISTS companies (
                id UUID PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                medex_id INTEGER NULL,
                status VARCHAR(20) NOT NULL DEFAULT 'pending',
                merged_into_id UUID NULL REFERENCES companies(id),
                submitted_by_tenant_id UUID NULL,
                submitted_by_user_id UUID NULL,
                reviewed_by_user_id UUID NULL,
                reviewed_at TIMESTAMPTZ NULL,
                review_note TEXT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT chk_companies_status CHECK (status IN ('pending', 'approved', 'rejected', 'merged')),
                CONSTRAINT chk_companies_merged CHECK ((status = 'merged') = (merged_into_id IS NOT NULL)),
                CONSTRAINT chk_companies_merge_self CHECK (merged_into_id <> id),
                CONSTRAINT chk_companies_rejected_note CHECK (status <> 'rejected' OR review_note IS NOT NULL)
            );

            CREATE TABLE IF NOT EXISTS generics (
                id UUID PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                medex_id INTEGER NULL,
                status VARCHAR(20) NOT NULL DEFAULT 'pending',
                merged_into_id UUID NULL REFERENCES generics(id),
                submitted_by_tenant_id UUID NULL,
                submitted_by_user_id UUID NULL,
                reviewed_by_user_id UUID NULL,
                reviewed_at TIMESTAMPTZ NULL,
                review_note TEXT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT chk_generics_status CHECK (status IN ('pending', 'approved', 'rejected', 'merged')),
                CONSTRAINT chk_generics_merged CHECK ((status = 'merged') = (merged_into_id IS NOT NULL)),
                CONSTRAINT chk_generics_merge_self CHECK (merged_into_id <> id),
                CONSTRAINT chk_generics_rejected_note CHECK (status <> 'rejected' OR review_note IS NOT NULL)
            );

            -- 4. Satellites of generics
            CREATE TABLE IF NOT EXISTS generic_monographs (
                generic_id UUID PRIMARY KEY REFERENCES generics(id) ON DELETE CASCADE,
                indications TEXT,
                description TEXT,
                pharmacology TEXT,
                dosage TEXT,
                interactions TEXT,
                contraindications TEXT,
                side_effects TEXT,
                pregnancy_and_lactation TEXT,
                precautions TEXT,
                storage TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS generic_ingredients (
                generic_id UUID NOT NULL REFERENCES generics(id) ON DELETE CASCADE,
                ingredient_id UUID NOT NULL REFERENCES ingredients(id),
                position INTEGER NOT NULL DEFAULT 1,
                PRIMARY KEY (generic_id, ingredient_id)
            );

            CREATE TABLE IF NOT EXISTS generic_therapeutic_classes (
                generic_id UUID NOT NULL REFERENCES generics(id) ON DELETE CASCADE,
                therapeutic_class_id UUID NOT NULL REFERENCES therapeutic_classes(id),
                PRIMARY KEY (generic_id, therapeutic_class_id)
            );

            -- 5. Products
            CREATE TABLE IF NOT EXISTS products (
                id UUID PRIMARY KEY,
                company_id UUID NOT NULL REFERENCES companies(id),
                generic_id UUID NULL REFERENCES generics(id),
                dosage_form_id UUID NULL REFERENCES dosage_forms(id),
                brand_name VARCHAR(255) NOT NULL,
                strength VARCHAR(100) NULL,
                product_type VARCHAR(20) NOT NULL DEFAULT 'allopathic',
                reference_unit_mrp NUMERIC(12, 2) NULL,
                is_discontinued BOOLEAN NOT NULL DEFAULT false,
                medex_id INTEGER NULL,
                status VARCHAR(20) NOT NULL DEFAULT 'pending',
                merged_into_id UUID NULL REFERENCES products(id),
                submitted_by_tenant_id UUID NULL,
                submitted_by_user_id UUID NULL,
                reviewed_by_user_id UUID NULL,
                reviewed_at TIMESTAMPTZ NULL,
                review_note TEXT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT chk_products_status CHECK (status IN ('pending', 'approved', 'rejected', 'merged')),
                CONSTRAINT chk_products_type CHECK (product_type IN ('allopathic', 'herbal', 'non_medicine')),
                CONSTRAINT chk_products_merged CHECK ((status = 'merged') = (merged_into_id IS NOT NULL)),
                CONSTRAINT chk_products_merge_self CHECK (merged_into_id <> id),
                CONSTRAINT chk_products_rejected_note CHECK (status <> 'rejected' OR review_note IS NOT NULL),
                CONSTRAINT chk_products_medicine_requires_generic_and_form CHECK (
                    product_type = 'non_medicine' OR (generic_id IS NOT NULL AND dosage_form_id IS NOT NULL)
                )
            );

            -- 6. Central product pack defaults
            CREATE TABLE IF NOT EXISTS product_packs (
                product_id UUID NOT NULL REFERENCES products(id) ON DELETE CASCADE,
                pack_unit_id UUID NOT NULL REFERENCES pack_units(id),
                base_units INTEGER NOT NULL CHECK (base_units > 0),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (product_id, pack_unit_id)
            );

            -- 7. Tenant-owned issue reports
            CREATE TABLE IF NOT EXISTS catalog_issue_reports (
                id UUID PRIMARY KEY,
                tenant_id UUID NOT NULL,
                user_id UUID NOT NULL,
                company_id UUID NULL REFERENCES companies(id),
                generic_id UUID NULL REFERENCES generics(id),
                product_id UUID NULL REFERENCES products(id),
                message TEXT NOT NULL,
                status VARCHAR(20) NOT NULL DEFAULT 'open',
                admin_note TEXT NULL,
                resolved_by_user_id UUID NULL,
                resolved_at TIMESTAMPTZ NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT chk_issue_reports_status CHECK (status IN ('open', 'resolved', 'dismissed')),
                CONSTRAINT chk_issue_reports_single_target CHECK (num_nonnulls(company_id, generic_id, product_id) = 1),
                CONSTRAINT uq_catalog_issue_reports_tenant_id UNIQUE (tenant_id, id)
            );

            -- Indexes
            CREATE UNIQUE INDEX IF NOT EXISTS idx_companies_name ON companies(name) WHERE status IN ('pending', 'approved');
            CREATE UNIQUE INDEX IF NOT EXISTS idx_companies_medex_id ON companies(medex_id) WHERE medex_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_companies_status ON companies(status);
            CREATE INDEX IF NOT EXISTS idx_companies_submitted_by_tenant ON companies(submitted_by_tenant_id);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_generics_name ON generics(name) WHERE status IN ('pending', 'approved');
            CREATE UNIQUE INDEX IF NOT EXISTS idx_generics_medex_id ON generics(medex_id) WHERE medex_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_generics_status ON generics(status);
            CREATE INDEX IF NOT EXISTS idx_generics_submitted_by_tenant ON generics(submitted_by_tenant_id);
            CREATE INDEX IF NOT EXISTS idx_generics_name_trgm ON generics USING gin (name gin_trgm_ops);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_products_unique ON products (company_id, brand_name, strength, dosage_form_id) NULLS NOT DISTINCT WHERE status IN ('pending', 'approved');
            CREATE UNIQUE INDEX IF NOT EXISTS idx_products_medex_id ON products(medex_id) WHERE medex_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_products_status ON products(status);
            CREATE INDEX IF NOT EXISTS idx_products_submitted_by_tenant ON products(submitted_by_tenant_id);
            CREATE INDEX IF NOT EXISTS idx_products_company_id ON products(company_id);
            CREATE INDEX IF NOT EXISTS idx_products_generic_id ON products(generic_id);
            CREATE INDEX IF NOT EXISTS idx_products_brand_name_trgm ON products USING gin (brand_name gin_trgm_ops);

            CREATE INDEX IF NOT EXISTS idx_catalog_issue_reports_tenant_status ON catalog_issue_reports(tenant_id, status, created_at);

            -- RLS Enablement & Enforcement for all 12 catalogue tables
            ALTER TABLE pack_units ENABLE ROW LEVEL SECURITY;
            ALTER TABLE pack_units FORCE ROW LEVEL SECURITY;

            ALTER TABLE dosage_forms ENABLE ROW LEVEL SECURITY;
            ALTER TABLE dosage_forms FORCE ROW LEVEL SECURITY;

            ALTER TABLE ingredients ENABLE ROW LEVEL SECURITY;
            ALTER TABLE ingredients FORCE ROW LEVEL SECURITY;

            ALTER TABLE therapeutic_classes ENABLE ROW LEVEL SECURITY;
            ALTER TABLE therapeutic_classes FORCE ROW LEVEL SECURITY;

            ALTER TABLE companies ENABLE ROW LEVEL SECURITY;
            ALTER TABLE companies FORCE ROW LEVEL SECURITY;

            ALTER TABLE generics ENABLE ROW LEVEL SECURITY;
            ALTER TABLE generics FORCE ROW LEVEL SECURITY;

            ALTER TABLE generic_monographs ENABLE ROW LEVEL SECURITY;
            ALTER TABLE generic_monographs FORCE ROW LEVEL SECURITY;

            ALTER TABLE generic_ingredients ENABLE ROW LEVEL SECURITY;
            ALTER TABLE generic_ingredients FORCE ROW LEVEL SECURITY;

            ALTER TABLE generic_therapeutic_classes ENABLE ROW LEVEL SECURITY;
            ALTER TABLE generic_therapeutic_classes FORCE ROW LEVEL SECURITY;

            ALTER TABLE products ENABLE ROW LEVEL SECURITY;
            ALTER TABLE products FORCE ROW LEVEL SECURITY;

            ALTER TABLE product_packs ENABLE ROW LEVEL SECURITY;
            ALTER TABLE product_packs FORCE ROW LEVEL SECURITY;

            ALTER TABLE catalog_issue_reports ENABLE ROW LEVEL SECURITY;
            ALTER TABLE catalog_issue_reports FORCE ROW LEVEL SECURITY;

            -- Shape A: Submittables (companies, generics, products)
            CREATE POLICY companies_select ON companies FOR SELECT USING (
                status = 'approved'
                OR submitted_by_tenant_id = app.current_tenant_id()
                OR current_setting('app.is_platform', true) = 'true'
            );
            CREATE POLICY companies_insert ON companies FOR INSERT WITH CHECK (
                status = 'pending'
                AND submitted_by_tenant_id = app.current_tenant_id()
            );
            CREATE POLICY companies_update ON companies FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            CREATE POLICY generics_select ON generics FOR SELECT USING (
                status = 'approved'
                OR submitted_by_tenant_id = app.current_tenant_id()
                OR current_setting('app.is_platform', true) = 'true'
            );
            CREATE POLICY generics_insert ON generics FOR INSERT WITH CHECK (
                status = 'pending'
                AND submitted_by_tenant_id = app.current_tenant_id()
            );
            CREATE POLICY generics_update ON generics FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            CREATE POLICY products_select ON products FOR SELECT USING (
                status = 'approved'
                OR submitted_by_tenant_id = app.current_tenant_id()
                OR current_setting('app.is_platform', true) = 'true'
            );
            CREATE POLICY products_insert ON products FOR INSERT WITH CHECK (
                status = 'pending'
                AND submitted_by_tenant_id = app.current_tenant_id()
            );
            CREATE POLICY products_update ON products FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            -- Shape B: Lookups and satellites (read to all, write to platform)
            CREATE POLICY pack_units_select ON pack_units FOR SELECT USING (true);
            CREATE POLICY pack_units_insert ON pack_units FOR INSERT WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY pack_units_update ON pack_units FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY pack_units_delete ON pack_units FOR DELETE USING (current_setting('app.is_platform', true) = 'true');

            CREATE POLICY dosage_forms_select ON dosage_forms FOR SELECT USING (true);
            CREATE POLICY dosage_forms_insert ON dosage_forms FOR INSERT WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY dosage_forms_update ON dosage_forms FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY dosage_forms_delete ON dosage_forms FOR DELETE USING (current_setting('app.is_platform', true) = 'true');

            CREATE POLICY ingredients_select ON ingredients FOR SELECT USING (true);
            CREATE POLICY ingredients_insert ON ingredients FOR INSERT WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY ingredients_update ON ingredients FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY ingredients_delete ON ingredients FOR DELETE USING (current_setting('app.is_platform', true) = 'true');

            CREATE POLICY therapeutic_classes_select ON therapeutic_classes FOR SELECT USING (true);
            CREATE POLICY therapeutic_classes_insert ON therapeutic_classes FOR INSERT WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY therapeutic_classes_update ON therapeutic_classes FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY therapeutic_classes_delete ON therapeutic_classes FOR DELETE USING (current_setting('app.is_platform', true) = 'true');

            CREATE POLICY generic_monographs_select ON generic_monographs FOR SELECT USING (true);
            CREATE POLICY generic_monographs_insert ON generic_monographs FOR INSERT WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY generic_monographs_update ON generic_monographs FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY generic_monographs_delete ON generic_monographs FOR DELETE USING (current_setting('app.is_platform', true) = 'true');

            CREATE POLICY generic_ingredients_select ON generic_ingredients FOR SELECT USING (true);
            CREATE POLICY generic_ingredients_insert ON generic_ingredients FOR INSERT WITH CHECK (
                current_setting('app.is_platform', true) = 'true'
                OR EXISTS (
                    SELECT 1 FROM generics g
                    WHERE g.id = generic_ingredients.generic_id
                      AND g.status = 'pending'
                      AND g.submitted_by_tenant_id = app.current_tenant_id()
                )
            );
            CREATE POLICY generic_ingredients_update ON generic_ingredients FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY generic_ingredients_delete ON generic_ingredients FOR DELETE USING (current_setting('app.is_platform', true) = 'true');

            CREATE POLICY generic_therapeutic_classes_select ON generic_therapeutic_classes FOR SELECT USING (true);
            CREATE POLICY generic_therapeutic_classes_insert ON generic_therapeutic_classes FOR INSERT WITH CHECK (
                current_setting('app.is_platform', true) = 'true'
                OR EXISTS (
                    SELECT 1 FROM generics g
                    WHERE g.id = generic_therapeutic_classes.generic_id
                      AND g.status = 'pending'
                      AND g.submitted_by_tenant_id = app.current_tenant_id()
                )
            );
            CREATE POLICY generic_therapeutic_classes_update ON generic_therapeutic_classes FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY generic_therapeutic_classes_delete ON generic_therapeutic_classes FOR DELETE USING (current_setting('app.is_platform', true) = 'true');

            -- Product packs: read to all, platform write, plus tenant insert for own pending product
            CREATE POLICY product_packs_select ON product_packs FOR SELECT USING (true);
            CREATE POLICY product_packs_submit ON product_packs FOR INSERT WITH CHECK (
                current_setting('app.is_platform', true) = 'true'
                OR EXISTS (
                    SELECT 1 FROM products p
                    WHERE p.id = product_packs.product_id
                      AND p.status = 'pending'
                      AND p.submitted_by_tenant_id = app.current_tenant_id()
                )
            );
            CREATE POLICY product_packs_update ON product_packs FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');
            CREATE POLICY product_packs_delete ON product_packs FOR DELETE
                USING (current_setting('app.is_platform', true) = 'true');

            -- Shape C: Tenant policy on catalog_issue_reports
            CREATE POLICY catalog_issue_reports_select ON catalog_issue_reports FOR SELECT USING (
                tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true'
            );
            CREATE POLICY catalog_issue_reports_insert ON catalog_issue_reports FOR INSERT WITH CHECK (
                tenant_id = app.current_tenant_id()
            );
            CREATE POLICY catalog_issue_reports_update ON catalog_issue_reports FOR UPDATE
                USING (current_setting('app.is_platform', true) = 'true')
                WITH CHECK (current_setting('app.is_platform', true) = 'true');

            -- Grants
            GRANT SELECT ON pack_units, dosage_forms, ingredients, therapeutic_classes,
                            companies, generics, generic_monographs, generic_ingredients,
                            generic_therapeutic_classes, products, product_packs,
                            catalog_issue_reports TO pos_app;
            GRANT INSERT ON companies, generics, products, product_packs, catalog_issue_reports,
                            generic_monographs, generic_ingredients, generic_therapeutic_classes TO pos_app;
            GRANT UPDATE ON companies, generics, products, product_packs, catalog_issue_reports,
                            generic_monographs TO pos_app;
            REVOKE UPDATE, DELETE ON pack_units, dosage_forms, ingredients, therapeutic_classes FROM pos_app;
            REVOKE DELETE ON companies, generics, products, product_packs, catalog_issue_reports,
                             generic_monographs, generic_ingredients, generic_therapeutic_classes FROM pos_app;

            GRANT SELECT ON pack_units, dosage_forms, ingredients, therapeutic_classes,
                            companies, generics, generic_monographs, generic_ingredients,
                            generic_therapeutic_classes, products, product_packs,
                            catalog_issue_reports TO pos_jobs;

            -- Seed the five base pack units
            INSERT INTO pack_units (id, name) VALUES
                ('0192a001-0000-7000-8000-000000000001', 'piece'),
                ('0192a001-0000-7000-8000-000000000002', 'strip'),
                ('0192a001-0000-7000-8000-000000000003', 'box'),
                ('0192a001-0000-7000-8000-000000000004', 'bottle'),
                ('0192a001-0000-7000-8000-000000000005', 'vial')
            ON CONFLICT (name) DO NOTHING;
            "#
        ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            r#"
            DROP TABLE IF EXISTS catalog_issue_reports CASCADE;
            DROP TABLE IF EXISTS product_packs CASCADE;
            DROP TABLE IF EXISTS products CASCADE;
            DROP TABLE IF EXISTS generic_therapeutic_classes CASCADE;
            DROP TABLE IF EXISTS generic_ingredients CASCADE;
            DROP TABLE IF EXISTS generic_monographs CASCADE;
            DROP TABLE IF EXISTS generics CASCADE;
            DROP TABLE IF EXISTS companies CASCADE;
            DROP TABLE IF EXISTS therapeutic_classes CASCADE;
            DROP TABLE IF EXISTS ingredients CASCADE;
            DROP TABLE IF EXISTS dosage_forms CASCADE;
            DROP TABLE IF EXISTS pack_units CASCADE;
            "#,
        )
        .await?;

        Ok(())
    }
}
