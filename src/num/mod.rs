//! The number types behind `Value`, chosen at build time:
//!
//! * by default, i128 rationals and f64, with no dependencies (`native`);
//! * with `--features precise`, rationals of any size and decimal floats to any
//!   precision, from the dashu crates (`precise`).
//!
//! Both offer the same interface, so the rest of calc doesn't know which it has.

#[cfg(not(feature = "precise"))]
mod native;
#[cfg(not(feature = "precise"))]
pub use native::*;

#[cfg(feature = "precise")]
mod precise;
#[cfg(feature = "precise")]
pub use precise::*;
