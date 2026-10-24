use cache::Cache;
use chrono::Utc;
use db::Tx;
use pos_core::context::TenantContext;
use pos_core::error::AppError;
use pos_core::ids::{DeviceId, SessionId, TenantId, UserId};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, Statement,
};
use uuid::Uuid;

use crate::password;
use crate::rate_limit;
use crate::settings::TokenSettings;
use crate::revocation;
use crate::tokens;
use crate::types::{
    LoginRequest, LoginResponse, LogoutRequest, PasswordResetConfirmRequest, RefreshTokenRequest,
    RefreshTokenResponse, User, UserStatus,
};

#[derive(Debug, Clone)]
struct UserLookupRow {
    id: UserId,
    tenant_id: TenantId,
    password_hash: String,
    status: String,
}

pub async fn login(
    db: &DatabaseConnection,
    cache: &Cache,
    req: LoginRequest,
    settings: &TokenSettings,
    ip: Option<String>,
    user_agent: Option<String>,
) -> Result<LoginResponse, AppError> {
    let norm_email = req.email.trim().to_lowercase();

    // 1. Check rate limit BEFORE any password hashing
    rate_limit::check_login_rate_limit(cache, &norm_email).await?;

    // 2. Lookup user outside tenant tx via security definer function
    let lookup_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT id, tenant_id, password_hash, status FROM lookup_user_for_login($1)",
        [norm_email.clone().into()],
    );

    let query_res = db
        .query_one_raw(lookup_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to lookup user for login", e.to_string()))?;

    let user_row = match query_res {
        Some(row) => {
            let id: Uuid = row
                .try_get_by_index(0)
                .map_err(|e| AppError::internal("DB error reading id", e.to_string()))?;
            let tenant_id: Uuid = row
                .try_get_by_index(1)
                .map_err(|e| AppError::internal("DB error reading tenant_id", e.to_string()))?;
            let password_hash: String = row
                .try_get_by_index(2)
                .map_err(|e| AppError::internal("DB error reading password_hash", e.to_string()))?;
            let status: String = row
                .try_get_by_index(3)
                .map_err(|e| AppError::internal("DB error reading status", e.to_string()))?;

            Some(UserLookupRow {
                id: UserId::from(id),
                tenant_id: TenantId::from(tenant_id),
                password_hash,
                status,
            })
        }
        None => None,
    };

    // 3. Timing-safe verification
    let is_pwd_valid = password::verify_password_or_dummy(
        &req.password,
        user_row.as_ref().map(|u| u.password_hash.as_str()),
    );

    let is_active = user_row
        .as_ref()
        .map(|u| u.status == "active")
        .unwrap_or(false);

    if user_row.is_none() || !is_pwd_valid || !is_active {
        rate_limit::record_login_failure(cache, &norm_email).await?;
        return Err(AppError::unauthorized("Invalid email or password"));
    }

    let user = match user_row {
        Some(u) => u,
        None => return Err(AppError::unauthorized("Invalid email or password")),
    };

    // 4. Successful login: clear failure counter in Redis
    rate_limit::clear_login_attempts(cache, &norm_email).await?;

    // 5. Open database transaction to create session and update last_login_at
    let session_id = SessionId::new_v7();
    let refresh_token = tokens::generate_refresh_token();
    let refresh_hash = tokens::hash_refresh_token(&refresh_token);
    let expires_at = Utc::now() + chrono::Duration::seconds(settings.refresh_ttl_secs);

    let ctx = TenantContext::new(user.tenant_id, user.id, vec![]);
    let tx = Tx::begin(db, &ctx).await?;
    let txn = tx.transaction()?;

    // If device_id provided, verify device exists and is active
    if let Some(dev_id) = req.device_id {
        let dev_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT status FROM devices WHERE id = $1 AND tenant_id = $2",
            [dev_id.as_uuid().into(), user.tenant_id.as_uuid().into()],
        );
        let dev_row = txn
            .query_one_raw(dev_stmt)
            .await
            .map_err(|e| AppError::internal("Failed to check device", e.to_string()))?;

        match dev_row {
            Some(row) => {
                let status: String = row
                    .try_get_by_index(0)
                    .map_err(|e| AppError::internal("Failed to read device status", e.to_string()))?;
                if status == "revoked" {
                    let _ = tx.rollback().await;
                    return Err(AppError::new(
                        pos_core::error::ErrorCode::DeviceRevoked,
                        "Device has been revoked",
                    ));
                }
            }
            None => {
                let _ = tx.rollback().await;
                return Err(AppError::not_found("Device not found"));
            }
        }
    }

    // Insert session
    let insert_session_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO sessions (
            id, tenant_id, user_id, device_id, refresh_token_hash,
            ip_address, user_agent, expires_at, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())"#,
        [
            session_id.as_uuid().into(),
            user.tenant_id.as_uuid().into(),
            user.id.as_uuid().into(),
            req.device_id.map(|d| d.as_uuid()).into(),
            refresh_hash.into(),
            ip.into(),
            user_agent.into(),
            expires_at.into(),
        ],
    );
    txn.execute_raw(insert_session_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to create session", e.to_string()))?;

    // Update last_login_at
    let update_login_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE users SET last_login_at = now() WHERE id = $1 AND tenant_id = $2",
        [user.id.as_uuid().into(), user.tenant_id.as_uuid().into()],
    );
    txn.execute_raw(update_login_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to update last_login_at", e.to_string()))?;

    // Read user details for response
    let select_user_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT name, email, phone, status, must_change_password, last_login_at FROM users WHERE id = $1 AND tenant_id = $2",
        [user.id.as_uuid().into(), user.tenant_id.as_uuid().into()],
    );
    let u_row = txn
        .query_one_raw(select_user_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query user details", e.to_string()))?
        .ok_or_else(|| AppError::not_found("User record not found"))?;

    let name: String = u_row
        .try_get_by_index(0)
        .map_err(|e| AppError::internal("DB error reading name", e.to_string()))?;
    let email: String = u_row
        .try_get_by_index(1)
        .map_err(|e| AppError::internal("DB error reading email", e.to_string()))?;
    let phone: Option<String> = u_row
        .try_get_by_index(2)
        .map_err(|e| AppError::internal("DB error reading phone", e.to_string()))?;
    let st: String = u_row
        .try_get_by_index(3)
        .map_err(|e| AppError::internal("DB error reading status", e.to_string()))?;
    let must_change_pwd: bool = u_row
        .try_get_by_index(4)
        .map_err(|e| AppError::internal("DB error reading must_change_password", e.to_string()))?;
    let last_login_at: Option<chrono::DateTime<Utc>> = u_row
        .try_get_by_index(5)
        .map_err(|e| AppError::internal("DB error reading last_login_at", e.to_string()))?;

    tx.commit().await?;

    let user_status = if st == "active" {
        UserStatus::Active
    } else {
        UserStatus::Disabled
    };

    let user_resource = User {
        id: user.id,
        name,
        email,
        phone,
        status: user_status,
        must_change_password: must_change_pwd,
        last_login_at,
    };

    // 6. Generate access token
    let access_token = tokens::encode_pharmacy_token(
        user.id,
        user.tenant_id,
        &settings.jwt_secret,
        settings.access_ttl_secs,
    )?;

    Ok(LoginResponse {
        access_token,
        refresh_token,
        expires_in: settings.access_ttl_secs as u64,
        user: user_resource,
    })
}

