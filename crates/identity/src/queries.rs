use cache::Cache;
use pos_core::error::AppError;
use pos_core::ids::{BranchId, RoleId, TenantId, UserId};
use std::collections::BTreeMap;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use uuid::Uuid;

use crate::types::{Device, DeviceStatus, Me, Permission, PlatformUser, Role, Session, User, UserStatus};

pub async fn assignable_permissions(
    db: &DatabaseConnection,
) -> Result<Vec<Permission>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT code, module, description, is_branch_scoped FROM permissions WHERE is_tenant_assignable = true ORDER BY code",
        [],
    );

    let rows = db
        .query_all_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query permissions", e.to_string()))?;

    let mut permissions = Vec::with_capacity(rows.len());
    for row in rows {
        let code: String = row
            .try_get_by_index(0)
            .map_err(|e| AppError::internal("DB error", e.to_string()))?;
        let module: String = row
            .try_get_by_index(1)
            .map_err(|e| AppError::internal("DB error", e.to_string()))?;
        let description: String = row
            .try_get_by_index(2)
            .map_err(|e| AppError::internal("DB error", e.to_string()))?;
        let is_branch_scoped: bool = row
            .try_get_by_index(3)
            .map_err(|e| AppError::internal("DB error", e.to_string()))?;

        permissions.push(Permission {
            code,
            module,
            description,
            is_branch_scoped,
        });
    }

    Ok(permissions)
}

pub async fn role_permissions<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
    role_id: RoleId,
) -> Result<Vec<String>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT permission_code FROM role_permissions WHERE tenant_id = $1 AND role_id = $2 ORDER BY permission_code",
        [tenant_id.as_uuid().into(), role_id.as_uuid().into()],
    );

    let rows = db
        .query_all_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query role permissions", e.to_string()))?;

    let mut perms = Vec::with_capacity(rows.len());
    for row in rows {
        let code: String = row
            .try_get_by_index(0)
            .map_err(|e| AppError::internal("DB error", e.to_string()))?;
        perms.push(code);
    }

    Ok(perms)
}

pub async fn permissions_at<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
    user_id: UserId,
    branch_id: BranchId,
) -> Result<Vec<String>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT DISTINCT rp.permission_code
           FROM user_branch_roles ubr
           JOIN role_permissions rp ON ubr.role_id = rp.role_id AND ubr.tenant_id = rp.tenant_id
           WHERE ubr.tenant_id = $1 AND ubr.user_id = $2 AND (ubr.branch_id = $3 OR ubr.branch_id IS NULL)
           ORDER BY rp.permission_code"#,
        [
            tenant_id.as_uuid().into(),
            user_id.as_uuid().into(),
            branch_id.as_uuid().into(),
        ],
    );

    let rows = db
        .query_all_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query permissions at branch", e.to_string()))?;

    let mut perms = Vec::with_capacity(rows.len());
    for row in rows {
        let code: String = row
            .try_get_by_index(0)
            .map_err(|e| AppError::internal("DB error", e.to_string()))?;
        perms.push(code);
    }

    Ok(perms)
}

pub async fn branch_access<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
    user_id: UserId,
) -> Result<Vec<BranchId>, AppError> {
    // Check if user has null-branch grant (meaning every active branch)
    let null_check_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT 1 FROM user_branch_roles WHERE tenant_id = $1 AND user_id = $2 AND branch_id IS NULL LIMIT 1",
        [tenant_id.as_uuid().into(), user_id.as_uuid().into()],
    );

    let has_null = db
        .query_one_raw(null_check_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to check null branch grant", e.to_string()))?
        .is_some();

    if has_null {
        let all_branches_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT id FROM branches WHERE tenant_id = $1 AND is_active = true ORDER BY name",
            [tenant_id.as_uuid().into()],
        );
        let rows = db
            .query_all_raw(all_branches_stmt)
            .await
            .map_err(|e| AppError::internal("Failed to query branches", e.to_string()))?;

        let mut branches = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row
                .try_get_by_index(0)
                .map_err(|e| AppError::internal("DB error", e.to_string()))?;
            branches.push(BranchId::from(id));
        }
        Ok(branches)
    } else {
        let specific_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            r#"SELECT DISTINCT b.id
               FROM branches b
               JOIN user_branch_roles ubr ON b.id = ubr.branch_id AND b.tenant_id = ubr.tenant_id
               WHERE b.tenant_id = $1 AND ubr.user_id = $2 AND b.is_active = true
               ORDER BY b.id"#,
            [tenant_id.as_uuid().into(), user_id.as_uuid().into()],
        );
        let rows = db
            .query_all_raw(specific_stmt)
            .await
            .map_err(|e| AppError::internal("Failed to query user branch access", e.to_string()))?;

        let mut branches = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row
                .try_get_by_index(0)
                .map_err(|e| AppError::internal("DB error", e.to_string()))?;
            branches.push(BranchId::from(id));
        }
        Ok(branches)
    }
}

