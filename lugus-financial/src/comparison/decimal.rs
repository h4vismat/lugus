use super::PercentageResult;
use crate::domain::Decimal;
use num_bigint::BigInt;
use num_traits::{Signed, Zero};
fn parse(d: &Decimal) -> Result<(BigInt, BigInt), &'static str> {
    let s = d.as_str();
    let frac = s.split('.').nth(1).map_or(0, str::len);
    if s.len() > 128 || frac > 18 {
        return Err("numeric_limit");
    }
    let digits = s.replace('.', "");
    Ok((
        digits.parse().map_err(|_| "numeric_limit")?,
        BigInt::from(10u8).pow(frac as u32),
    ))
}
fn gcd(mut a: BigInt, mut b: BigInt) -> BigInt {
    while !b.is_zero() {
        let r = &a % &b;
        a = b;
        b = r;
    }
    a
}
fn rounded(n: &BigInt, d: &BigInt, places: u32) -> String {
    let scaled = n.abs() * BigInt::from(10u8).pow(places);
    let mut q = &scaled / d;
    let r = &scaled % d;
    let twice = r * 2u8;
    if twice > *d || (twice == *d && &q % 2u8 != BigInt::ZERO) {
        q += 1u8;
    }
    let negative = n.is_negative() && !q.is_zero();
    let digits = format!("{:0>width$}", q, width = places as usize + 1);
    let cut = digits.len() - places as usize;
    format!(
        "{}{}.{}",
        if negative { "-" } else { "" },
        &digits[..cut],
        &digits[cut..]
    )
}
fn percentage(n: BigInt, d: BigInt) -> Result<PercentageResult, &'static str> {
    if d <= BigInt::ZERO {
        return Err("nonpositive_denominator");
    }
    let n = n * 100u8;
    let g = gcd(n.abs(), d.clone());
    let n = n / &g;
    let d = d / g;
    let value = rounded(&n, &d, 8);
    let value = value.trim_end_matches('0').trim_end_matches('.');
    Ok(PercentageResult {
        numerator: n.to_string(),
        denominator: d.to_string(),
        value: Decimal::new(value).expect("rounded decimal"),
        display: rounded(&n, &d, 2),
    })
}
pub(super) fn ratio_percent(n: &Decimal, d: &Decimal) -> Result<PercentageResult, &'static str> {
    let (n, ns) = parse(n)?;
    let (d, ds) = parse(d)?;
    percentage(n * ds, d * ns)
}
pub(super) fn growth_percent(
    current: &Decimal,
    prior: &Decimal,
) -> Result<PercentageResult, &'static str> {
    let (c, cs) = parse(current)?;
    let (p, ps) = parse(prior)?;
    percentage(c * &ps - &p * &cs, p * cs)
}
