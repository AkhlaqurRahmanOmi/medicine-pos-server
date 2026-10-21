use chrono::{DateTime, Duration, Utc};
use pos_core::Clock;
use std::sync::atomic::{AtomicI64, Ordering};

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug)]
pub struct FixedClock {
    millis: AtomicI64,
}

impl FixedClock {
    #[must_use]
    pub fn new(time: DateTime<Utc>) -> Self {
        Self {
            millis: AtomicI64::new(time.timestamp_millis()),
        }
    }

    pub fn set(&self, time: DateTime<Utc>) {
        self.millis.store(time.timestamp_millis(), Ordering::SeqCst);
    }

    pub fn advance(&self, duration: Duration) {
        self.millis
            .fetch_add(duration.num_milliseconds(), Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        let ms = self.millis.load(Ordering::SeqCst);
        DateTime::from_timestamp_millis(ms).unwrap_or(DateTime::UNIX_EPOCH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_is_deterministic() {
        let fixed_time = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let clock = FixedClock::new(fixed_time);

        assert_eq!(clock.now(), fixed_time);
        assert_eq!(clock.now(), fixed_time);

        clock.advance(Duration::seconds(60));
        assert_eq!(clock.now(), fixed_time + Duration::seconds(60));
    }
}