/// Everything the auth layer puts on an `AuthUser`, resolved in one pass.
///
/// `wide` is the pharmacy-wide grant — `user_branch_roles.branch_id IS NULL` `[D-14]` —
/// and `branch_permissions` carries the branch-local extras, keyed by **every** branch the
/// user can reach so the caller can derive the branch list from its keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedAccess {
    pub wide: Vec<String>,
    pub branch_permissions: BTreeMap<BranchId, Vec<String>>,
}

/// Resolve a user's permissions and branches.
///
/// **Must run inside a transaction that carries the tenant context.** Every table read here
/// is tenant-owned with forced RLS, so on a bare connection this returns nothing at all
/// rather than failing — which is how an empty `AuthUser` went unnoticed. Two queries, never
/// one per branch: this is on the path of every request `[AC-U13]`.
pub async fn resolve_access<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
    user_id: UserId,
) -> Result<ResolvedAccess, AppError> {
    let grants_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT ubr.branch_id, rp.permission_code
           FROM user_branch_roles ubr
           JOIN role_permissions rp
             ON rp.tenant_id = ubr.tenant_id AND rp.role_id = ubr.role_id
           LEFT JOIN branches b
             ON b.id = ubr.branch_id AND b.tenant_id = ubr.tenant_id
           WHERE ubr.tenant_id = $1 AND ubr.user_id = $2
             AND (ubr.branch_id IS NULL OR b.is_active = true)
           ORDER BY rp.permission_code"#,
        [tenant_id.as_uuid().into(), user_id.as_uuid().into()],
    );
    let rows = db
        .query_all_raw(grants_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to resolve permissions", e.to_string()))?;

    let mut wide: Vec<String> = Vec::new();
    let mut branch_permissions: BTreeMap<BranchId, Vec<String>> = BTreeMap::new();
    let mut has_wide_grant = false;

    for row in rows {
        let branch: Option<Uuid> = row
            .try_get_by_index(0)
            .map_err(|e| AppError::internal("DB error", e.to_string()))?;
        let code: String = row
            .try_get_by_index(1)
            .map_err(|e| AppError::internal("DB error", e.to_string()))?;

        match branch {
            None => {
                has_wide_grant = true;
                if !wide.contains(&code) {
                    wide.push(code);
                }
            }
            Some(b) => {
                let entry = branch_permissions.entry(BranchId::from(b)).or_default();
                if !entry.contains(&code) {
                    entry.push(code);
                }
            }
        }
    }

    // A null-branch grant means every **active** branch, including ones created later
    // `[AC-U13]`, so the branch list is derived now rather than stored.
    if has_wide_grant {
        for branch in branch_access(db, tenant_id, user_id).await? {
            branch_permissions.entry(branch).or_default();
        }
    }

    Ok(ResolvedAccess {
        wide,
        branch_permissions,
    })
}

