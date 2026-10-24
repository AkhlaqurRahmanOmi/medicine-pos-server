use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use pos_core::error::AppError;
use std::sync::OnceLock;

static COMPUTED_DUMMY_HASH: OnceLock<String> = OnceLock::new();

pub fn get_dummy_hash() -> &'static str {
    COMPUTED_DUMMY_HASH.get_or_init(|| {
        hash_password("dummy-password-for-timing-safety").unwrap_or_else(|_| {
            "$argon2id$v=19$m=19456,t=2,p=1$ZHVtbXktc2FsdA$ZHVtbXktaGFzaA".to_string()
        })
    })
}

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes())
        .map_err(|e| AppError::internal("Failed to hash password", e.to_string()))?
        .to_string();
    Ok(password_hash)
}

pub fn verify_password(password: &str, hash_str: &str) -> bool {
    let parsed_hash = match PasswordHash::new(hash_str) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

pub fn verify_pin(pin: &str, hash_str: &str) -> bool {
    verify_password(pin, hash_str)
}

pub fn verify_password_or_dummy(password: &str, hash_str: Option<&str>) -> bool {
    match hash_str {
        Some(hash) => verify_password(password, hash),
        None => {
            let _ = verify_password(password, get_dummy_hash());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_verifies_against_its_own_hash_only() {
        let password = "CorrectHorseBatteryStaple123!";
        let hash = hash_password(password).unwrap();

        // 1. Verifies against its own hash
        assert!(verify_password(password, &hash));

        // 2. Fails against wrong password
        assert!(!verify_password("WrongPassword!", &hash));
        assert!(!verify_password("", &hash));

        // 3. PIN hashing also works
        let pin = "1234";
        let pin_h = hash_password(pin).unwrap();
        assert!(verify_pin(pin, &pin_h));
        assert!(!verify_pin("9999", &pin_h));
    }

    #[test]
    fn an_absent_user_is_verified_against_a_dummy_hash() {
        let dummy = get_dummy_hash();
        assert!(!dummy.is_empty());

        // Verifying an absent user runs argon2 against dummy hash and returns false
        let result = verify_password_or_dummy("any_password", None);
        assert!(!result);

        // Verify timing does argon2 work (takes > 0ms, properly executes)
        let start = std::time::Instant::now();
        verify_password_or_dummy("test_pwd", None);
        let elapsed = start.elapsed();
        assert!(elapsed.as_micros() > 500, "Argon2 must perform actual work");
    }
}
