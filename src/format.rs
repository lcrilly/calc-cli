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

pub fn format(value: Value, decimal_input: bool, precision: Precision) -> String {
    let s = match value {
        Value::Exact(r) => format_exact(r, precision),
        Value::Approx(f) => format_float(f, precision),
    };
    tidy(s, decimal_input)
}

fn format_exact(r: Ratio, precision: Precision) -> String {
    if r.den == 1 {
        return r.num.to_string();
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

fn format_float(f: f64, precision: Precision) -> String {
    if f.abs() >= SCIENTIFIC_THRESHOLD {
        return match precision {
            Precision::Default => format!("{f:.DEFAULT_PLACES$e}"),
            Precision::Places(n) => format!("{f:.n$e}"),
            Precision::Max => format!("{f:e}"),
        };
    }
    match precision {
        Precision::Default => format!("{f:.DEFAULT_PLACES$}"),
        Precision::Places(n) => format!("{f:.n$}"),
        Precision::Max => format!("{f}"),
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

/// Exact long division to `places` digits, rounding half away from zero.
/// `None` if an intermediate would overflow, so the caller can fall back to f64.
fn round_exact(r: Ratio, places: usize) -> Option<String> {
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

    let mut s = String::with_capacity(places + 42);
    if r.num < 0 {
        s.push('-');
    }
    s.push_str(&int.to_string());
    if !digits.is_empty() {
        s.push('.');
        s.extend(digits.iter().map(|&d| char::from(b'0' + d)));
    }
    Some(s)
}

/// Trims trailing fractional zeros, drops "-0", and keeps a ".0" when the
/// input itself was written with decimals.
fn tidy(mut s: String, decimal_input: bool) -> String {
    if let Some((mantissa, exponent)) = s.split_once('e') {
        return format!("{}e{exponent}", tidy(mantissa.into(), false));
    }
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0').trim_end_matches('.').len();
        s.truncate(trimmed);
    }
    if s.trim_start_matches('-') == "0" {
        s = "0".into();
    }
    if decimal_input && !s.contains('.') {
        s.push_str(".0");
    }
    s
}
