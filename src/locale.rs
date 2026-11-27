//! How numbers are written for people: decimal mark and digit grouping.
//!
//! The locale only changes how numbers are *displayed* (and read, under an explicit
//! `--pretty`). Plain input syntax is the same everywhere, so a script gives the same
//! answer on every machine.

use std::sync::OnceLock;

use crate::error::Error;
use crate::format::Decimal;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumberFormat {
    pub decimal: char,
    pub group: Option<char>,
    /// Size of the group nearest the decimal mark (3 almost everywhere).
    pub primary: usize,
    /// Size of every group after that (2 in en_IN: 12,34,567).
    pub secondary: usize,
}

impl NumberFormat {
    pub const PLAIN: NumberFormat = NumberFormat { decimal: '.', group: None, primary: 3, secondary: 3 };

    /// Reads a format from an example such as `1,234.5`, `1.234,5`, `1 234,5` or `12,34,567.8`.
    /// The last separator is the decimal mark, so the example must have a fractional part.
    pub fn from_example(example: &str) -> Result<NumberFormat, Error> {
        let invalid = || {
            Error::usage(format!(
                "invalid number format '{example}' (give an example with decimals, such as 1,234.5 or 1.234,5)"
            ))
        };
        let chars: Vec<char> = example.chars().collect();
        let mark = chars.iter().rposition(|c| !c.is_ascii_digit()).ok_or_else(invalid)?;
        if mark == 0 || mark + 1 == chars.len() {
            return Err(invalid());
        }
        let decimal = chars[mark];

        let mut groups = vec![0usize];
        let mut group = None;
        for &c in &chars[..mark] {
            if c.is_ascii_digit() {
                *groups.last_mut().unwrap() += 1;
            } else if group.is_none_or(|g| g == c) && c != decimal {
                group = Some(c);
                groups.push(0);
            } else {
                return Err(invalid());
            }
        }
        if groups.contains(&0) {
            return Err(invalid());
        }
        if group.is_none() {
            return Ok(NumberFormat { decimal, ..NumberFormat::PLAIN });
        }
        let primary = *groups.last().unwrap();
        let secondary = if groups.len() >= 3 { groups[groups.len() - 2] } else { primary };
        Ok(NumberFormat { decimal, group, primary, secondary })
    }

    /// Writes a decimal in this format.
    pub fn apply(&self, d: &Decimal) -> String {
        let mut out = String::new();
        if d.negative {
            out.push('-');
        }
        match (self.group, d.exp) {
            // Grouping a mantissa (1.5e20) would mean nothing.
            (Some(g), None) => out.push_str(&self.group_digits(&d.int, g)),
            _ => out.push_str(&d.int),
        }
        if !d.frac.is_empty() {
            out.push(self.decimal);
            out.push_str(&d.frac);
        }
        if let Some(e) = d.exp {
            out.push_str(&format!("e{e}"));
        }
        out
    }

    fn group_digits(&self, int: &str, sep: char) -> String {
        let digits: Vec<char> = int.chars().collect();
        let mut cuts = Vec::new();
        let mut end = digits.len();
        let mut size = self.primary;
        while end > size && size > 0 {
            end -= size;
            cuts.push(end);
            size = self.secondary;
        }
        let mut out = String::with_capacity(int.len() * 2);
        for (i, c) in digits.iter().enumerate() {
            if i > 0 && cuts.contains(&i) {
                out.push(sep);
            }
            out.push(*c);
        }
        out
    }

    /// True when `n` digits after a group separator make a valid group.
    pub fn is_group_size(&self, n: usize) -> bool {
        n == self.primary || n == self.secondary
    }
}

/// The user's preferred format, detected once. In order of precedence:
/// `CALCX_NUMBER_FORMAT` (an example), `LC_ALL`/`LC_NUMERIC` when set,
/// the macOS Language & Region settings, then `LANG`.
pub fn detected() -> NumberFormat {
    static DETECTED: OnceLock<NumberFormat> = OnceLock::new();
    *DETECTED.get_or_init(|| {
        if let Ok(example) = std::env::var("CALCX_NUMBER_FORMAT")
            && let Ok(format) = NumberFormat::from_example(&example)
        {
            return format;
        }
        let set = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        if !set("LC_ALL") && !set("LC_NUMERIC")
            && let Some(format) = system_format()
        {
            return format;
        }
        posix_format().unwrap_or(NumberFormat::PLAIN)
    })
}