pub async fn branch_access_cached(
    db: &DatabaseConnection,
    cache: &Cache,
    tenant_id: TenantId,
    user_id: UserId,
) -> Result<Vec<BranchId>, AppError> {
    let key = format!("t:{tenant_id}:u:{user_id}:branches");

    // Cache lookup: if cache errors, DENY (do not fallback to all branches)
    let cached_opt = cache.get(&key).await?;

    if let Some(json_str) = cached_opt {
        if let Ok(branches) = serde_json::from_str::<Vec<BranchId>>(&json_str) {
            return Ok(branches);
        }
    }

    // Cache miss: derive from the database, inside a transaction that carries the context —
    // `branches` is RLS-forced, so a bare read would cache an empty list for five minutes.
    let ctx = pos_core::context::TenantContext::new(tenant_id, user_id, vec![]);
    let tx = db::Tx::begin(db, &ctx).await?;
    let branches = branch_access(tx.transaction()?, tenant_id, user_id).await?;
    tx.commit().await?;

    if let Ok(json_str) = serde_json::to_string(&branches) {
        let _ = cache.set_ex(&key, &json_str, 300).await;
    }

    Ok(branches)
}

pub async fn list_users<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
) -> Result<Vec<User>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, name, email, phone, status, must_change_password, last_login_at
           FROM users
           WHERE tenant_id = $1
           ORDER BY created_at"#,
        [tenant_id.as_uuid().into()],
    );
    let rows = db
        .query_all_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to list users", e.to_string()))?;

    let mut users = Vec::with_capacity(rows.len());
    for r in rows {
        let status_str: String = r.try_get_by_index(4).map_err(|e| AppError::internal("DB error", e.to_string()))?;
        let status = if status_str == "active" { UserStatus::Active } else { UserStatus::Disabled };
        users.push(User {
            id: UserId::from(r.try_get_by_index::<Uuid>(0).map_err(|e| AppError::internal("DB error", e.to_string()))?),
            name: r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            email: r.try_get_by_index(2).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            phone: r.try_get_by_index(3).ok(),
            status,
            must_change_password: r.try_get_by_index(5).unwrap_or(false),
            last_login_at: r.try_get_by_index(6).ok(),
        });
    }
    Ok(users)
}

pub async fn get_user<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
    user_id: UserId,
) -> Result<Option<User>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, name, email, phone, status, must_change_password, last_login_at
           FROM users
           WHERE id = $1 AND tenant_id = $2"#,
        [user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let row = db
        .query_one_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to get user", e.to_string()))?;

    match row {
        Some(r) => {
            let status_str: String = r.try_get_by_index(4).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let status = if status_str == "active" { UserStatus::Active } else { UserStatus::Disabled };
            Ok(Some(User {
                id: UserId::from(r.try_get_by_index::<Uuid>(0).map_err(|e| AppError::internal("DB error", e.to_string()))?),
                name: r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
                email: r.try_get_by_index(2).map_err(|e| AppError::internal("DB error", e.to_string()))?,
                phone: r.try_get_by_index(3).ok(),
                status,
                must_change_password: r.try_get_by_index(5).unwrap_or(false),
                last_login_at: r.try_get_by_index(6).ok(),
            }))
        }
        None => Ok(None),
    }
}

pub async fn list_roles<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
) -> Result<Vec<Role>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, name, is_system, is_owner, template_code
           FROM roles
           WHERE tenant_id = $1
           ORDER BY name"#,
        [tenant_id.as_uuid().into()],
    );
    let rows = db
        .query_all_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to list roles", e.to_string()))?;

    let mut roles = Vec::with_capacity(rows.len());
    for r in rows {
        let role_id = RoleId::from(r.try_get_by_index::<Uuid>(0).map_err(|e| AppError::internal("DB error", e.to_string()))?);
        let perms = role_permissions(db, tenant_id, role_id).await.unwrap_or_default();
        roles.push(Role {
            id: role_id,
            name: r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            is_system: r.try_get_by_index(2).unwrap_or(false),
            is_owner: r.try_get_by_index(3).unwrap_or(false),
            template_code: r.try_get_by_index(4).ok(),
            permissions: perms,
        });
    }
    Ok(roles)
}

