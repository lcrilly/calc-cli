//! The default backend, with no dependencies: exact i128 rationals, and f64 for
//! everything else. Exact operations return `None` on overflow, so the caller can
//! fall back to f64.

use std::cmp::Ordering;

use crate::error::Error;
use crate::format::{Decimal, Precision};

/// Factorials beyond this overflow an f64.
pub const MAX_FACTORIAL: u64 = 170;

/// f64 has a fixed precision, so there is nothing to set.
pub fn set_precision(_: Precision) {}

/// Invariant: `den > 0` and `gcd(num, den) == 1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ratio {
    num: i128,
    den: i128,
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Ratio {
    /// Builds a normalised ratio; `None` on overflow. `den` must be non-zero.
    fn new(mut num: i128, mut den: i128) -> Option<Ratio> {
        let g = i128::try_from(gcd(num.unsigned_abs(), den.unsigned_abs())).ok()?;
        num /= g;
        den /= g;
        if den < 0 {
            num = num.checked_neg()?;
            den = den.checked_neg()?;
        }
        Some(Ratio { num, den })
    }

    pub fn from_int(n: i64) -> Ratio {
        Ratio { num: n.into(), den: 1 }
    }

    /// The digits of a decimal literal either side of the point; `None` if it doesn't fit.
    pub fn from_digits(int: &str, frac: &str) -> Option<Ratio> {
        let digits: i128 = format!("{int}{frac}").parse().ok()?;
        let scale = 10i128.checked_pow(u32::try_from(frac.len()).ok()?)?;
        Ratio::new(digits, scale)
    }

    pub fn checked_add(&self, rhs: &Ratio) -> Option<Ratio> {
        let g = gcd(self.den as u128, rhs.den as u128) as i128;
        let num = self.num.checked_mul(rhs.den / g)?.checked_add(rhs.num.checked_mul(self.den / g)?)?;
        Ratio::new(num, (self.den / g).checked_mul(rhs.den)?)
    }

    pub fn checked_mul(&self, rhs: &Ratio) -> Option<Ratio> {
        // Cross-reduce first to keep intermediates small.
        let g1 = gcd(self.num.unsigned_abs(), rhs.den as u128) as i128;
        let g2 = gcd(rhs.num.unsigned_abs(), self.den as u128) as i128;
        let num = (self.num / g1).checked_mul(rhs.num / g2)?;
        let den = (self.den / g2).checked_mul(rhs.den / g1)?;
        Ratio::new(num, den)
    }

    /// `None` for zero, as well as on overflow.
    pub fn checked_recip(&self) -> Option<Ratio> {
        if self.num == 0 {
            return None;
        }
        Ratio::new(self.den, self.num)
    }

    pub fn checked_neg(&self) -> Option<Ratio> {
        Some(Ratio { num: self.num.checked_neg()?, den: self.den })
    }

    /// Remainder of floored division, so it takes the sign of `rhs`. `rhs` must be non-zero.
    pub fn checked_rem(&self, rhs: &Ratio) -> Option<Ratio> {
        let q = self.checked_mul(&rhs.checked_recip()?)?;
        let floor = Ratio { num: q.num.div_euclid(q.den), den: 1 };
        self.checked_add(&rhs.checked_mul(&floor)?.checked_neg()?)
    }

    /// `self` to a whole-number power.
    pub fn checked_powi(&self, exp: &Ratio) -> Option<Ratio> {
        if exp.den != 1 {
            return None;
        }
        let e = u32::try_from(exp.num.unsigned_abs()).ok()?;
        // Powers of coprime numbers stay coprime, so no re-normalising needed.
        let r = Ratio { num: self.num.checked_pow(e)?, den: self.den.checked_pow(e)? };
        if exp.num < 0 { r.checked_recip() } else { Some(r) }
    }

    /// The exact square root, when there is one.
    pub fn checked_sqrt(&self) -> Option<Ratio> {
        if self.num < 0 {
            return None;
        }
        let (n, d) = (self.num.isqrt(), self.den.isqrt());
        (n * n == self.num && d * d == self.den).then_some(Ratio { num: n, den: d })
    }

    pub fn checked_cmp(&self, rhs: &Ratio) -> Option<Ordering> {
        Some(self.num.checked_mul(rhs.den)?.cmp(&rhs.num.checked_mul(self.den)?))
    }

    pub fn is_zero(&self) -> bool {
        self.num == 0
    }

    pub fn is_negative(&self) -> bool {
        self.num < 0
    }

    pub fn is_integer(&self) -> bool {
        self.den == 1
    }

    pub fn floor(&self) -> Option<Ratio> {
        Some(Ratio { num: self.num.div_euclid(self.den), den: 1 })
    }

    /// Rounds half away from zero.
    pub fn round(&self) -> Option<Ratio> {
        let n = self.num.unsigned_abs().checked_mul(2)?.checked_add(self.den as u128)?;
        let q = i128::try_from(n / (self.den as u128 * 2)).ok()?;
        Some(Ratio { num: if self.num < 0 { -q } else { q }, den: 1 })
    }

    pub fn to_u64(&self) -> Option<u64> {
        if self.den == 1 { u64::try_from(self.num).ok() } else { None }
    }

    pub fn to_float(&self) -> Float {
        Float(self.num as f64 / self.den as f64)
    }

    pub fn format(&self, precision: Precision) -> Decimal {
        if self.den == 1 {
            return Decimal { negative: self.num < 0, int: self.num.unsigned_abs().to_string(), frac: String::new(), exp: None };
        }
        let places = match (precision, terminating_places(self.den)) {
            (Precision::Places(n), _) => Some(n),
            (_, Some(n)) => Some(n),
            (Precision::Default, None) => Some(crate::format::DEFAULT_PLACES),
            (Precision::Max, None) => None,
        };
        places.and_then(|n| self.round_to(n)).unwrap_or_else(|| self.to_float().format(precision))
    }

    /// Exact long division to `places` digits, rounding half away from zero.
    /// `None` if an intermediate would overflow, so the caller can fall back to f64.
    fn round_to(&self, places: usize) -> Option<Decimal> {
        let den = self.den as u128;
        let n = self.num.unsigned_abs();
        let mut int = n / den;
        let mut rem = n % den;

        let mut digits = Vec::with_capacity(places);
        for _ in 0..places {
            rem = rem.checked_mul(10)?;
            digits.push((rem / den) as u8);
            rem %= den;
        }

        if rem * 2 >= den {
            let mut carry = true;
            for d in digits.iter_mut().rev() {
                if *d == 9 {
                    *d = 0;
                } else {
                    *d += 1;
                    carry = false;
                    break;
                }
            }
            if carry {
                int = int.checked_add(1)?;
            }
        }

        let frac = digits.iter().map(|&d| char::from(b'0' + d)).collect();
        Some(Decimal { negative: self.num < 0, int: int.to_string(), frac, exp: None }.tidy())
    }
}

