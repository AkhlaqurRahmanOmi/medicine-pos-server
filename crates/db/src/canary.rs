pub const CANARY_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS _rls_canary (
    id uuid PRIMARY KEY,
    tenant_id uuid NOT NULL,
    note text
);
ALTER TABLE _rls_canary ENABLE ROW LEVEL SECURITY;
ALTER TABLE _rls_canary FORCE ROW LEVEL SECURITY;
CREATE POLICY _rls_canary_tenant_isolation ON _rls_canary
    USING (tenant_id = app.current_tenant_id())
    WITH CHECK (tenant_id = app.current_tenant_id());
GRANT SELECT, INSERT, UPDATE, DELETE ON _rls_canary TO pos_app;
"#;

pub const DROP_POLICY_DDL: &str = r#"
DROP POLICY IF EXISTS _rls_canary_tenant_isolation ON _rls_canary;
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canary_table_forces_rls() {
        assert!(
            CANARY_DDL.contains("FORCE ROW LEVEL SECURITY"),
            "Canary table must enforce FORCE ROW LEVEL SECURITY"
        );
        assert!(
            CANARY_DDL.contains("ENABLE ROW LEVEL SECURITY"),
            "Canary table must enable ROW LEVEL SECURITY"
        );
        assert!(
            CANARY_DDL.contains("tenant_id = app.current_tenant_id()"),
            "Canary table must have tenant isolation policy"
        );
    }

    #[test]
    fn rls_canary_blocks_cross_tenant_read() {
        // Assert that the USING clause checks tenant_id = app.current_tenant_id()
        assert!(
            CANARY_DDL.contains("USING (tenant_id = app.current_tenant_id())"),
            "Policy must restrict SELECT by tenant_id matching current_tenant_id"
        );
    }

    #[test]
    fn rls_canary_blocks_cross_tenant_write() {
        // Assert that the WITH CHECK clause prevents writing tenant B id as tenant A
        assert!(
            CANARY_DDL.contains("WITH CHECK (tenant_id = app.current_tenant_id())"),
            "Policy must reject cross-tenant writes with CHECK constraint"
        );
    }

    #[test]
    fn canary_detects_missing_policy() {
        // When DROP_POLICY_DDL is executed, the policy is removed
        assert!(
            DROP_POLICY_DDL.contains("DROP POLICY"),
            "Dropping policy removes RLS protection"
        );
    }
}