pub async fn refresh_token(
    db: &DatabaseConnection,
    cache: &Cache,
    req: RefreshTokenRequest,
    settings: &TokenSettings,
) -> Result<RefreshTokenResponse, AppError> {
    let presented_hash = tokens::hash_refresh_token(&req.refresh_token);

    // 1. Theft detection: has this token been used before?
    if let Some(stolen_session_id) =
        revocation::check_refresh_token_reuse(cache, &presented_hash).await?
    {
        // Immediately revoke session in Redis and in DB
        revocation::revoke_session(cache, stolen_session_id, 86400).await?;

        let nil_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), UserId::from_uuid(Uuid::nil()), vec![]);
        let tx = Tx::begin_platform(db, &nil_ctx).await?;
        let txn = tx.transaction()?;
        let revoke_db_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE sessions SET revoked_at = now() WHERE id = $1",
            [stolen_session_id.as_uuid().into()],
        );
        let _ = txn.execute_raw(revoke_db_stmt).await;
        let _ = tx.commit().await;

        return Err(AppError::unauthorized(
            "Refresh token reuse detected. Session has been revoked.",
        ));
    }

    // 2. Look up session by presented refresh token hash
    let nil_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin_platform(db, &nil_ctx).await?;
    let txn = tx.transaction()?;

    let select_session_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, tenant_id, user_id, device_id, expires_at, revoked_at
           FROM sessions
           WHERE refresh_token_hash = $1"#,
        [presented_hash.clone().into()],
    );

    let session_res = txn
        .query_one_raw(select_session_stmt)
        .await
        .map_err(|e| AppError::internal("DB query error", e.to_string()))?;

    let (session_id, tenant_id, user_id, device_id, expires_at, revoked_at) = match session_res {
        Some(row) => {
            let s_id: Uuid = row.try_get_by_index(0).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let t_id: Uuid = row.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let u_id: Uuid = row.try_get_by_index(2).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let d_id: Option<Uuid> = row.try_get_by_index(3).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let exp: chrono::DateTime<Utc> = row.try_get_by_index(4).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let rev: Option<chrono::DateTime<Utc>> = row.try_get_by_index(5).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            (SessionId::from(s_id), TenantId::from(t_id), UserId::from(u_id), d_id.map(DeviceId::from), exp, rev)
        }
        None => {
            let _ = tx.rollback().await;
            return Err(AppError::unauthorized("Invalid refresh token"));
        }
    };

    // Check if session is revoked in DB or Redis
    if revoked_at.is_some() || revocation::is_session_revoked(cache, session_id).await? {
        let _ = tx.rollback().await;
        return Err(AppError::unauthorized("Session has been revoked"));
    }

    // Check expiry
    if expires_at < Utc::now() {
        let _ = tx.rollback().await;
        return Err(AppError::unauthorized("Session has expired"));
    }

    // Check device binding
    if let Some(bound_dev) = device_id {
        if req.device_id != Some(bound_dev) {
            let _ = tx.rollback().await;
            return Err(AppError::unauthorized("Refresh token is bound to another device"));
        }
    }

    // 3. Rotate refresh token
    let new_refresh_token = tokens::generate_refresh_token();
    let new_hash = tokens::hash_refresh_token(&new_refresh_token);

    // Record presented token as used in Redis
    revocation::record_used_refresh_token(cache, &presented_hash, session_id, 86400 * 7).await?;

    // Update session in DB
    let update_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE sessions SET refresh_token_hash = $1, last_used_at = now() WHERE id = $2",
        [new_hash.into(), session_id.as_uuid().into()],
    );
    txn.execute_raw(update_stmt)
        .await
        .map_err(|e| AppError::internal("Failed to update session with new refresh token", e.to_string()))?;

    tx.commit().await?;

    // 4. Generate new access token
    let access_token = tokens::encode_pharmacy_token(
        user_id,
        tenant_id,
        &settings.jwt_secret,
        settings.access_ttl_secs,
    )?;

    Ok(RefreshTokenResponse {
        access_token,
        refresh_token: new_refresh_token,
        expires_in: settings.access_ttl_secs as u64,
    })
}

