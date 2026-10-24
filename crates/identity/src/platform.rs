use chrono::{DateTime, Duration, Utc};
use db::Tx;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use pos_core::error::AppError;
use pos_core::ids::TenantId;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde::{Deserialize, Serialize};
use totp_rs::{Algorithm as TotpAlgorithm, Secret, TOTP};
use uuid::Uuid;

use crate::password;
use crate::settings::TokenSettings;
use crate::tokens::encode_platform_token;
use crate::types::{
    ImpersonationSessionItem, PlatformLoginResponse, PlatformLoginStepOneRequest,
    PlatformLoginStepOneResponse, PlatformLoginStepTwoRequest,
};

const STEP_TWO_PURPOSE: &str = "platform_step_two";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StepTwoClaims {
    sub: String,
    role: String,
    purpose: String,
    exp: i64,
}

pub async fn platform_login_step_one<C: ConnectionTrait>(
    conn: &C,
    req: PlatformLoginStepOneRequest,
    jwt_secret: &str,
) -> Result<PlatformLoginStepOneResponse, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, password_hash, totp_secret_encrypted, role, status
           FROM platform_users
           WHERE lower(email) = lower($1)
           LIMIT 1"#,
        [req.email.trim().to_lowercase().into()],
    );
    let row = conn
        .query_one_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query platform user", e.to_string()))?;

    let (user_id, pwd_hash, totp_secret, role, status): (
        Uuid,
        String,
        String,
        String,
        String,
    ) = match row {
        Some(r) => (
            r.try_get_by_index(0).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            r.try_get_by_index(2).unwrap_or_default(),
            r.try_get_by_index(3).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            r.try_get_by_index(4).map_err(|e| AppError::internal("DB error", e.to_string()))?,
        ),
        None => return Err(AppError::unauthorized("Invalid platform credentials")),
    };

    if status != "active" {
        return Err(AppError::unauthorized("Platform user account is disabled"));
    }

    // Two-factor is required, not optional, for every platform user
    if totp_secret.trim().is_empty() || totp_secret.trim() == "none" {
        return Err(AppError::forbidden(
            "Two-factor authentication is required for every platform user",
        ));
    }

    let valid_pwd = password::verify_password(&req.password, &pwd_hash);
    if !valid_pwd {
        return Err(AppError::unauthorized("Invalid platform credentials"));
    }

    // Step one issues NO access token, only a temporary step-two ticket
    let now = Utc::now().timestamp();
    let claims = StepTwoClaims {
        sub: user_id.to_string(),
        role,
        purpose: STEP_TWO_PURPOSE.to_string(),
        exp: now + 300, // 5 minutes
    };
    let temp_token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::internal("Failed to encode step two token", e.to_string()))?;

    Ok(PlatformLoginStepOneResponse {
        temp_token,
        requires_totp: true,
    })
}

pub async fn platform_login_step_two<C: ConnectionTrait>(
    conn: &C,
    req: PlatformLoginStepTwoRequest,
    settings: &TokenSettings,
) -> Result<PlatformLoginResponse, AppError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_aud = false;
    validation.leeway = 0;

    let token_data = decode::<StepTwoClaims>(
        &req.temp_token,
        &DecodingKey::from_secret(settings.jwt_secret.as_bytes()),
        &validation,
    )
    .map_err(|e| AppError::unauthorized(format!("Invalid step two token: {e}")))?;

    if token_data.claims.purpose != STEP_TWO_PURPOSE {
        return Err(AppError::unauthorized("Invalid token purpose"));
    }

    let user_uuid = Uuid::parse_str(&token_data.claims.sub)
        .map_err(|_| AppError::unauthorized("Invalid user id in token"))?;

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT totp_secret_encrypted, role, status FROM platform_users WHERE id = $1",
        [user_uuid.into()],
    );
    let row = conn
        .query_one_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query platform user", e.to_string()))?;

    let (totp_secret, role, status): (String, String, String) = match row {
        Some(r) => (
            r.try_get_by_index(0).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            r.try_get_by_index(2).map_err(|e| AppError::internal("DB error", e.to_string()))?,
        ),
        None => return Err(AppError::unauthorized("Platform user not found")),
    };

    if status != "active" {
        return Err(AppError::unauthorized("Platform user account is disabled"));
    }

    // Verify TOTP code
    let secret_bytes = Secret::Encoded(totp_secret)
        .to_bytes()
        .map_err(|e| AppError::internal("Failed to decode TOTP secret", e.to_string()))?;

    let totp = TOTP::new(
        TotpAlgorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
    )
    .map_err(|e| AppError::internal("Failed to initialize TOTP", e.to_string()))?;

    let valid = totp
        .check_current(&req.totp_code)
        .map_err(|e| AppError::internal("Failed to check TOTP code", e.to_string()))?;

    if !valid {
        return Err(AppError::unauthorized("Invalid TOTP code"));
    }

    // Update last_login_at
    let update_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE platform_users SET last_login_at = now(), updated_at = now() WHERE id = $1",
        [user_uuid.into()],
    );
    let _ = conn.execute_raw(update_stmt).await;

    // Issue platform access token
    let access_token = encode_platform_token(user_uuid, &role, &settings.jwt_secret, settings.access_ttl_secs)?;

    Ok(PlatformLoginResponse {
        access_token,
        role,
        expires_in: 900,
    })
}

// ----------------------------------------------------------------------------
// Impersonation
// ----------------------------------------------------------------------------

