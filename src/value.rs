//! Numbers are exact rationals while possible and fall back to floats once a result
//! is irrational (π, non-perfect square roots) or too large to keep exact. What
//! "too large" and "float" mean depends on the build: see `num`.

use std::cmp::Ordering;

use crate::error::Error;
use crate::num::{Float, MAX_FACTORIAL, Ratio};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Exact(Ratio),
    Approx(Float),
}

impl Value {
    pub fn from_int(n: i64) -> Value {
        Value::Exact(Ratio::from_int(n))
    }

    /// Reads a plain decimal literal (`12`, `3.25`, `.5`): exactly when it fits.
    pub fn parse(literal: &str) -> Option<Value> {
        let (int, frac) = literal.split_once('.').unwrap_or((literal, ""));
        let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if !digits(int) || !digits(frac) || (int.is_empty() && frac.is_empty()) {
            return None;
        }
        match Ratio::from_digits(int, frac) {
            Some(r) => Some(Value::Exact(r)),
            None => Float::parse(literal).map(Value::Approx),
        }
    }

    pub fn pi() -> Value {
        Value::Approx(Float::pi())
    }

    /// One degree, in radians.
    pub fn degree() -> Value {
        Value::Approx(Float::degree())
    }

    fn to_float(&self) -> Float {
        match self {
            Value::Exact(r) => r.to_float(),
            Value::Approx(f) => f.clone(),
        }
    }

    fn is_zero(&self) -> bool {
        match self {
            Value::Exact(r) => r.is_zero(),
            Value::Approx(f) => f.is_zero(),
        }
    }

    fn is_negative(&self) -> bool {
        match self {
            Value::Exact(r) => r.is_negative(),
            Value::Approx(f) => f.is_negative(),
        }
    }

    /// Runs `exact` when both values are exact, falling back to `float` when either
    /// isn't or the exact result doesn't fit.
    fn combine(
        &self,
        rhs: &Value,
        exact: impl Fn(&Ratio, &Ratio) -> Option<Ratio>,
        float: impl Fn(&Float, &Float) -> Result<Float, Error>,
    ) -> Result<Value, Error> {
        if let (Value::Exact(a), Value::Exact(b)) = (self, rhs)
            && let Some(r) = exact(a, b)
        {
            return Ok(Value::Exact(r));
        }
        float(&self.to_float(), &rhs.to_float()).map(Value::Approx)
    }

    pub fn add(self, rhs: Value) -> Result<Value, Error> {
        self.combine(&rhs, Ratio::checked_add, Float::add)
    }

    pub fn sub(self, rhs: Value) -> Result<Value, Error> {
        self.add(rhs.neg())
    }

    pub fn mul(self, rhs: Value) -> Result<Value, Error> {
        self.combine(&rhs, Ratio::checked_mul, Float::mul)
    }

    pub fn div(self, rhs: Value) -> Result<Value, Error> {
        if rhs.is_zero() {
            return Err(Error::new("division by zero"));
        }
        self.combine(&rhs, |a, b| a.checked_mul(&b.checked_recip()?), Float::div)
    }

    /// Modulo with the sign of the divisor (floored, as in Python): `-7 mod 3` is 2.
    pub fn rem(self, rhs: Value) -> Result<Value, Error> {
        if rhs.is_zero() {
            return Err(Error::new("modulo by zero"));
        }
        self.combine(&rhs, Ratio::checked_rem, Float::rem)
    }

    pub fn pow(self, exp: Value) -> Result<Value, Error> {
        if self.is_zero() && exp.is_negative() {
            return Err(Error::new("division by zero"));
        }
        self.combine(&exp, Ratio::checked_powi, Float::powf)
    }

    /// Total order; exact values compare exactly where they can.
    pub fn total_cmp(&self, rhs: &Value) -> Ordering {
        if let (Value::Exact(a), Value::Exact(b)) = (self, rhs)
            && let Some(order) = a.checked_cmp(b)
        {
            return order;
        }
        self.to_float().total_cmp(&rhs.to_float())
    }

