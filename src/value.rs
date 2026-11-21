//! Numbers are exact rationals while possible and fall back to f64 once a result
//! is irrational (π, non-perfect square roots) or overflows i128.

use std::cmp::Ordering;

use crate::error::Error;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Exact(Ratio),
    Approx(f64),
}

/// Invariant: `den > 0` and `gcd(num, den) == 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ratio {
    pub num: i128,
    pub den: i128,
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Ratio {
    /// Builds a normalised ratio; `None` on overflow. `den` must be non-zero.
    pub fn new(mut num: i128, mut den: i128) -> Option<Ratio> {
        let g = i128::try_from(gcd(num.unsigned_abs(), den.unsigned_abs())).ok()?;
        num /= g;
        den /= g;
        if den < 0 {
            num = num.checked_neg()?;
            den = den.checked_neg()?;
        }
        Some(Ratio { num, den })
    }

    fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    fn checked_add(self, rhs: Ratio) -> Option<Ratio> {
        let g = gcd(self.den as u128, rhs.den as u128) as i128;
        let num = self
            .num
            .checked_mul(rhs.den / g)?
            .checked_add(rhs.num.checked_mul(self.den / g)?)?;
        Ratio::new(num, (self.den / g).checked_mul(rhs.den)?)
    }

    fn checked_mul(self, rhs: Ratio) -> Option<Ratio> {
        // Cross-reduce first to keep intermediates small.
        let g1 = gcd(self.num.unsigned_abs(), rhs.den as u128) as i128;
        let g2 = gcd(rhs.num.unsigned_abs(), self.den as u128) as i128;
        let num = (self.num / g1).checked_mul(rhs.num / g2)?;
        let den = (self.den / g2).checked_mul(rhs.den / g1)?;
        Ratio::new(num, den)
    }

    fn checked_recip(self) -> Option<Ratio> {
        if self.num == 0 {
            return None;
        }
        Ratio::new(self.den, self.num)
    }

    /// Remainder of floored division, so it takes the sign of `rhs`. `rhs` must be non-zero.
    fn checked_rem(self, rhs: Ratio) -> Option<Ratio> {
        let q = self.checked_mul(rhs.checked_recip()?)?;
        let floor = Ratio { num: q.num.div_euclid(q.den), den: 1 };
        self.checked_add(rhs.checked_mul(floor)?.checked_neg()?)
    }

    fn checked_neg(self) -> Option<Ratio> {
        Some(Ratio { num: self.num.checked_neg()?, den: self.den })
    }

    fn checked_powi(self, exp: i128) -> Option<Ratio> {
        let e = u32::try_from(exp.unsigned_abs()).ok()?;
        // Powers of coprime numbers stay coprime, so no re-normalising needed.
        let r = Ratio { num: self.num.checked_pow(e)?, den: self.den.checked_pow(e)? };
        if exp < 0 { r.checked_recip() } else { Some(r) }
    }

    fn checked_sqrt(self) -> Option<Ratio> {
        let (n, d) = (self.num.isqrt(), self.den.isqrt());
        (n * n == self.num && d * d == self.den).then_some(Ratio { num: n, den: d })
    }
}

pub fn approx(f: f64) -> Result<Value, Error> {
    if f.is_nan() {
        Err(Error::new("undefined result"))
    } else if f.is_infinite() {
        Err(Error::new("result out of range"))
    } else {
        Ok(Value::Approx(f))
    }
}

impl Value {
    pub fn to_f64(self) -> f64 {
        match self {
            Value::Exact(r) => r.to_f64(),
            Value::Approx(f) => f,
        }
    }

    fn is_zero(self) -> bool {
        match self {
            Value::Exact(r) => r.num == 0,
            Value::Approx(f) => f == 0.0,
        }
    }

    pub fn is_negative(self) -> bool {
        match self {
            Value::Exact(r) => r.num < 0,
            Value::Approx(f) => f < 0.0,
        }
    }

    pub fn add(self, rhs: Value) -> Result<Value, Error> {
        if let (Value::Exact(a), Value::Exact(b)) = (self, rhs)
            && let Some(r) = a.checked_add(b)
        {
            return Ok(Value::Exact(r));
        }
        approx(self.to_f64() + rhs.to_f64())
    }

    pub fn sub(self, rhs: Value) -> Result<Value, Error> {
        self.add(rhs.neg())
    }

    pub fn mul(self, rhs: Value) -> Result<Value, Error> {
        if let (Value::Exact(a), Value::Exact(b)) = (self, rhs)
            && let Some(r) = a.checked_mul(b)
        {
            return Ok(Value::Exact(r));
        }
        approx(self.to_f64() * rhs.to_f64())
    }

