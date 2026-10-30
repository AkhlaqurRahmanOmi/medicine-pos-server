use cache::Cache;
use db::Tx;
use pos_core::error::AppError;
use pos_core::ids::{BranchId, DeviceId, RoleId, SessionId, UserId};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

use crate::{password, revocation};

pub async fn seed_roles_for_tenant(
    tx: &Tx,
) -> Result<(RoleId, RoleId, RoleId), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let owner_role_id = RoleId::new_v7();
    let manager_role_id = RoleId::new_v7();
    let cashier_role_id = RoleId::new_v7();

    // 1. Insert 3 system roles from templates
    let insert_roles_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO roles (id, tenant_id, name, is_system, is_owner, template_code, created_at, updated_at)
           VALUES
             ($1, $4, 'Owner', true, true, 'owner', now(), now()),
             ($2, $4, 'Manager', true, false, 'manager', now(), now()),
             ($3, $4, 'Cashier', true, false, 'cashier', now(), now())"#,
        [
            owner_role_id.as_uuid().into(),
            manager_role_id.as_uuid().into(),
            cashier_role_id.as_uuid().into(),
            tenant_id.as_uuid().into(),
        ],
    );
    conn.execute_raw(insert_roles_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to insert system roles", e.to_string()))?;

    // 2. Populate role_permissions from role_template_permissions
    let copy_perms_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO role_permissions (tenant_id, role_id, permission_code, created_at)
           SELECT $1, r.id, rtp.permission_code, now()
           FROM roles r
           JOIN role_template_permissions rtp ON r.template_code = rtp.template_code
           WHERE r.tenant_id = $1"#,
        [tenant_id.as_uuid().into()],
    );
    conn.execute_raw(copy_perms_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to copy template permissions", e.to_string()))?;

    Ok((owner_role_id, manager_role_id, cashier_role_id))
}

pub async fn create_role(
    tx: &Tx,
    name: &str,
    permissions: &[String],
) -> Result<RoleId, AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;
    let role_id = RoleId::new_v7();

    // Verify all permissions are tenant assignable
    for perm in permissions {
        let check_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT is_tenant_assignable FROM permissions WHERE code = $1",
            [perm.clone().into()],
        );
        let row = conn
            .query_one_raw(check_stmt)
            .await
            .map_err(|e| AppError::internal("Failed to check permission", e.to_string()))?;

        match row {
            Some(r) => {
                let assignable: bool = r
                    .try_get_by_index(0)
                    .map_err(|e| AppError::internal("DB error", e.to_string()))?;
                if !assignable {
                    return Err(AppError::forbidden(format!(
                        "Permission '{perm}' is not tenant assignable"
                    )));
                }
            }
            None => {
                return Err(AppError::not_found(format!("Permission '{perm}' not found")));
            }
        }
    }

    let insert_role_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO roles (id, tenant_id, name, is_system, is_owner, created_at, updated_at)
           VALUES ($1, $2, $3, false, false, now(), now())"#,
        [role_id.as_uuid().into(), tenant_id.as_uuid().into(), name.into()],
    );
    conn.execute_raw(insert_role_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to insert role", e.to_string()))?;

    for perm in permissions {
        let insert_perm_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            r#"INSERT INTO role_permissions (tenant_id, role_id, permission_code, created_at)
               VALUES ($1, $2, $3, now())"#,
            [tenant_id.as_uuid().into(), role_id.as_uuid().into(), perm.clone().into()],
        );
        conn.execute_raw(insert_perm_stmt)
            .await
            .map_err(|e| AppError::internal("Failed to insert role permission", e.to_string()))?;
    }

    Ok(role_id)
}

