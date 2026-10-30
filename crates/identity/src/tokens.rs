use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use pos_core::error::AppError;
use pos_core::ids::{SessionId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const PHARMACY_AUDIENCE: &str = "pos:pharmacy";
pub const PLATFORM_AUDIENCE: &str = "pos:platform";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PharmacyClaims {
    pub sub: String,
    pub tenant_id: String,
    pub session_id: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformClaims {
    pub sub: String,
    pub role: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
}

pub fn encode_pharmacy_token(
    user_id: UserId,
    tenant_id: TenantId,
    session_id: SessionId,
    secret: &str,
    expires_in_secs: i64,
) -> Result<String, AppError> {
    let now = Utc::now().timestamp();
    let claims = PharmacyClaims {
        sub: user_id.to_string(),
        tenant_id: tenant_id.to_string(),
        session_id: session_id.to_string(),
        aud: PHARMACY_AUDIENCE.to_string(),
        exp: now + expires_in_secs,
        iat: now,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::internal("Failed to encode pharmacy token", e.to_string()))
}

pub fn decode_pharmacy_token(token: &str, secret: &str) -> Result<PharmacyClaims, AppError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&[PHARMACY_AUDIENCE]);
    validation.leeway = 0;
    let token_data = decode::<PharmacyClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
            AppError::new(pos_core::error::ErrorCode::TokenExpired, "Access token has expired")
        }
        _ => AppError::unauthorized(format!("Invalid token: {e}")),
    })?;
    Ok(token_data.claims)
}

pub fn encode_platform_token(
    platform_user_id: Uuid,
    role: &str,
    secret: &str,
    expires_in_secs: i64,
) -> Result<String, AppError> {
    let now = Utc::now().timestamp();
    let claims = PlatformClaims {
        sub: platform_user_id.to_string(),
        role: role.to_string(),
        aud: PLATFORM_AUDIENCE.to_string(),
        exp: now + expires_in_secs,
        iat: now,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::internal("Failed to encode platform token", e.to_string()))
}

pub fn decode_platform_token(token: &str, secret: &str) -> Result<PlatformClaims, AppError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&[PLATFORM_AUDIENCE]);
    validation.leeway = 0;
    let token_data = decode::<PlatformClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
            AppError::new(pos_core::error::ErrorCode::TokenExpired, "Platform token has expired")
        }
        _ => AppError::unauthorized(format!("Invalid platform token: {e}")),
    })?;
    Ok(token_data.claims)
}

pub fn generate_refresh_token() -> String {
    format!("rfr_{}", Uuid::now_v7())
}

pub fn hash_refresh_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pharmacy_token_carries_its_session_id() {
        let secret = "test-secret-key-12345";
        let user_id = UserId::new_v7();
        let tenant_id = TenantId::new_v7();
        let session_id = SessionId::new_v7();

        let token = encode_pharmacy_token(user_id, tenant_id, session_id, secret, 900).unwrap();
        let claims = decode_pharmacy_token(&token, secret).unwrap();

        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.tenant_id, tenant_id.to_string());
        assert_eq!(claims.session_id, session_id.to_string());
        assert_eq!(claims.aud, PHARMACY_AUDIENCE);
    }

    #[test]
    fn a_pharmacy_token_fails_platform_decoding() {
        let secret = "test-secret-key-12345";
        let user_id = UserId::new_v7();
        let tenant_id = TenantId::new_v7();
        let session_id = SessionId::new_v7();

        let pharmacy_token = encode_pharmacy_token(user_id, tenant_id, session_id, secret, 900).unwrap();

        // Decoding with platform validator MUST fail due to audience mismatch
        let result = decode_platform_token(&pharmacy_token, secret);
        assert!(result.is_err(), "Pharmacy token must fail platform decoding");

        // And vice versa
        let platform_token = encode_platform_token(Uuid::now_v7(), "super_admin", secret, 900).unwrap();
        let result_pharm = decode_pharmacy_token(&platform_token, secret);
        assert!(result_pharm.is_err(), "Platform token must fail pharmacy decoding");
    }

    #[test]
    fn the_token_claims_are_exactly_what_we_signed() {
        let secret = "test-secret-key-12345";
        let user_id = UserId::new_v7();
        let tenant_id = TenantId::new_v7();
        let session_id = SessionId::new_v7();

        let token = encode_pharmacy_token(user_id, tenant_id, session_id, secret, 900).unwrap();
        let claims = decode_pharmacy_token(&token, secret).unwrap();

        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.tenant_id, tenant_id.to_string());
        assert_eq!(claims.session_id, session_id.to_string());
        assert_eq!(claims.aud, PHARMACY_AUDIENCE);
    }

    #[test]
    fn an_expired_access_token_is_refused() {
        let secret = "test-secret-key-12345";
        let user_id = UserId::new_v7();
        let tenant_id = TenantId::new_v7();
        let session_id = SessionId::new_v7();

        // Expired in the past (-10 seconds)
        let token = encode_pharmacy_token(user_id, tenant_id, session_id, secret, -10).unwrap();
        let result = decode_pharmacy_token(&token, secret);
        assert!(result.is_err(), "Expired token must be refused");
        let err = result.unwrap_err();
        assert_eq!(err.code, pos_core::error::ErrorCode::TokenExpired);
    }
}
