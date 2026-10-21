use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[repr(transparent)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Money(Decimal);

impl Money {
    pub const ZERO: Self = Self(Decimal::ZERO);

    #[must_use]
    pub fn from_decimal(mut d: Decimal) -> Self {
        d.rescale(2);
        Self(d)
    }

    #[must_use]
    pub fn from_cents(cents: i64) -> Self {
        Self::from_decimal(Decimal::new(cents, 2))
    }

    #[must_use]
    pub const fn as_decimal(&self) -> Decimal {
        self.0
    }

    #[must_use]
    pub fn checked_add(&self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self::from_decimal)
    }

    #[must_use]
    pub fn checked_sub(&self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self::from_decimal)
    }

    #[must_use]
    pub fn checked_mul(&self, factor: Decimal) -> Option<Self> {
        self.0.checked_mul(factor).map(Self::from_decimal)
    }

    #[must_use]
    pub fn checked_div(&self, divisor: Decimal) -> Option<Self> {
        self.0.checked_div(divisor).map(Self::from_decimal)
    }
}

impl FromStr for Money {
    type Err = rust_decimal::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let d = Decimal::from_str(s)?;
        Ok(Self::from_decimal(d))
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "৳{:.2}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_keeps_two_decimal_places() {
        let m1 = Money::from_str("10.5").expect("valid decimal");
        assert_eq!(m1.as_decimal().scale(), 2);
        assert_eq!(format!("{m1}"), "৳10.50");

        let m2 = Money::from_str("10.5000").expect("valid decimal");
        assert_eq!(m2.as_decimal().scale(), 2);
        assert_eq!(format!("{m2}"), "৳10.50");

        let m3 = Money::from_cents(1250);
        assert_eq!(m3.as_decimal().scale(), 2);
        assert_eq!(format!("{m3}"), "৳12.50");

        let sum = m1.checked_add(m3).expect("checked add");
        assert_eq!(sum.as_decimal().scale(), 2);
        assert_eq!(format!("{sum}"), "৳23.00");
    }
}
