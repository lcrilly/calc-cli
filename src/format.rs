//! Turns values into decimals for printing.

use std::fmt;

use crate::value::Value;

/// Decimal places shown for recurring and irrational results when no precision is given.
pub const DEFAULT_PLACES: usize = 5;

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
    pub exp: Option<i64>,
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
    pub fn tidy(mut self) -> Decimal {
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

pub fn format(value: &Value, precision: Precision) -> Decimal {
    match value {
        Value::Exact(r) => r.format(precision),
        Value::Approx(f) => f.format(precision),
    }
}
