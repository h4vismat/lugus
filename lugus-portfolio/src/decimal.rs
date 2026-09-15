use crate::{PortfolioError, Result};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
/// Checked scale-18 decimal, at most 38 coefficient digits. Never binary float.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Decimal {
    coefficient: BigInt,
}
fn scale() -> BigInt {
    BigInt::from(10u8).pow(18)
}
fn rounded(n: BigInt, d: BigInt) -> Result<BigInt> {
    if d.is_zero() {
        return Err(PortfolioError::InvalidNumber);
    }
    let negative = n.is_negative() != d.is_negative();
    let n = n.abs();
    let d = d.abs();
    let mut q = &n / &d;
    let r = &n % &d;
    let twice = r * 2u8;
    if twice > d || (twice == d && &q % 2u8 != BigInt::ZERO) {
        q += 1u8;
    }
    Ok(if negative { -q } else { q })
}
impl Decimal {
    fn from_coefficient(coefficient: BigInt) -> Result<Self> {
        if coefficient.abs() >= BigInt::from(10u8).pow(38) {
            return Err(PortfolioError::Overflow);
        }
        Ok(Self { coefficient })
    }
    pub fn parse(value: &str) -> Result<Self> {
        if value.is_empty() || value.len() > 64 {
            return Err(PortfolioError::InvalidNumber);
        }
        let raw = value.strip_prefix('-').unwrap_or(value);
        let mut parts = raw.split('.');
        let whole = parts.next().unwrap_or("");
        let frac = parts.next();
        if whole.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || parts.next().is_some()
            || frac.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(PortfolioError::InvalidNumber);
        }
        let frac = frac.unwrap_or("");
        if frac.len() > 18 {
            return Err(PortfolioError::Precision);
        }
        let digits = format!("{whole}{frac}{}", "0".repeat(18 - frac.len()));
        let mut coefficient =
            BigInt::parse_bytes(digits.as_bytes(), 10).ok_or(PortfolioError::InvalidNumber)?;
        if value.starts_with('-') {
            coefficient = -coefficient;
        }
        Self::from_coefficient(coefficient)
    }
    pub fn zero() -> Self {
        Self::default()
    }
    pub fn is_zero(&self) -> bool {
        self.coefficient.is_zero()
    }
    pub fn is_negative(&self) -> bool {
        self.coefficient.is_negative()
    }
    pub fn is_positive(&self) -> bool {
        self.coefficient > BigInt::ZERO
    }
    pub fn is_cents(&self) -> bool {
        (&self.coefficient % BigInt::from(10u8).pow(16)).is_zero()
    }
    pub fn checked_add(&self, b: &Self) -> Result<Self> {
        Self::from_coefficient(&self.coefficient + &b.coefficient)
    }
    pub fn checked_sub(&self, b: &Self) -> Result<Self> {
        Self::from_coefficient(&self.coefficient - &b.coefficient)
    }
    pub fn checked_mul(&self, b: &Self) -> Result<Self> {
        Self::from_coefficient(rounded(&self.coefficient * &b.coefficient, scale())?)
    }
    pub fn allocated(&self, n: &Self, d: &Self) -> Result<Self> {
        Self::from_coefficient(rounded(
            &self.coefficient * &n.coefficient,
            d.coefficient.clone(),
        )?)
    }
    pub fn round_cents(&self) -> Result<Self> {
        let unit = BigInt::from(10u8).pow(16);
        Self::from_coefficient(rounded(self.coefficient.clone(), unit.clone())? * unit)
    }
    /// Apply an exact rational action chain, rounding only its final scale-18 value.
    pub fn adjusted_by_splits(&self, splits: &[(u64, u64)]) -> Result<Self> {
        crate::require(splits.len() <= 1000, "split action chain exceeds limit")?;
        let mut numerator = self.coefficient.clone();
        let mut denominator = BigInt::from(1u8);
        for (n, d) in splits {
            if *n == 0 || *d == 0 {
                return Err(PortfolioError::InvalidNumber);
            }
            numerator *= *n;
            denominator *= *d;
        }
        Self::from_coefficient(rounded(numerator, denominator)?)
    }
    pub fn split_exact(&self, n: u64, d: u64) -> Result<Self> {
        if n == 0 || d == 0 {
            return Err(PortfolioError::InvalidNumber);
        }
        let v = &self.coefficient * n;
        if &v % d != BigInt::ZERO {
            return Err(PortfolioError::Precision);
        }
        Self::from_coefficient(v / d)
    }
}
impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits = format!("{:0>19}", self.coefficient.abs().to_string());
        let cut = digits.len() - 18;
        let frac = digits[cut..].trim_end_matches('0');
        if self.is_negative() {
            write!(f, "-")?;
        }
        write!(f, "{}", &digits[..cut])?;
        if !frac.is_empty() {
            write!(f, ".{frac}")?;
        }
        Ok(())
    }
}
impl Serialize for Decimal {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Self::parse(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