    pub fn div(self, rhs: Value) -> Result<Value, Error> {
        if rhs.is_zero() {
            return Err(Error::new("division by zero"));
        }
        if let (Value::Exact(a), Value::Exact(b)) = (self, rhs)
            && let Some(r) = b.checked_recip().and_then(|b| a.checked_mul(b))
        {
            return Ok(Value::Exact(r));
        }
        approx(self.to_f64() / rhs.to_f64())
    }

    /// Modulo with the sign of the divisor (floored, as in Python): `-7 mod 3` is 2.
    pub fn rem(self, rhs: Value) -> Result<Value, Error> {
        if rhs.is_zero() {
            return Err(Error::new("modulo by zero"));
        }
        if let (Value::Exact(a), Value::Exact(b)) = (self, rhs)
            && let Some(r) = a.checked_rem(b)
        {
            return Ok(Value::Exact(r));
        }
        let (a, b) = (self.to_f64(), rhs.to_f64());
        let r = a % b;
        approx(if r != 0.0 && (r < 0.0) != (b < 0.0) { r + b } else { r })
    }

    pub fn pow(self, exp: Value) -> Result<Value, Error> {
        if self.is_zero() && exp.is_negative() {
            return Err(Error::new("division by zero"));
        }
        if let (Value::Exact(base), Value::Exact(e)) = (self, exp)
            && e.den == 1
            && let Some(r) = base.checked_powi(e.num)
        {
            return Ok(Value::Exact(r));
        }
        approx(self.to_f64().powf(exp.to_f64()))
    }

    pub fn from_int(n: i128) -> Value {
        Value::Exact(Ratio { num: n, den: 1 })
    }

    /// Total order, as `f64::total_cmp`; exact values compare exactly where the products fit.
    pub fn total_cmp(self, rhs: Value) -> Ordering {
        if let (Value::Exact(a), Value::Exact(b)) = (self, rhs)
            && let (Some(l), Some(r)) = (a.num.checked_mul(b.den), b.num.checked_mul(a.den))
        {
            return l.cmp(&r);
        }
        self.to_f64().total_cmp(&rhs.to_f64())
    }

    pub fn abs(self) -> Value {
        if self.is_negative() { self.neg() } else { self }
    }

    pub fn floor(self) -> Value {
        match self {
            Value::Exact(r) => Value::from_int(r.num.div_euclid(r.den)),
            Value::Approx(f) => Value::Approx(f.floor()),
        }
    }

    pub fn ceil(self) -> Value {
        self.neg().floor().neg()
    }

    /// Rounds half away from zero, as spreadsheets do.
    pub fn round(self) -> Value {
        match self {
            Value::Exact(r) => {
                let rounded = || {
                    let n = r.num.unsigned_abs().checked_mul(2)?.checked_add(r.den as u128)?;
                    let q = i128::try_from(n / (r.den as u128 * 2)).ok()?;
                    Some(if r.num < 0 { -q } else { q })
                };
                rounded().map_or(Value::Approx(r.to_f64().round()), Value::from_int)
            }
            Value::Approx(f) => Value::Approx(f.round()),
        }
    }

    /// n! for a whole number n: exact while it fits in i128 (up to 33!), then f64 up to 170!.
    pub fn factorial(self) -> Result<Value, Error> {
        let n = match self {
            Value::Exact(r) if r.den == 1 && r.num >= 0 => r.num,
            Value::Approx(f) if f >= 0.0 && f.fract() == 0.0 => f.min(171.0) as i128,
            _ => return Err(Error::new("factorial needs a whole number ≥ 0")),
        };
        if n > 170 {
            return Err(Error::new("result out of range"));
        }
        let (mut exact, mut float) = (Some(1i128), 1.0f64);
        for k in 2..=n {
            exact = exact.and_then(|p| p.checked_mul(k));
            float *= k as f64;
        }
        Ok(exact.map_or(Value::Approx(float), Value::from_int))
    }

    pub fn neg(self) -> Value {
        match self {
            Value::Exact(r) => r.checked_neg().map_or(Value::Approx(-r.to_f64()), Value::Exact),
            Value::Approx(f) => Value::Approx(-f),
        }
    }

    pub fn sqrt(self) -> Result<Value, Error> {
        if self.is_negative() {
            return Err(Error::new("square root of a negative number"));
        }
        if let Value::Exact(r) = self
            && let Some(root) = r.checked_sqrt()
        {
            return Ok(Value::Exact(root));
        }
        approx(self.to_f64().sqrt())
    }
}