/// Asks the C library for the LC_NUMERIC locale (honouring LC_ALL, LC_NUMERIC, LANG).
fn posix_format() -> Option<NumberFormat> {
    use std::ffi::{CStr, c_char, c_int};

    #[repr(C)]
    struct LconvHead {
        decimal_point: *const c_char,
        thousands_sep: *const c_char,
        grouping: *const c_char,
    }
    unsafe extern "C" {
        fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
        fn localeconv() -> *const LconvHead;
    }
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    const LC_NUMERIC: c_int = 4;
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
    const LC_NUMERIC: c_int = 1;

    // SAFETY: calcx is single-threaded when this runs, and the lconv strings are
    // copied out before anything else can touch the locale.
    unsafe {
        if setlocale(LC_NUMERIC, c"".as_ptr()).is_null() {
            return None;
        }
        let lc = localeconv();
        if lc.is_null() {
            return None;
        }
        let text = |p: *const c_char| {
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        };
        let decimal = text((*lc).decimal_point).chars().next().unwrap_or('.');
        let group = text((*lc).thousands_sep).chars().next();
        let grouping: Vec<u8> = if (*lc).grouping.is_null() {
            Vec::new()
        } else {
            CStr::from_ptr((*lc).grouping).to_bytes().to_vec()
        };
        // Back to the C locale so nothing else changes behaviour.
        setlocale(LC_NUMERIC, c"C".as_ptr());

        let size = |b: Option<&u8>| b.filter(|&&b| b > 0 && b < 127).map(|&b| b as usize);
        let primary = size(grouping.first());
        let group = group.filter(|_| primary.is_some());
        let primary = primary.unwrap_or(3);
        let secondary = size(grouping.get(1)).unwrap_or(primary);
        Some(NumberFormat { decimal, group, primary, secondary })
    }
}

/// The number format from System Settings → Language & Region, including any
/// custom symbols, found by formatting a sample number with Core Foundation.
#[cfg(target_os = "macos")]
fn system_format() -> Option<NumberFormat> {
    use std::ffi::{c_char, c_void};

    type CFTypeRef = *const c_void;
    const DECIMAL_STYLE: isize = 1; // kCFNumberFormatterDecimalStyle
    const DOUBLE_TYPE: isize = 13; // kCFNumberDoubleType
    const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFLocaleCopyCurrent() -> CFTypeRef;
        fn CFNumberFormatterCreate(alloc: CFTypeRef, locale: CFTypeRef, style: isize) -> CFTypeRef;
        fn CFNumberFormatterCreateStringWithValue(
            alloc: CFTypeRef,
            formatter: CFTypeRef,
            number_type: isize,
            value: *const c_void,
        ) -> CFTypeRef;
        fn CFStringGetCString(s: CFTypeRef, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(cf: CFTypeRef);
    }

    // SAFETY: each object is checked for null and released exactly once.
    let sample = unsafe {
        let locale = CFLocaleCopyCurrent();
        if locale.is_null() {
            return None;
        }
        let formatter = CFNumberFormatterCreate(std::ptr::null(), locale, DECIMAL_STYLE);
        CFRelease(locale);
        if formatter.is_null() {
            return None;
        }
        let value: f64 = 1234567.5;
        let string = CFNumberFormatterCreateStringWithValue(
            std::ptr::null(),
            formatter,
            DOUBLE_TYPE,
            (&raw const value).cast(),
        );
        CFRelease(formatter);
        if string.is_null() {
            return None;
        }
        let mut buffer = [0 as c_char; 64];
        let ok = CFStringGetCString(string, buffer.as_mut_ptr(), buffer.len() as isize, UTF8);
        CFRelease(string);
        if ok == 0 {
            return None;
        }
        std::ffi::CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned()
    };
    NumberFormat::from_example(&sample).ok()
}

#[cfg(not(target_os = "macos"))]
fn system_format() -> Option<NumberFormat> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(example: &str) -> NumberFormat {
        NumberFormat::from_example(example).unwrap()
    }

    #[test]
    fn examples() {
        assert_eq!(fmt("1,234.5"), NumberFormat { decimal: '.', group: Some(','), primary: 3, secondary: 3 });
        assert_eq!(fmt("1.234,5"), NumberFormat { decimal: ',', group: Some('.'), primary: 3, secondary: 3 });
        assert_eq!(fmt("12,34,567.8"), NumberFormat { decimal: '.', group: Some(','), primary: 3, secondary: 2 });
        assert_eq!(fmt("1234.5"), NumberFormat::PLAIN);
        assert!(NumberFormat::from_example("1,234").is_ok()); // read as decimal comma, no grouping
        assert!(NumberFormat::from_example("1,234,").is_err());
        assert!(NumberFormat::from_example("1,234 567.5").is_err());
        assert!(NumberFormat::from_example("abc").is_err());
    }

    #[test]
    fn apply() {
        assert_eq!(fmt("1,234.5").apply(&Decimal::from_std("3141.6")), "3,141.6");
        assert_eq!(fmt("1.234,5").apply(&Decimal::from_std("3141.6")), "3.141,6");
        assert_eq!(fmt("1\u{202f}234,5").apply(&Decimal::from_std("-1234567")), "-1\u{202f}234\u{202f}567");
        assert_eq!(fmt("12,34,567.8").apply(&Decimal::from_std("1234567.25")), "12,34,567.25");
        assert_eq!(fmt("1,234.5").apply(&Decimal::from_std("999")), "999");
        assert_eq!(fmt("1.234,5").apply(&Decimal::from_std("1.5e20")), "1,5e20");
    }
}