pub async fn get_role<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
    role_id: RoleId,
) -> Result<Option<Role>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, name, is_system, is_owner, template_code
           FROM roles
           WHERE id = $1 AND tenant_id = $2"#,
        [role_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let row = db
        .query_one_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to get role", e.to_string()))?;

    match row {
        Some(r) => {
            let perms = role_permissions(db, tenant_id, role_id).await.unwrap_or_default();
            Ok(Some(Role {
                id: role_id,
                name: r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
                is_system: r.try_get_by_index(2).unwrap_or(false),
                is_owner: r.try_get_by_index(3).unwrap_or(false),
                template_code: r.try_get_by_index(4).ok(),
                permissions: perms,
            }))
        }
        None => Ok(None),
    }
}

pub async fn list_devices<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
) -> Result<Vec<Device>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, branch_id, name, invoice_prefix, status, app_version, last_seen_at
           FROM devices
           WHERE tenant_id = $1
           ORDER BY created_at"#,
        [tenant_id.as_uuid().into()],
    );
    let rows = db
        .query_all_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to list devices", e.to_string()))?;

    let mut devices = Vec::with_capacity(rows.len());
    for r in rows {
        let status_str: String = r.try_get_by_index(4).map_err(|e| AppError::internal("DB error", e.to_string()))?;
        let status = if status_str == "active" { DeviceStatus::Active } else { DeviceStatus::Revoked };
        devices.push(Device {
            id: pos_core::ids::DeviceId::from(r.try_get_by_index::<Uuid>(0).map_err(|e| AppError::internal("DB error", e.to_string()))?),
            branch_id: BranchId::from(r.try_get_by_index::<Uuid>(1).map_err(|e| AppError::internal("DB error", e.to_string()))?),
            name: r.try_get_by_index(2).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            invoice_prefix: r.try_get_by_index(3).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            status,
            app_version: r.try_get_by_index(5).ok(),
            last_seen_at: r.try_get_by_index(6).ok(),
        });
    }
    Ok(devices)
}

pub async fn get_me<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
    user_id: UserId,
) -> Result<Option<Me>, AppError> {
    let user = match get_user(db, tenant_id, user_id).await? {
        Some(u) => u,
        None => return Ok(None),
    };
    let access = resolve_access(db, tenant_id, user_id).await?;
    let branch_ids = access.branch_permissions.keys().copied().collect();
    Ok(Some(Me {
        user,
        permissions: access.wide,
        branch_ids,
    }))
}

pub async fn list_sessions<C: ConnectionTrait>(
    db: &C,
    tenant_id: TenantId,
) -> Result<Vec<Session>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, user_id, device_id, last_used_at, expires_at
           FROM sessions
           WHERE tenant_id = $1 AND revoked_at IS NULL AND expires_at > now()
           ORDER BY created_at DESC"#,
        [tenant_id.as_uuid().into()],
    );
    let rows = db
        .query_all_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to list sessions", e.to_string()))?;

    let mut sessions = Vec::with_capacity(rows.len());
    for r in rows {
        let dev_opt: Option<Uuid> = r.try_get_by_index(2).ok().flatten();
        sessions.push(Session {
            id: pos_core::ids::SessionId::from(r.try_get_by_index::<Uuid>(0).map_err(|e| AppError::internal("DB error", e.to_string()))?),
            user_id: UserId::from(r.try_get_by_index::<Uuid>(1).map_err(|e| AppError::internal("DB error", e.to_string()))?),
            device_id: dev_opt.map(pos_core::ids::DeviceId::from),
            last_used_at: r.try_get_by_index(3).ok(),
            expires_at: r.try_get_by_index(4).map_err(|e| AppError::internal("DB error", e.to_string()))?,
        });
    }
    Ok(sessions)
}

pub async fn list_platform_users<C: ConnectionTrait>(
    db: &C,
) -> Result<Vec<PlatformUser>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, email, role, status, last_login_at
           FROM platform_users
           ORDER BY created_at"#,
        [],
    );
    let rows = db
        .query_all_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to list platform users", e.to_string()))?;

    let mut users = Vec::with_capacity(rows.len());
    for r in rows {
        users.push(PlatformUser {
            id: r.try_get_by_index::<Uuid>(0).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            email: r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            role: r.try_get_by_index(2).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            status: r.try_get_by_index(3).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            last_login_at: r.try_get_by_index(4).ok(),
        });
    }
    Ok(users)
}