pub async fn logout(
    db: &DatabaseConnection,
    cache: &Cache,
    req: LogoutRequest,
) -> Result<(), AppError> {
    let hash = tokens::hash_refresh_token(&req.refresh_token);

    let nil_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin_platform(db, &nil_ctx).await?;
    let txn = tx.transaction()?;

    let select_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT id FROM sessions WHERE refresh_token_hash = $1",
        [hash.into()],
    );
    let row = txn.query_one_raw(select_stmt).await.map_err(|e| AppError::internal("DB error", e.to_string()))?;

    if let Some(r) = row {
        let s_id: Uuid = r.try_get_by_index(0).map_err(|e| AppError::internal("DB error", e.to_string()))?;
        let session_id = SessionId::from(s_id);

        revocation::revoke_session(cache, session_id, 900).await?;

        let revoke_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE sessions SET revoked_at = now() WHERE id = $1",
            [s_id.into()],
        );
        txn.execute_raw(revoke_stmt)
            .await
            .map_err(|e| AppError::internal("DB error", e.to_string()))?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn request_password_reset(
    db: &DatabaseConnection,
    cache: &Cache,
    email: &str,
) -> Result<Option<String>, AppError> {
    let norm_email = email.trim().to_lowercase();

    // Check rate limit
    let rl_key = format!("rl:reset:{norm_email}");
    let curr = cache
        .get(&rl_key)
        .await?
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    if curr >= 5 {
        return Err(AppError::rate_limited(
            "Too many password reset requests. Please try again later.",
        ));
    }
    cache
        .set_ex(&rl_key, &(curr + 1).to_string(), 900)
        .await?;

    let lookup_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT id, tenant_id, password_hash, status FROM lookup_user_for_login($1)",
        [norm_email.into()],
    );

    let row = db
        .query_one_raw(lookup_stmt)
        .await
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;

    match row {
        Some(r) => {
            let u_id: Uuid = r.try_get_by_index(0).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let t_id: Uuid = r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?;

            let raw_token = format!("rst_{}", Uuid::now_v7());
            let token_hash = tokens::hash_refresh_token(&raw_token);
            let token_id = Uuid::now_v7();
            let expires_at = Utc::now() + chrono::Duration::minutes(15);

            let ctx = TenantContext::new(TenantId::from_uuid(t_id), UserId::from_uuid(u_id), vec![]);
            let tx = Tx::begin(db, &ctx).await?;
            let txn = tx.transaction()?;

            let insert_stmt = Statement::from_sql_and_values(
                DbBackend::Postgres,
                r#"INSERT INTO auth_tokens (id, tenant_id, user_id, token_hash, purpose, expires_at, created_at)
                   VALUES ($1, $2, $3, $4, 'password_reset', $5, now())"#,
                [token_id.into(), t_id.into(), u_id.into(), token_hash.into(), expires_at.into()],
            );
            txn.execute_raw(insert_stmt)
                .await
                .map_err(|e| AppError::internal("DB error", e.to_string()))?;
            tx.commit().await?;

            Ok(Some(raw_token))
        }
        None => {
            // Equalize timing with dummy work
            let _ = password::verify_password_or_dummy("dummy", None);
            Ok(None)
        }
    }
}

