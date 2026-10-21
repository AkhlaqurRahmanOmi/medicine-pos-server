use chrono::{DateTime, Utc};

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyClock;
    impl Clock for DummyClock {
        fn now(&self) -> DateTime<Utc> {
            DateTime::UNIX_EPOCH
        }
    }

    #[test]
    fn clock_trait_is_usable() {
        let clock = DummyClock;
        assert_eq!(clock.now(), DateTime::UNIX_EPOCH);
    }
}