    pub fn neg(self) -> Value {
        match self {
            Value::Exact(r) => r.checked_neg().map_or_else(|| Value::Approx(r.to_float().neg()), Value::Exact),
            Value::Approx(f) => Value::Approx(f.neg()),
        }
    }

    pub fn abs(self) -> Value {
        if self.is_negative() { self.neg() } else { self }
    }

    pub fn floor(self) -> Value {
        match self {
            Value::Exact(r) => r.floor().map_or_else(|| Value::Approx(r.to_float().floor()), Value::Exact),
            Value::Approx(f) => Value::Approx(f.floor()),
        }
    }

    pub fn ceil(self) -> Value {
        self.neg().floor().neg()
    }

    /// Rounds half away from zero, as spreadsheets do.
    pub fn round(self) -> Value {
        match self {
            Value::Exact(r) => r.round().map_or_else(|| Value::Approx(r.to_float().round()), Value::Exact),
            Value::Approx(f) => Value::Approx(f.round()),
        }
    }

    pub fn sqrt(self) -> Result<Value, Error> {
        if self.is_negative() {
            return Err(Error::new("square root of a negative number"));
        }
        if let Value::Exact(r) = &self
            && let Some(root) = r.checked_sqrt()
        {
            return Ok(Value::Exact(root));
        }
        self.float(Float::sqrt)
    }

    /// n! for a whole number n: exact while it fits, then a float, up to `MAX_FACTORIAL`.
    pub fn factorial(self) -> Result<Value, Error> {
        let whole = match &self {
            Value::Exact(r) => r.is_integer(),
            Value::Approx(f) => f.is_integer(),
        };
        if !whole || self.is_negative() {
            return Err(Error::new("factorial needs a whole number ≥ 0"));
        }
        let n = match &self {
            Value::Exact(r) => r.to_u64(),
            Value::Approx(f) => f.to_u64(),
        };
        let n = n.filter(|&n| n <= MAX_FACTORIAL).ok_or_else(|| Error::new("result out of range"))?;
        (2..=n as i64).try_fold(Value::from_int(1), |product, k| product.mul(Value::from_int(k)))
    }

    fn float(&self, f: fn(&Float) -> Result<Float, Error>) -> Result<Value, Error> {
        f(&self.to_float()).map(Value::Approx)
    }

    /// For ln and log: zero is out of range, negatives are undefined.
    fn positive(&self) -> Result<(), Error> {
        if self.is_zero() {
            Err(Error::new("result out of range"))
        } else if self.is_negative() {
            Err(Error::new("undefined result"))
        } else {
            Ok(())
        }
    }

    /// For asin and acos: only -1 to 1.
    fn unit(&self) -> Result<(), Error> {
        let one = Value::from_int(1);
        let outside = self.total_cmp(&one).is_gt() || self.total_cmp(&one.neg()).is_lt();
        if outside { Err(Error::new("undefined result")) } else { Ok(()) }
    }

    pub fn ln(self) -> Result<Value, Error> {
        self.positive()?;
        self.float(Float::ln)
    }
    pub fn log10(self) -> Result<Value, Error> {
        self.positive()?;
        self.float(Float::log10)
    }
    pub fn exp(self) -> Result<Value, Error> {
        self.float(Float::exp)
    }
    pub fn sin(self) -> Result<Value, Error> {
        self.float(Float::sin)
    }
    pub fn cos(self) -> Result<Value, Error> {
        self.float(Float::cos)
    }
    pub fn tan(self) -> Result<Value, Error> {
        self.float(Float::tan)
    }
    pub fn asin(self) -> Result<Value, Error> {
        self.unit()?;
        self.float(Float::asin)
    }
    pub fn acos(self) -> Result<Value, Error> {
        self.unit()?;
        self.float(Float::acos)
    }
    pub fn atan(self) -> Result<Value, Error> {
        self.float(Float::atan)
    }
    pub fn sinh(self) -> Result<Value, Error> {
        self.float(Float::sinh)
    }
    pub fn cosh(self) -> Result<Value, Error> {
        self.float(Float::cosh)
    }
    pub fn tanh(self) -> Result<Value, Error> {
        self.float(Float::tanh)
    }
}