pub async fn open_impersonation_session<C: ConnectionTrait>(
    conn: &C,
    platform_user_id: Uuid,
    tenant_id: TenantId,
    reason: &str,
    duration_secs: i64,
) -> Result<Uuid, AppError> {
    if reason.trim().is_empty() {
        return Err(AppError::bad_request(
            "An impersonation session requires a stated reason",
        ));
    }

    let session_id = Uuid::now_v7();
    let expires_at = Utc::now() + Duration::seconds(duration_secs);

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO impersonation_sessions
           (id, platform_user_id, tenant_id, reason, writes_enabled_at, expires_at, created_at)
           VALUES ($1, $2, $3, $4, NULL, $5, now())"#,
        [
            session_id.into(),
            platform_user_id.into(),
            tenant_id.as_uuid().into(),
            reason.into(),
            expires_at.into(),
        ],
    );
    conn.execute_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to open impersonation session", e.to_string()))?;

    Ok(session_id)
}

pub async fn enable_impersonation_writes<C: ConnectionTrait>(
    conn: &C,
    session_id: Uuid,
    platform_user_id: Uuid,
) -> Result<(), AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"UPDATE impersonation_sessions
           SET writes_enabled_at = now()
           WHERE id = $1 AND platform_user_id = $2 AND ended_at IS NULL AND expires_at > now()"#,
        [session_id.into(), platform_user_id.into()],
    );
    let res = conn
        .execute_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to enable impersonation writes", e.to_string()))?;

    if res.rows_affected() == 0 {
        return Err(AppError::not_found(
            "Active impersonation session not found",
        ));
    }

    Ok(())
}

pub async fn get_impersonation_session<C: ConnectionTrait>(
    conn: &C,
    session_id: Uuid,
) -> Result<Option<ImpersonationSessionItem>, AppError> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, platform_user_id, reason, writes_enabled_at, expires_at, ended_at, created_at
           FROM impersonation_sessions
           WHERE id = $1"#,
        [session_id.into()],
    );
    let row = conn
        .query_one_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query impersonation session", e.to_string()))?;

    match row {
        Some(r) => Ok(Some(ImpersonationSessionItem {
            id: r.try_get_by_index(0).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            platform_user_id: r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            reason: r.try_get_by_index(2).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            writes_enabled_at: r.try_get_by_index(3).ok(),
            expires_at: r.try_get_by_index(4).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            ended_at: r.try_get_by_index(5).ok(),
            created_at: r.try_get_by_index(6).map_err(|e| AppError::internal("DB error", e.to_string()))?,
        })),
        None => Ok(None),
    }
}

pub async fn list_impersonation_sessions_for_tenant(
    tx: &Tx,
) -> Result<Vec<ImpersonationSessionItem>, AppError> {
    let tenant_id = tx.tenant_id()?;
    let conn = tx.transaction()?;

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT id, platform_user_id, reason, writes_enabled_at, expires_at, ended_at, created_at
           FROM impersonation_sessions
           WHERE tenant_id = $1
           ORDER BY created_at DESC"#,
        [tenant_id.as_uuid().into()],
    );
    let rows = conn
        .query_all_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to query impersonation sessions", e.to_string()))?;

    let mut items = Vec::new();
    for r in rows {
        items.push(ImpersonationSessionItem {
            id: r.try_get_by_index(0).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            platform_user_id: r.try_get_by_index(1).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            reason: r.try_get_by_index(2).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            writes_enabled_at: r.try_get_by_index(3).ok(),
            expires_at: r.try_get_by_index(4).map_err(|e| AppError::internal("DB error", e.to_string()))?,
            ended_at: r.try_get_by_index(5).ok(),
            created_at: r.try_get_by_index(6).map_err(|e| AppError::internal("DB error", e.to_string()))?,
        });
    }

    Ok(items)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DualIdentityAudit {
    pub tenant_id: TenantId,
    pub platform_user_id: Uuid,
    pub performed_by_user_id: Option<pos_core::ids::UserId>,
    pub reason: String,
}

pub async fn execute_impersonated_write<C: ConnectionTrait, F, R>(
    conn: &C,
    session_id: Uuid,
    platform_user_id: Uuid,
    action: F,
) -> Result<(R, DualIdentityAudit), AppError>
where
    F: FnOnce(&DualIdentityAudit) -> Result<R, AppError>,
{
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"SELECT tenant_id, reason, writes_enabled_at, expires_at, ended_at
           FROM impersonation_sessions
           WHERE id = $1 AND platform_user_id = $2"#,
        [session_id.into(), platform_user_id.into()],
    );
    let row = conn
        .query_one_raw(stmt)
        .await
        .map_err(|e| AppError::internal("Failed to check impersonation session", e.to_string()))?;

    let row = match row {
        Some(r) => r,
        None => return Err(AppError::unauthorized("Impersonation session not found")),
    };

    let tenant_uuid: Uuid = row
        .try_get_by_index(0)
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;
    let reason: String = row
        .try_get_by_index(1)
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;
    let writes_enabled_at: Option<DateTime<Utc>> = row.try_get_by_index(2).ok();
    let expires_at: DateTime<Utc> = row
        .try_get_by_index(3)
        .map_err(|e| AppError::internal("DB error", e.to_string()))?;
    let ended_at: Option<DateTime<Utc>> = row.try_get_by_index(4).ok();

    if ended_at.is_some() || expires_at < Utc::now() {
        return Err(AppError::unauthorized("Impersonation session has expired or ended"));
    }

    if writes_enabled_at.is_none() {
        return Err(AppError::forbidden(
            "Impersonation session is read-only; writes are not enabled",
        ));
    }

    let audit = DualIdentityAudit {
        tenant_id: TenantId::from(tenant_uuid),
        platform_user_id,
        performed_by_user_id: None,
        reason,
    };

    let result = action(&audit)?;
    Ok((result, audit))
}
