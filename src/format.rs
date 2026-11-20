//! Turns values into decimals for printing.

use std::fmt;

use crate::value::{Ratio, Value};

/// Decimal places shown for recurring and irrational results when no precision is given.
const DEFAULT_PLACES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Precision {
    /// Terminating decimals in full; recurring/irrational values to `DEFAULT_PLACES`.
    Default,
    Places(usize),
    /// Everything an f64 can hold (shortest round-trip representation).
    Max,
}

/// A number ready to print: its sign, the digits either side of the decimal mark, and
/// an exponent. Printing it plainly (`Display`) or in a local style (`NumberFormat::apply`)
/// is then only a matter of punctuation.
#[derive(Clone, Debug, PartialEq)]
pub struct Decimal {
    pub negative: bool,
    pub int: String,
    /// Fractional digits, without trailing zeros.
    pub frac: String,
    pub exp: Option<i32>,
}

impl Decimal {
    /// Reads Rust's own formatting of an f64 (`-1234.5`, `1.5e20`).
    pub fn from_std(s: &str) -> Decimal {
        let (negative, s) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let (mantissa, exp) = match s.split_once('e') {
            Some((m, e)) => (m, e.parse().ok()),
            None => (s, None),
        };
        let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        Decimal { negative, int: int.into(), frac: frac.into(), exp }.tidy()
    }

    /// Trims trailing fractional zeros and drops the sign from zero.
    fn tidy(mut self) -> Decimal {
        let len = self.frac.trim_end_matches('0').len();
        self.frac.truncate(len);
        if self.frac.is_empty() && self.int.bytes().all(|b| b == b'0') {
            self.negative = false;
        }
        self
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.negative {
            f.write_str("-")?;
        }
        f.write_str(&self.int)?;
        if !self.frac.is_empty() {
            write!(f, ".{}", self.frac)?;
        }
        if let Some(e) = self.exp {
            write!(f, "e{e}")?;
        }
        Ok(())
    }
}

pub fn format(value: Value, precision: Precision) -> Decimal {
    match value {
        Value::Exact(r) => format_exact(r, precision),
        Value::Approx(f) => format_float(f, precision),
    }
}

fn format_exact(r: Ratio, precision: Precision) -> Decimal {
    if r.den == 1 {
        return Decimal { negative: r.num < 0, int: r.num.unsigned_abs().to_string(), frac: String::new(), exp: None };
    }
    let places = match (precision, terminating_places(r.den)) {
        (Precision::Places(n), _) => Some(n),
        (_, Some(n)) => Some(n),
        (Precision::Default, None) => Some(DEFAULT_PLACES),
        (Precision::Max, None) => None,
    };
    places
        .and_then(|n| round_exact(r, n))
        .unwrap_or_else(|| format_float(Value::Exact(r).to_f64(), precision))
}

/// Beyond this an f64's integer digits are mostly noise, so switch to scientific notation.
const SCIENTIFIC_THRESHOLD: f64 = 1e16;

fn format_float(f: f64, precision: Precision) -> Decimal {
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

/// Exact long division to `places` digits, rounding half away from zero.
/// `None` if an intermediate would overflow, so the caller can fall back to f64.
fn round_exact(r: Ratio, places: usize) -> Option<Decimal> {
    let den = r.den as u128;
    let n = r.num.unsigned_abs();
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
    Some(Decimal { negative: r.num < 0, int: int.to_string(), frac, exp: None }.tidy())
}
