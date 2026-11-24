//! The `precise` backend: exact rationals of any size (dashu-ratio), and decimal
//! floats (dashu-float) computed to as many digits as `-p` asks for.
//!
//! Exact results stay exact until they grow past `MAX_BITS`, then fall back to a
//! float, so `2^{10^9}` gives an answer rather than exhausting memory.

use std::cmp::Ordering;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use dashu_float::DBig;
use dashu_int::ops::{BitTest, DivRem, SquareRoot, UnsignedAbs};
use dashu_int::{IBig, UBig};
use dashu_ratio::RBig;

use crate::error::Error;
use crate::format::{DEFAULT_PLACES, Decimal, Precision};

/// Factorials beyond this would take too long to be useful.
pub const MAX_FACTORIAL: u64 = 100_000;

/// The largest exact number kept, in bits of numerator plus denominator (about 300,000 digits).
const MAX_BITS: usize = 1 << 20;

/// Digits computed beyond those printed, to absorb rounding in intermediate steps.
const GUARD: usize = 10;
/// Significant digits printed by `-p max`.
const MAX_DIGITS: usize = 100;
/// Significant digits printed beyond the requested decimal places, so large values keep them.
const HEADROOM: usize = 50;

/// Significant decimal digits that floats are computed to.
static WORKING: AtomicUsize = AtomicUsize::new(DEFAULT_PLACES + HEADROOM + GUARD);

fn working() -> usize {
    WORKING.load(Relaxed)
}

/// Sets how many digits floats are computed to, from the precision they'll be printed at.
pub fn set_precision(precision: Precision) {
    let printed = match precision {
        Precision::Default => DEFAULT_PLACES + HEADROOM,
        Precision::Places(n) => n.saturating_add(HEADROOM),
        Precision::Max => MAX_DIGITS,
    };
    WORKING.store(printed.saturating_add(GUARD), Relaxed);
}

fn bits(n: &IBig) -> usize {
    n.clone().unsigned_abs().bit_len()
}

/// The size of a rational, in bits.
fn size(r: &RBig) -> usize {
    bits(r.numerator()) + r.denominator().bit_len()
}

fn small(r: RBig) -> Option<Ratio> {
    (size(&r) <= MAX_BITS).then_some(Ratio(r))
}

/// `n / d` rounded half away from zero.
fn div_round(n: &IBig, d: &UBig) -> IBig {
    let (q, r) = n.clone().unsigned_abs().div_rem(d);
    let q = if r * 2u8 >= *d { q + 1u8 } else { q };
    if *n < IBig::ZERO { -IBig::from(q) } else { IBig::from(q) }
}

fn pow10(n: usize) -> UBig {
    UBig::from(10u8).pow(n)
}