pub async fn confirm_password_reset(
    db: &DatabaseConnection,
    req: PasswordResetConfirmRequest,
) -> Result<(), AppError> {
    let token_hash = tokens::hash_refresh_token(&req.token);

    let nil_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin_platform(db, &nil_ctx).await?;
    let txn = tx.transaction()?;

    let select_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, tenant_id, user_id, expires_at, used_at
           FROM auth_tokens
           WHERE token_hash = $1 AND purpose = 'password_reset'"#,
        [token_hash.into()],
    );

    let row = txn
        .query_one_raw(select_stmt)
        .await
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;

    let (token_id, tenant_id, user_id, expires_at, used_at) = match row {
        Some(r) => {
            let tok_id: Uuid = r.try_get_by_index(0).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let t_id: Uuid = r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let u_id: Uuid = r.try_get_by_index(2).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let exp: chrono::DateTime<Utc> = r.try_get_by_index(3).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            let used: Option<chrono::DateTime<Utc>> = r.try_get_by_index(4).map_err(|e| AppError::internal("DB error", e.to_string()))?;
            (tok_id, t_id, u_id, exp, used)
        }
        None => {
            let _ = tx.rollback().await;
            return Err(AppError::bad_request("Invalid password reset token"));
        }
    };

    if used_at.is_some() {
        let _ = tx.rollback().await;
        return Err(AppError::bad_request("Password reset token has already been used"));
    }

    if expires_at < Utc::now() {
        let _ = tx.rollback().await;
        return Err(AppError::bad_request("Password reset token has expired"));
    }

    // Mark token used
    let mark_used_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE auth_tokens SET used_at = now() WHERE id = $1",
        [token_id.into()],
    );
    txn.execute_raw(mark_used_stmt)
        .await
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;

    // Hash new password
    let new_pwd_hash = password::hash_password(&req.new_password)?;

    // Update user password and clear must_change_password
    let update_pwd_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE users SET password_hash = $1, must_change_password = false WHERE id = $2 AND tenant_id = $3",
        [new_pwd_hash.into(), user_id.into(), tenant_id.into()],
    );
    txn.execute_raw(update_pwd_stmt)
        .await
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;

    tx.commit().await?;

    Ok(())
}