/// Number of decimal places needed if `1/den` terminates (den = 2^a·5^b), else `None`.
fn terminating_places(mut den: i128) -> Option<usize> {
    let (mut twos, mut fives) = (0, 0);
    while den % 2 == 0 {
        den /= 2;
        twos += 1;
    }
    while den % 5 == 0 {
        den /= 5;
        fives += 1;
    }
    (den == 1).then_some(twos.max(fives))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Float(f64);

/// A finite f64, or the reason there isn't one.
fn checked(f: f64) -> Result<Float, Error> {
    if f.is_nan() {
        Err(Error::new("undefined result"))
    } else if f.is_infinite() {
        Err(Error::new("result out of range"))
    } else {
        Ok(Float(f))
    }
}

/// Beyond this an f64's integer digits are mostly noise, so switch to scientific notation.
const SCIENTIFIC_THRESHOLD: f64 = 1e16;

impl Float {
    pub fn pi() -> Float {
        Float(std::f64::consts::PI)
    }

    pub fn degree() -> Float {
        Float(std::f64::consts::PI / 180.0)
    }

    pub fn parse(literal: &str) -> Option<Float> {
        literal.parse().ok().map(Float)
    }

    pub fn add(&self, rhs: &Float) -> Result<Float, Error> {
        checked(self.0 + rhs.0)
    }

    pub fn mul(&self, rhs: &Float) -> Result<Float, Error> {
        checked(self.0 * rhs.0)
    }

    pub fn div(&self, rhs: &Float) -> Result<Float, Error> {
        checked(self.0 / rhs.0)
    }

    /// Floored remainder, taking the sign of `rhs`.
    pub fn rem(&self, rhs: &Float) -> Result<Float, Error> {
        let (a, b) = (self.0, rhs.0);
        let r = a % b;
        checked(if r != 0.0 && (r < 0.0) != (b < 0.0) { r + b } else { r })
    }

    pub fn powf(&self, exp: &Float) -> Result<Float, Error> {
        checked(self.0.powf(exp.0))
    }

    pub fn neg(&self) -> Float {
        Float(-self.0)
    }

    pub fn floor(&self) -> Float {
        Float(self.0.floor())
    }

    /// Rounds half away from zero.
    pub fn round(&self) -> Float {
        Float(self.0.round())
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0.0
    }

    pub fn is_negative(&self) -> bool {
        self.0 < 0.0
    }

    pub fn is_integer(&self) -> bool {
        self.0.fract() == 0.0
    }

    pub fn to_u64(&self) -> Option<u64> {
        (self.is_integer() && self.0 >= 0.0 && self.0 <= u64::MAX as f64).then_some(self.0 as u64)
    }

    pub fn total_cmp(&self, rhs: &Float) -> Ordering {
        self.0.total_cmp(&rhs.0)
    }

    pub fn sqrt(&self) -> Result<Float, Error> {
        checked(self.0.sqrt())
    }
    pub fn ln(&self) -> Result<Float, Error> {
        checked(self.0.ln())
    }
    pub fn log10(&self) -> Result<Float, Error> {
        checked(self.0.log10())
    }
    pub fn exp(&self) -> Result<Float, Error> {
        checked(self.0.exp())
    }
    pub fn sin(&self) -> Result<Float, Error> {
        checked(self.0.sin())
    }
    pub fn cos(&self) -> Result<Float, Error> {
        checked(self.0.cos())
    }
    pub fn tan(&self) -> Result<Float, Error> {
        checked(self.0.tan())
    }
    pub fn asin(&self) -> Result<Float, Error> {
        checked(self.0.asin())
    }
    pub fn acos(&self) -> Result<Float, Error> {
        checked(self.0.acos())
    }
    pub fn atan(&self) -> Result<Float, Error> {
        checked(self.0.atan())
    }
    pub fn sinh(&self) -> Result<Float, Error> {
        checked(self.0.sinh())
    }
    pub fn cosh(&self) -> Result<Float, Error> {
        checked(self.0.cosh())
    }
    pub fn tanh(&self) -> Result<Float, Error> {
        checked(self.0.tanh())
    }

    pub fn format(&self, precision: Precision) -> Decimal {
        use crate::format::DEFAULT_PLACES;
        let f = self.0;
        let s = if f.abs() >= SCIENTIFIC_THRESHOLD {
            match precision {
                Precision::Default => format!("{f:.DEFAULT_PLACES$e}"),
                Precision::Places(n) => format!("{f:.n$e}"),
                Precision::Max => format!("{f:e}"),
            }
        } else {
            match precision {
                Precision::Default => format!("{f:.DEFAULT_PLACES$}"),
                Precision::Places(n) => format!("{f:.n$}"),
                Precision::Max => format!("{f}"),
            }
        };
        Decimal::from_std(&s)
    }
}