/// Splits `|n|`, a number scaled up by 10^places, into whole and fractional digits.
fn split_scaled(n: &IBig, places: usize) -> (String, String) {
    let digits = n.clone().unsigned_abs().to_string();
    let digits = if digits.len() <= places { format!("{}{digits}", "0".repeat(places + 1 - digits.len())) } else { digits };
    let (int, frac) = digits.split_at(digits.len() - places);
    (int.to_string(), frac.to_string())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ratio(RBig);

impl Ratio {
    pub fn from_int(n: i64) -> Ratio {
        Ratio(RBig::from_parts(IBig::from(n), UBig::ONE))
    }

    pub fn from_digits(int: &str, frac: &str) -> Option<Ratio> {
        let digits: IBig = format!("{int}{frac}").parse().ok()?;
        small(RBig::from_parts(digits, pow10(frac.len())))
    }

    pub fn checked_add(&self, rhs: &Ratio) -> Option<Ratio> {
        small(&self.0 + &rhs.0)
    }

    pub fn checked_mul(&self, rhs: &Ratio) -> Option<Ratio> {
        // Check the size first, so a huge product is never computed.
        if size(&self.0) + size(&rhs.0) > MAX_BITS {
            return None;
        }
        small(&self.0 * &rhs.0)
    }

    pub fn checked_recip(&self) -> Option<Ratio> {
        if self.0.is_zero() {
            return None;
        }
        Some(Ratio(RBig::ONE / &self.0))
    }

    pub fn checked_neg(&self) -> Option<Ratio> {
        Some(Ratio(-self.0.clone()))
    }

    /// Remainder of floored division, so it takes the sign of `rhs`. `rhs` must be non-zero.
    pub fn checked_rem(&self, rhs: &Ratio) -> Option<Ratio> {
        let q = (&self.0 / &rhs.0).floor();
        small(&self.0 - &rhs.0 * RBig::from_parts(q, UBig::ONE))
    }

    /// `self` to a whole-number power; `None` if the result would be too large to keep exact.
    pub fn checked_powi(&self, exp: &Ratio) -> Option<Ratio> {
        if !exp.0.is_int() {
            return None;
        }
        let e = i64::try_from(exp.0.numerator().clone()).ok()?;
        let n = usize::try_from(e.unsigned_abs()).ok()?;
        if size(&self.0).checked_mul(n)? > MAX_BITS {
            return None;
        }
        let r = Ratio(self.0.pow(isize::try_from(n).ok()?));
        if e < 0 { r.checked_recip() } else { Some(r) }
    }

    /// The exact square root, when there is one.
    pub fn checked_sqrt(&self) -> Option<Ratio> {
        if self.is_negative() {
            return None;
        }
        let root = |n: UBig| {
            let s = n.sqrt();
            (&s * &s == n).then_some(s)
        };
        let num = root(self.0.numerator().clone().unsigned_abs())?;
        let den = root(self.0.denominator().clone())?;
        Some(Ratio(RBig::from_parts(IBig::from(num), den)))
    }

    pub fn checked_cmp(&self, rhs: &Ratio) -> Option<Ordering> {
        self.0.partial_cmp(&rhs.0)
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    pub fn is_negative(&self) -> bool {
        *self.0.numerator() < IBig::ZERO
    }

    pub fn is_integer(&self) -> bool {
        self.0.is_int()
    }

    pub fn floor(&self) -> Option<Ratio> {
        Some(Ratio(RBig::from_parts(self.0.floor(), UBig::ONE)))
    }

    /// Rounds half away from zero.
    pub fn round(&self) -> Option<Ratio> {
        Some(Ratio(RBig::from_parts(self.0.round(), UBig::ONE)))
    }

    pub fn to_u64(&self) -> Option<u64> {
        if self.0.is_int() { u64::try_from(self.0.numerator().clone()).ok() } else { None }
    }

    pub fn to_float(&self) -> Float {
        let w = working();
        let num = DBig::from(self.0.numerator().clone()).with_precision(w).value();
        let den = DBig::from(IBig::from(self.0.denominator().clone())).with_precision(w).value();
        Float::new(num / den)
    }

    pub fn format(&self, precision: Precision) -> Decimal {
        let (num, den) = (self.0.numerator(), self.0.denominator());
        let negative = *num < IBig::ZERO;
        if self.0.is_int() {
            return Decimal { negative, int: num.clone().unsigned_abs().to_string(), frac: String::new(), exp: None };
        }
        let places = match (precision, terminating_places(den)) {
            (Precision::Places(n), _) => n,
            (_, Some(n)) => n,
            (Precision::Default, None) => DEFAULT_PLACES,
            (Precision::Max, None) => return self.to_float().format(precision),
        };
        let scaled = div_round(&(num * IBig::from(pow10(places))), den);
        let (int, frac) = split_scaled(&scaled, places);
        Decimal { negative, int, frac, exp: None }.tidy()
    }
}

/// Number of decimal places needed if `1/den` terminates (den = 2^a·5^b), else `None`.
fn terminating_places(den: &UBig) -> Option<usize> {
    let twos = den.trailing_zeros().unwrap_or(0);
    let mut rest = den >> twos;
    let mut fives = 0;
    let five = UBig::from(5u8);
    loop {
        let (q, r) = (&rest).div_rem(&five);
        if !r.is_zero() {
            break;
        }
        rest = q;
        fives += 1;
    }
    (rest == UBig::ONE).then_some(twos.max(fives))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Float(DBig);

impl Float {
    /// Rounds to the working precision. dashu keeps the larger precision of two operands,
    /// so this also stops a low-precision value (an integer, say) dragging a result down.
    fn new(x: DBig) -> Float {
        Float(x.with_precision(working()).value())
    }

    pub fn pi() -> Float {
        Float(DBig::pi(working()))
    }

    pub fn degree() -> Float {
        Float::new(DBig::pi(working()) / DBig::from(180u8))
    }

    pub fn from_int(n: i64) -> Float {
        Float::new(DBig::from(IBig::from(n)))
    }

    pub fn parse(literal: &str) -> Option<Float> {
        literal.parse().ok().map(Float::new)
    }

    pub fn add(&self, rhs: &Float) -> Result<Float, Error> {
        Ok(Float::new(&self.0 + &rhs.0))
    }

    pub fn mul(&self, rhs: &Float) -> Result<Float, Error> {
        Ok(Float::new(&self.0 * &rhs.0))
    }

    pub fn div(&self, rhs: &Float) -> Result<Float, Error> {
        if rhs.is_zero() {
            return Err(Error::new("division by zero"));
        }
        Ok(Float::new(&self.0 / &rhs.0))
    }

    /// Floored remainder, taking the sign of `rhs`.
    pub fn rem(&self, rhs: &Float) -> Result<Float, Error> {
        let q = self.div(rhs)?.floor();
        Ok(Float::new(&self.0 - &rhs.0 * &q.0))
    }

    pub fn powf(&self, exp: &Float) -> Result<Float, Error> {
        if self.is_zero() {
            return match exp.0.sign() {
                dashu_int::Sign::Negative if !exp.is_zero() => Err(Error::new("division by zero")),
                _ if exp.is_zero() => Ok(Float::from_int(1)),
                _ => Ok(Float::from_int(0)),
            };
        }
        if self.is_negative() {
            // Only whole powers of a negative number are real: (-2)^3 is -8.
            if !exp.is_integer() {
                return Err(Error::new("undefined result"));
            }
            let magnitude = self.neg().powf(exp)?;
            let odd = !(exp.0.to_int().value() % IBig::from(2)).is_zero();
            return Ok(if odd { magnitude.neg() } else { magnitude });
        }
        Ok(Float::new(self.0.powf(&exp.0)))
    }

    pub fn neg(&self) -> Float {
        Float(-self.0.clone())
    }

    pub fn floor(&self) -> Float {
        Float::new(self.0.floor())
    }

    /// Rounds half away from zero.
    pub fn round(&self) -> Float {
        Float::new(self.0.round())
    }

    pub fn is_zero(&self) -> bool {
        self.0.repr().significand().is_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.0.sign() == dashu_int::Sign::Negative && !self.is_zero()
    }

    pub fn is_integer(&self) -> bool {
        self.0.floor() == self.0
    }

    pub fn to_u64(&self) -> Option<u64> {
        if self.is_integer() { u64::try_from(self.0.to_int().value()).ok() } else { None }
    }

    pub fn total_cmp(&self, rhs: &Float) -> Ordering {
        self.0.partial_cmp(&rhs.0).unwrap_or(Ordering::Equal)
    }

    // The caller checks each function's domain first: dashu panics outside it.
    pub fn sqrt(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.sqrt()))
    }
    pub fn ln(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.ln()))
    }
    pub fn log10(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.ln() / Float::from_int(10).0.ln()))
    }
    pub fn exp(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.exp()))
    }
    pub fn sin(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.sin()))
    }
    pub fn cos(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.cos()))
    }
    pub fn tan(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.tan()))
    }
    pub fn asin(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.asin()))
    }
    pub fn acos(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.acos()))
    }
    pub fn atan(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.atan()))
    }
    pub fn sinh(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.sinh()))
    }
    pub fn cosh(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.cosh()))
    }
    pub fn tanh(&self) -> Result<Float, Error> {
        Ok(Float::new(self.0.tanh()))
    }

    /// Prints to the requested decimal places, but never more significant digits than
    /// were computed reliably; past that, scientific notation.
    pub fn format(&self, precision: Precision) -> Decimal {
        if self.is_zero() {
            return Decimal::from_std("0");
        }
        let printable = working() - GUARD;
        let wanted = match precision {
            Precision::Default => DEFAULT_PLACES,
            Precision::Places(n) => n,
            Precision::Max => usize::MAX,
        };
        let (significand, exponent) = self.0.repr().clone().into_parts();
        let negative = significand < IBig::ZERO;
        let digits = significand.clone().unsigned_abs().to_string().len() as isize;
        // Digits before the decimal point (negative for 0.000…).
        let magnitude = digits + exponent;
        let scale = |places: isize| -> IBig {
            let shift = exponent + places;
            if shift >= 0 {
                &significand * IBig::from(pow10(shift as usize))
            } else {
                div_round(&significand, &pow10(shift.unsigned_abs()))
            }
        };

        if magnitude > printable as isize {
            // Mantissa digits after the point, like f64's 1.23457e20.
            let places = wanted.min(printable - 1);
            let mantissa = scale(places as isize - magnitude + 1);
            let (int, frac) = split_scaled(&mantissa, places);
            // Rounding can carry into a new digit: 9.99…e5 → 10.0e5.
            let (int, frac, magnitude) = if int.len() > 1 {
                (int[..1].to_string(), format!("{}{frac}", &int[1..])[..places].to_string(), magnitude + 1)
            } else {
                (int, frac, magnitude)
            };
            return Decimal { negative, int, frac, exp: Some((magnitude - 1) as i64) }.tidy();
        }
        let places = wanted.min((printable as isize - magnitude) as usize);
        let (int, frac) = split_scaled(&scale(places as isize), places);
        Decimal { negative, int, frac, exp: None }.tidy()
    }
}