pub async fn update_role(
    tx: &Tx,
    role_id: RoleId,
    new_name: &str,
) -> Result<(), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let check_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT is_system FROM roles WHERE id = $1 AND tenant_id = $2",
        [role_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let row = conn
        .query_one_raw(check_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query role", e.to_string()))?
        .ok_or_else(|| AppError::not_found("Role not found"))?;

    let is_system: bool = row
        .try_get_by_index(0)
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;

    if is_system {
        return Err(AppError::conflict_code(
            "role_is_system",
            "A system role cannot be renamed or modified",
        ));
    }

    let update_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE roles SET name = $1, updated_at = now() WHERE id = $2 AND tenant_id = $3",
        [new_name.into(), role_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    conn.execute_raw(update_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to update role", e.to_string()))?;

    Ok(())
}

pub async fn delete_role(
    tx: &Tx,
    role_id: RoleId,
) -> Result<(), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let check_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT is_system FROM roles WHERE id = $1 AND tenant_id = $2",
        [role_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let row = conn
        .query_one_raw(check_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query role", e.to_string()))?
        .ok_or_else(|| AppError::not_found("Role not found"))?;

    let is_system: bool = row
        .try_get_by_index(0)
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;

    if is_system {
        return Err(AppError::conflict_code(
            "role_is_system",
            "A system role cannot be deleted",
        ));
    }

    let del_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "DELETE FROM roles WHERE id = $1 AND tenant_id = $2",
        [role_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    conn.execute_raw(del_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to delete role", e.to_string()))?;

    Ok(())
}

pub async fn set_role_permissions(
    tx: &Tx,
    cache: &Cache,
    role_id: RoleId,
    permissions: &[String],
) -> Result<(), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let check_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT is_system FROM roles WHERE id = $1 AND tenant_id = $2",
        [role_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let row = conn
        .query_one_raw(check_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query role", e.to_string()))?
        .ok_or_else(|| AppError::not_found("Role not found"))?;

    let is_system: bool = row
        .try_get_by_index(0)
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;

    if is_system {
        return Err(AppError::conflict_code(
            "role_is_system",
            "A system role's permissions cannot be modified",
        ));
    }

    // Check all permissions are assignable
    for perm in permissions {
        let check_perm_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT is_tenant_assignable FROM permissions WHERE code = $1",
            [perm.clone().into()],
        );
        let p_row = conn
            .query_one_raw(check_perm_stmt)
            .await
            .map_err(|e| AppError::internal("Failed to check permission", e.to_string()))?;

        match p_row {
            Some(r) => {
                let assignable: bool = r
                    .try_get_by_index(0)
                    .map_err(|e| AppError::internal("DB error", e.to_string()))?;
                if !assignable {
                    return Err(AppError::forbidden(format!(
                        "Permission '{perm}' is not tenant assignable"
                    )));
                }
            }
            None => {
                return Err(AppError::not_found(format!("Permission '{perm}' not found")));
            }
        }
    }

    let del_perms_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "DELETE FROM role_permissions WHERE role_id = $1 AND tenant_id = $2",
        [role_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    conn.execute_raw(del_perms_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to delete role permissions", e.to_string()))?;

    for perm in permissions {
        let ins_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO role_permissions (tenant_id, role_id, permission_code, created_at) VALUES ($1, $2, $3, now())",
            [tenant_id.as_uuid().into(), role_id.as_uuid().into(), perm.clone().into()],
        );
        conn.execute_raw(ins_stmt)
            .await
            .map_err(|e| AppError::internal("Failed to insert role permission", e.to_string()))?;
    }

    // Invalidate cached branch access for all users with this role
    let users_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT DISTINCT user_id FROM user_branch_roles WHERE role_id = $1 AND tenant_id = $2",
        [role_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let u_rows = conn
        .query_all_raw(users_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query role users", e.to_string()))?;

    for u_row in u_rows {
        if let Ok(uid) = u_row.try_get_by_index::<Uuid>(0) {
            let _ = revocation::clear_cached_branch_access(cache, tenant_id, UserId::from(uid)).await;
        }
    }

    Ok(())
}

pub async fn grant_role(
    tx: &Tx,
    cache: &Cache,
    user_id: UserId,
    role_id: RoleId,
    branch_id: Option<BranchId>,
) -> Result<Uuid, AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let user_check = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT 1 FROM users WHERE id = $1 AND tenant_id = $2",
        [user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    if conn
        .query_one_raw(user_check)
        .await
        .map_err(|e| AppError::internal("Failed to check user", e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("User not found"));
    }

    let role_check = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT 1 FROM roles WHERE id = $1 AND tenant_id = $2",
        [role_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    if conn
        .query_one_raw(role_check)
        .await
        .map_err(|e| AppError::internal("Failed to check role", e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("Role not found"));
    }

    if let Some(bid) = branch_id {
        let branch_check = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT 1 FROM branches WHERE id = $1 AND tenant_id = $2",
            [bid.as_uuid().into(), tenant_id.as_uuid().into()],
        );
        if conn
            .query_one_raw(branch_check)
            .await
            .map_err(|e| AppError::internal("Failed to check branch", e.to_string()))?
            .is_none()
        {
            return Err(AppError::not_found("Branch not found"));
        }
    }

    let grant_id = Uuid::now_v7();

    let insert_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO user_branch_roles (id, tenant_id, user_id, role_id, branch_id, created_at)
           VALUES ($1, $2, $3, $4, $5, now())"#,
        [
            grant_id.into(),
            tenant_id.as_uuid().into(),
            user_id.as_uuid().into(),
            role_id.as_uuid().into(),
            branch_id.map(|b| b.as_uuid()).into(),
        ],
    );
    conn.execute_raw(insert_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to grant role", e.to_string()))?;

    // Clear cached branch access
    revocation::clear_cached_branch_access(cache, tenant_id, user_id).await?;

    Ok(grant_id)
}

pub async fn revoke_grant(
    tx: &Tx,
    cache: &Cache,
    grant_id: Uuid,
) -> Result<(), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let query_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT ubr.user_id, r.is_owner
           FROM user_branch_roles ubr
           JOIN roles r ON ubr.role_id = r.id AND ubr.tenant_id = r.tenant_id
           WHERE ubr.id = $1 AND ubr.tenant_id = $2"#,
        [grant_id.into(), tenant_id.as_uuid().into()],
    );
    let row = conn
        .query_one_raw(query_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query grant", e.to_string()))?
        .ok_or_else(|| AppError::not_found("Role grant not found"))?;

    let u_id: Uuid = row.try_get_by_index(0).map_err(|e| AppError::internal("DB error", e.to_string()))?;
    let is_owner: bool = row.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?;

    if is_owner {
        let owners_count_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            r#"SELECT count(DISTINCT u.id)
               FROM users u
               JOIN user_branch_roles ubr ON u.id = ubr.user_id AND u.tenant_id = ubr.tenant_id
               JOIN roles r ON ubr.role_id = r.id AND ubr.tenant_id = r.tenant_id
               WHERE u.tenant_id = $1 AND r.is_owner = true AND u.status = 'active' AND ubr.id != $2"#,
            [tenant_id.as_uuid().into(), grant_id.into()],
        );
        let count_row = conn
            .query_one_raw(owners_count_stmt)
            .await
            .map_err(|e| AppError::internal("Failed to count active owners", e.to_string()))?
            .ok_or_else(|| AppError::internal("Count query failed", ""))?;

        let count: i64 = count_row.try_get_by_index(0).map_err(|e| AppError::internal("DB error", e.to_string()))?;
        if count == 0 {
            return Err(AppError::conflict_code(
                "last_active_owner",
                "The last active owner cannot be stripped of the owner role",
            ));
        }
    }

    let del_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "DELETE FROM user_branch_roles WHERE id = $1 AND tenant_id = $2",
        [grant_id.into(), tenant_id.as_uuid().into()],
    );
    conn.execute_raw(del_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to revoke role grant", e.to_string()))?;

    revocation::clear_cached_branch_access(cache, tenant_id, UserId::from(u_id)).await?;

    Ok(())
}

pub async fn disable_user(
    tx: &Tx,
    cache: &Cache,
    user_id: UserId,
) -> Result<(), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let user_check_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT 1 FROM users WHERE id = $1 AND tenant_id = $2",
        [user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    if conn
        .query_one_raw(user_check_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to check user existence", e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("User not found"));
    }

    // Check if this user holds owner role
    let owner_check_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT 1
           FROM user_branch_roles ubr
           JOIN roles r ON ubr.role_id = r.id AND ubr.tenant_id = r.tenant_id
           WHERE ubr.tenant_id = $1 AND ubr.user_id = $2 AND r.is_owner = true
           LIMIT 1"#,
        [tenant_id.as_uuid().into(), user_id.as_uuid().into()],
    );
    let is_owner = conn
        .query_one_raw(owner_check_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to check owner status", e.to_string()))?
        .is_some();

    if is_owner {
        let other_owners_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            r#"SELECT count(DISTINCT u.id)
               FROM users u
               JOIN user_branch_roles ubr ON u.id = ubr.user_id AND u.tenant_id = ubr.tenant_id
               JOIN roles r ON ubr.role_id = r.id AND ubr.tenant_id = r.tenant_id
               WHERE u.tenant_id = $1 AND r.is_owner = true AND u.status = 'active' AND u.id != $2"#,
            [tenant_id.as_uuid().into(), user_id.as_uuid().into()],
        );
        let count_row = conn
            .query_one_raw(other_owners_stmt)
            .await
            .map_err(|e| AppError::internal("Failed to count active owners", e.to_string()))?
            .ok_or_else(|| AppError::internal("Count query failed", ""))?;

        let count: i64 = count_row
            .try_get_by_index(0)
            .map_err(|e| AppError::internal("DB error", e.to_string()))?;

        if count == 0 {
            return Err(AppError::conflict_code(
                "last_active_owner",
                "The last active owner cannot be disabled",
            ));
        }
    }

    let update_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE users SET status = 'disabled', updated_at = now() WHERE id = $1 AND tenant_id = $2",
        [user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    conn.execute_raw(update_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to disable user", e.to_string()))?;

    // Revoke sessions of disabled user
    let sessions_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT id FROM sessions WHERE user_id = $1 AND tenant_id = $2 AND revoked_at IS NULL",
        [user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let s_rows = conn
        .query_all_raw(sessions_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query sessions", e.to_string()))?;

    for s_row in s_rows {
        if let Ok(sid) = s_row.try_get_by_index::<Uuid>(0) {
            let _ = revocation::revoke_session(cache, pos_core::ids::SessionId::from(sid), 900).await;
        }
    }

    let revoke_db_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND tenant_id = $2",
        [user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    conn.execute_raw(revoke_db_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to revoke sessions in DB", e.to_string()))?;

    revocation::clear_cached_branch_access(cache, tenant_id, user_id).await?;

    Ok(())
}

pub async fn register_device(
    tx: &Tx,
    branch_id: BranchId,
    name: &str,
    invoice_prefix: &str,
    platform: Option<&str>,
    app_version: Option<&str>,
) -> Result<DeviceId, AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;
    let device_id = DeviceId::new_v7();
    let platform = platform.unwrap_or("android");

    // Check branch belongs to tenant and is active
    let branch_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        // `branches` carries `is_active`, not a `status` enum — 02 Tenancy owns the table.
        "SELECT 1 FROM branches WHERE id = $1 AND tenant_id = $2 AND is_active = true",
        [branch_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let branch_exists = conn
        .query_one_raw(branch_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to check branch", e.to_string()))?
        .is_some();
    if !branch_exists {
        return Err(AppError::not_found("Branch not found or inactive"));
    }

    // Check unique prefix in tenant
    let prefix_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT 1 FROM devices WHERE tenant_id = $1 AND invoice_prefix = $2",
        [tenant_id.as_uuid().into(), invoice_prefix.into()],
    );
    let prefix_taken = conn
        .query_one_raw(prefix_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to check device prefix", e.to_string()))?
        .is_some();
    if prefix_taken {
        return Err(AppError::conflict_code(
            "prefix_already_in_use",
            format!("Invoice prefix '{invoice_prefix}' is already in use in this pharmacy"),
        ));
    }

    let insert_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO devices (id, tenant_id, branch_id, name, invoice_prefix, status, platform, app_version, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, 'active', $6, $7, now(), now())"#,
        [
            device_id.as_uuid().into(),
            tenant_id.as_uuid().into(),
            branch_id.as_uuid().into(),
            name.into(),
            invoice_prefix.into(),
            platform.into(),
            app_version.map(String::from).into(),
        ],
    );
    conn.execute_raw(insert_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to register device", e.to_string()))?;

    Ok(device_id)
}

pub async fn revoke_device(
    tx: &Tx,
    cache: &Cache,
    device_id: DeviceId,
) -> Result<(), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let device_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT status FROM devices WHERE id = $1 AND tenant_id = $2",
        [device_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let device_row = conn
        .query_one_raw(device_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query device", e.to_string()))?;

    match device_row {
        Some(r) => {
            let status: String = r
                .try_get_by_index(0)
                .map_err(|e| AppError::internal("DB error", e.to_string()))?;
            if status == "revoked" {
                return Ok(());
            }
        }
        None => return Err(AppError::not_found("Device not found")),
    }

    // Revoke device
    let update_device_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE devices SET status = 'revoked', revoked_at = now(), updated_at = now() WHERE id = $1 AND tenant_id = $2",
        [device_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    conn.execute_raw(update_device_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to revoke device", e.to_string()))?;

    // Query active sessions on this device
    let sessions_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT id FROM sessions WHERE tenant_id = $1 AND device_id = $2 AND revoked_at IS NULL",
        [tenant_id.as_uuid().into(), device_id.as_uuid().into()],
    );
    let s_rows = conn
        .query_all_raw(sessions_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query device sessions", e.to_string()))?;

    for s in s_rows {
        if let Ok(sid) = s.try_get_by_index::<Uuid>(0) {
            let _ = revocation::revoke_session(cache, SessionId::from(sid), 900).await;
        }
    }

    // In same transaction: mark sessions revoked
    let revoke_sessions_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE sessions SET revoked_at = now() WHERE tenant_id = $1 AND device_id = $2 AND revoked_at IS NULL",
        [tenant_id.as_uuid().into(), device_id.as_uuid().into()],
    );
    conn.execute_raw(revoke_sessions_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to revoke sessions for device", e.to_string()))?;

    Ok(())
}

pub async fn set_user_pin(
    tx: &Tx,
    user_id: UserId,
    pin: &str,
) -> Result<(), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    if pin.len() < 4 {
        return Err(AppError::bad_request("PIN must be at least 4 digits"));
    }

    let pin_hash = password::hash_password(pin)?;

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE users SET pin_hash = $1, updated_at = now() WHERE id = $2 AND tenant_id = $3",
        [pin_hash.into(), user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let res = conn.execute_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to set user PIN", e.to_string()))?;

    if res.rows_affected() == 0 {
        return Err(AppError::not_found("User not found"));
    }

    Ok(())
}

pub async fn unlock_session_with_pin(
    tx: &Tx,
    device_id: DeviceId,
    user_id: UserId,
    pin: &str,
) -> Result<SessionId, AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    // Check device is active
    let dev_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT status FROM devices WHERE id = $1 AND tenant_id = $2",
        [device_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let dev_row = conn
        .query_one_raw(dev_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to check device", e.to_string()))?;
    match dev_row {
        Some(r) => {
            let status: String = r
                .try_get_by_index(0)
                .map_err(|e| AppError::internal("DB error", e.to_string()))?;
            if status != "active" {
                return Err(AppError::unauthorized("Device is revoked"));
            }
        }
        None => return Err(AppError::not_found("Device not found")),
    }

    // Check for an existing unrevoked, unexpired session for this user on this device
    let sess_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id
           FROM sessions
           WHERE tenant_id = $1 AND user_id = $2 AND device_id = $3
             AND revoked_at IS NULL AND expires_at > now()
           ORDER BY created_at DESC
           LIMIT 1"#,
        [tenant_id.as_uuid().into(), user_id.as_uuid().into(), device_id.as_uuid().into()],
    );
    let sess_row = conn
        .query_one_raw(sess_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query session", e.to_string()))?;

    let session_uuid: Uuid = match sess_row {
        Some(r) => r
            .try_get_by_index(0)
            .map_err(|e| AppError::internal("DB error", e.to_string()))?,
        None => {
            return Err(AppError::unauthorized(
                "No active session on this device for this user; full login required",
            ));
        }
    };

    // Verify user and pin
    let user_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT pin_hash, status FROM users WHERE id = $1 AND tenant_id = $2",
        [user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let user_row = conn
        .query_one_raw(user_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query user", e.to_string()))?;

    let (pin_hash_opt, status): (Option<String>, String) = match user_row {
        Some(r) => (
            r.try_get_by_index(0).ok(),
            r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
        ),
        None => return Err(AppError::unauthorized("User not found")),
    };

    if status != "active" {
        return Err(AppError::unauthorized("User is disabled"));
    }

    let pin_hash = match pin_hash_opt {
        Some(h) if !h.is_empty() => h,
        _ => return Err(AppError::unauthorized("User has no PIN set")),
    };

    let valid = password::verify_password(pin, &pin_hash);
    if !valid {
        return Err(AppError::unauthorized("Invalid PIN"));
    }

    // Update last_used_at
    let update_sess = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE sessions SET last_used_at = now() WHERE id = $1 AND tenant_id = $2",
        [session_uuid.into(), tenant_id.as_uuid().into()],
    );
    let _ = conn.execute_raw(update_sess).await;

    Ok(SessionId::from(session_uuid))
}

pub async fn verify_manager_override(
    tx: &Tx,
    manager_user_id: UserId,
    pin: &str,
    branch_id: Option<BranchId>,
    required_permission: &str,
) -> Result<UserId, AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    // 1. Fetch manager user
    let user_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT pin_hash, status FROM users WHERE id = $1 AND tenant_id = $2",
        [manager_user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let user_row = conn
        .query_one_raw(user_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query manager user", e.to_string()))?;

    let (pin_hash_opt, status): (Option<String>, String) = match user_row {
        Some(r) => (
            r.try_get_by_index(0).ok(),
            r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
        ),
        None => return Err(AppError::unauthorized("Manager user not found")),
    };

    if status != "active" {
        return Err(AppError::unauthorized("Manager user is disabled"));
    }

    let pin_hash = match pin_hash_opt {
        Some(h) if !h.is_empty() => h,
        _ => return Err(AppError::unauthorized("Manager has no PIN configured")),
    };

    let valid = password::verify_password(pin, &pin_hash);
    if !valid {
        return Err(AppError::unauthorized("Invalid manager PIN"));
    }

    // 2. Check if manager holds the required permission at branch_id (or pharmacy-wide)
    let permissions = if let Some(bid) = branch_id {
        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            r#"SELECT DISTINCT rp.permission_code
               FROM user_branch_roles ubr
               JOIN role_permissions rp ON ubr.role_id = rp.role_id AND ubr.tenant_id = rp.tenant_id
               WHERE ubr.tenant_id = $1 AND ubr.user_id = $2 AND (ubr.branch_id = $3 OR ubr.branch_id IS NULL)"#,
            [
                tenant_id.as_uuid().into(),
                manager_user_id.as_uuid().into(),
                bid.as_uuid().into(),
            ],
        );
        let rows = conn
            .query_all_raw(stmt)
            .await
            .map_err(|e| AppError::internal("Failed to query branch permissions", e.to_string()))?;
        let mut codes = Vec::new();
        for r in rows {
            if let Ok(c) = r.try_get_by_index::<String>(0) {
                codes.push(c);
            }
        }
        codes
    } else {
        let wide_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            r#"SELECT DISTINCT rp.permission_code
               FROM user_branch_roles ubr
               JOIN role_permissions rp ON ubr.role_id = rp.role_id AND ubr.tenant_id = rp.tenant_id
               WHERE ubr.tenant_id = $1 AND ubr.user_id = $2"#,
            [tenant_id.as_uuid().into(), manager_user_id.as_uuid().into()],
        );
        let rows = conn
            .query_all_raw(wide_stmt)
            .await
            .map_err(|e| AppError::internal("Failed to query permissions", e.to_string()))?;
        let mut codes = Vec::new();
        for r in rows {
            if let Ok(c) = r.try_get_by_index::<String>(0) {
                codes.push(c);
            }
        }
        codes
    };

    if !permissions.iter().any(|p| p == required_permission) {
        return Err(AppError::forbidden(format!(
            "Approver does not have required permission '{required_permission}'"
        )));
    }

    Ok(manager_user_id)
}

pub async fn create_user(
    tx: &Tx,
    name: &str,
    email: &str,
    phone: Option<&str>,
    initial_password: Option<&str>,
) -> Result<UserId, AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;
    let user_id = UserId::new_v7();
    let email = email.trim().to_lowercase();

    // Check email uniqueness across all pharmacies
    let email_check = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT 1 FROM users WHERE lower(email) = $1",
        [email.clone().into()],
    );
    let email_taken = conn
        .query_one_raw(email_check)
        .await
        .map_err(|e| AppError::internal("Failed to check email", e.to_string()))?
        .is_some();
    if email_taken {
        return Err(AppError::conflict_code(
            "email_already_registered",
            "This email is already registered",
        ));
    }

    let default_pwd = initial_password.unwrap_or("TemporaryPass123!");
    let pwd_hash = password::hash_password(default_pwd)?;

    let insert_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO users (id, tenant_id, email, name, phone, password_hash, status, must_change_password, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, 'active', true, now(), now())"#,
        [
            user_id.as_uuid().into(),
            tenant_id.as_uuid().into(),
            email.into(),
            name.into(),
            phone.map(String::from).into(),
            pwd_hash.into(),
        ],
    );
    conn.execute_raw(insert_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to insert user", e.to_string()))?;

    Ok(user_id)
}

pub async fn update_user(
    tx: &Tx,
    user_id: UserId,
    name: Option<&str>,
    phone: Option<&str>,
) -> Result<(), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"UPDATE users
           SET name = COALESCE($1, name),
               phone = COALESCE($2, phone),
               updated_at = now()
           WHERE id = $3 AND tenant_id = $4"#,
        [
            name.map(String::from).into(),
            phone.map(String::from).into(),
            user_id.as_uuid().into(),
            tenant_id.as_uuid().into(),
        ],
    );
    let res = conn.execute_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to update user", e.to_string()))?;

    if res.rows_affected() == 0 {
        return Err(AppError::not_found("User not found"));
    }

    Ok(())
}

pub async fn reset_user_password(
    tx: &Tx,
    user_id: UserId,
) -> Result<String, AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;
    let temp_password = format!("TempPass{}!", &Uuid::now_v7().to_string()[..8]);
    let pwd_hash = password::hash_password(&temp_password)?;

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE users SET password_hash = $1, must_change_password = true, updated_at = now() WHERE id = $2 AND tenant_id = $3",
        [pwd_hash.into(), user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let res = conn
        .execute_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to reset password", e.to_string()))?;
    if res.rows_affected() == 0 {
        return Err(AppError::not_found("User not found"));
    }
    Ok(temp_password)
}

pub async fn change_password(
    tx: &Tx,
    user_id: UserId,
    current_password: Option<&str>,
    new_password: &str,
) -> Result<(), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let select_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT password_hash FROM users WHERE id = $1 AND tenant_id = $2",
        [user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let row = conn
        .query_one_raw(select_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query user", e.to_string()))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    let current_hash: String = row
        .try_get_by_index(0)
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;
    if let Some(curr_pwd) = current_password {
        if !password::verify_password_or_dummy(curr_pwd, Some(&current_hash)) {
            return Err(AppError::unauthorized("Current password does not match"));
        }
    }

    let new_hash = password::hash_password(new_password)?;
    let update_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE users SET password_hash = $1, must_change_password = false, updated_at = now() WHERE id = $2 AND tenant_id = $3",
        [new_hash.into(), user_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    conn.execute_raw(update_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to update password", e.to_string()))?;

    Ok(())
}

pub async fn revoke_session(
    tx: &Tx,
    cache: &Cache,
    session_id: SessionId,
) -> Result<(), AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE sessions SET revoked_at = now() WHERE id = $1 AND tenant_id = $2",
        [session_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let res = conn
        .execute_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to revoke session", e.to_string()))?;

    if res.rows_affected() == 0 {
        return Err(AppError::not_found("Session not found"));
    }

    revocation::revoke_session(cache, session_id, 86400).await?;
    Ok(())
}

pub async fn create_platform_user<C: ConnectionTrait>(
    conn: &C,
    email: &str,
    role: &str,
) -> Result<Uuid, AppError> {
    let user_id = Uuid::now_v7();
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO platform_users (id, email, role, totp_secret_encrypted, status, created_at, updated_at)
           VALUES ($1, $2, $3, 'dummy_totp_secret_for_staff', 'active', now(), now())"#,
        [user_id.into(), email.to_lowercase().into(), role.into()],
    );
    conn.execute_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to create platform user", e.to_string()))?;
    Ok(user_id)
}

pub async fn end_impersonation_session<C: ConnectionTrait>(
    conn: &C,
    id: Uuid,
) -> Result<(), AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE impersonation_sessions SET ended_at = now() WHERE id = $1",
        [id.into()],
    );
    let res = conn
        .execute_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to end impersonation session", e.to_string()))?;
    if res.rows_affected() == 0 {
        return Err(AppError::not_found("Impersonation session not found"));
    }
    Ok(())
}
