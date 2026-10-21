#[must_use]
pub fn prefixed_key(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}:{key}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_prefixed() {
        let prefix = "medicine_pos";
        let key = "rate_limit:login:user1";
        let full_key = prefixed_key(prefix, key);
        assert_eq!(full_key, "medicine_pos:rate_limit:login:user1");
        assert!(full_key.starts_with(prefix));
    }
}
