#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The slugs this crate declares must be rows a pharmacy can actually be granted.
//!
//! `perms.rs` declaring a slug and `rest.rs` enforcing it proves nothing on its own: until
//! `m0006_catalog_permissions` seeded them, none of the five existed in `permissions`, so
//! `role_permissions`'s FK and its `check_role_permission_assignable()` trigger refused every
//! grant and each gated endpoint was unreachable for everyone, the owner included.
//! `specs/01-catalog/BACKLOG.md` A1.

use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;

const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";

async fn owner_pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| OWNER_URL.to_string());
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(1000))
        .connect(&url)
        .await
        .expect("Owner database connection required")
}

/// Every slug in `catalog::perms`, with whether a pharmacy may hold it `[D-78]`.
const SLUGS: &[(&str, bool)] = &[
    ("catalog.view", true),
    ("catalog.submit", true),
    ("catalog.issue.report", true),
    ("catalog.review", false),
    ("catalog.lookup.manage", false),
];

#[tokio::test]
async fn every_catalog_slug_is_a_grantable_permission() {
    let owner = owner_pool().await;

    // 1. Every slug exists, in this module's group, with the assignability `[D-78]` requires.
    for (code, assignable) in SLUGS {
        let row: Option<(String, bool, bool)> = sqlx::query_as(
            "SELECT module, is_tenant_assignable, is_branch_scoped FROM permissions WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&owner)
        .await
        .unwrap();

        let (module, is_assignable, is_branch_scoped) = row.unwrap_or_else(|| {
            panic!("permission '{code}' is declared in catalog::perms but has no row in `permissions`")
        });

        assert_eq!(module, "catalog", "'{code}' should be grouped under catalog");
        assert_eq!(
            is_assignable, *assignable,
            "'{code}' has the wrong is_tenant_assignable; the platform-only pair is hidden from the role editor [D-78]"
        );
        assert!(
            !is_branch_scoped,
            "'{code}' should not be branch-scoped — the catalogue is one shared central list"
        );
    }

    // 2. The assignable three can actually be granted. This is the half that was broken: the
    //    trigger refused the insert, so no role could ever hold them.
    let mut tx = owner.begin().await.unwrap();
    let tenant_id = Uuid::now_v7();
    let role_id = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO tenants (id, name, status, created_at, updated_at)
         VALUES ($1, 'Grantable Slug Probe', 'active', now(), now())",
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .expect("seed tenant");

    sqlx::query(
        "INSERT INTO roles (id, tenant_id, name, is_system, is_owner, template_code, created_at, updated_at)
         VALUES ($1, $2, 'Probe Owner', true, true, 'owner', now(), now())",
    )
    .bind(role_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .expect("seed owner role");

    for (code, assignable) in SLUGS {
        let res = sqlx::query(
            "INSERT INTO role_permissions (tenant_id, role_id, permission_code, created_at)
             VALUES ($1, $2, $3, now())",
        )
        .bind(tenant_id)
        .bind(role_id)
        .bind(code)
        .execute(&mut *tx)
        .await;

        if *assignable {
            res.unwrap_or_else(|e| {
                panic!("'{code}' must be grantable to a role, but the insert failed: {e}")
            });
        } else {
            assert!(
                res.is_err(),
                "'{code}' is platform-only and must be refused on a direct insert [D-78]"
            );
            // The trigger aborts the statement, so the transaction needs a clean slate.
            break;
        }
    }

    // Nothing is kept: this probe only proves the grant is permitted.
    tx.rollback().await.unwrap();

    // 3. `[D-79]`: a new pharmacy's owner role holds every assignable permission, which is
    //    seeded from the template — so the template must carry the three.
    for (code, assignable) in SLUGS.iter().filter(|(_, a)| *a) {
        let _ = assignable;
        let present: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM role_template_permissions WHERE template_code = 'owner' AND permission_code = $1",
        )
        .bind(code)
        .fetch_optional(&owner)
        .await
        .unwrap();
        assert!(
            present.is_some(),
            "the owner template must hold '{code}', or a new pharmacy's owner cannot use the catalogue [D-79]"
        );
    }
}
