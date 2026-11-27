//! Errors remember where in the input they happened, so they can be shown in place:
//!
//! ```text
//! calcx: missing '}'
//!   2x{3+4
//!     ^
//! ```

use std::fmt;
use std::ops::Range;

/// A range of character positions in an expression or a line of input.
pub type Span = Range<usize>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ErrorKind {
    /// The command line itself is wrong: exit status 2.
    Usage,
    /// An expression or its input is wrong: exit status 1.
    Input,
}

#[derive(Clone, Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    span: Option<Span>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Error {
        Error { kind: ErrorKind::Input, message: message.into(), span: None }
    }

    pub fn usage(message: impl Into<String>) -> Error {
        Error { kind: ErrorKind::Usage, ..Error::new(message) }
    }

    /// Points the error at `span`, unless something more specific already has.
    pub fn at(mut self, span: Span) -> Error {
        if self.span.is_none() {
            self.span = Some(span);
        }
        self
    }

    pub fn exit_code(&self) -> u8 {
        match self.kind {
            ErrorKind::Usage => 2,
            ErrorKind::Input => 1,
        }
    }

    /// The message followed by `source` with the error's place underlined.
    pub fn in_source<'a>(&'a self, source: &'a str) -> impl fmt::Display + 'a {
        InSource { error: self, source }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.message)
    }
}

struct InSource<'a> {
    error: &'a Error,
    source: &'a str,
}

impl fmt::Display for InSource<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.error.message)?;
        let source = self.source.trim_end();
        let Some(span) = self.error.span.as_ref().filter(|_| !source.is_empty()) else {
            return Ok(());
        };
        let carets = "^".repeat(span.len().max(1));
        write!(f, "\n  {source}\n  {:indent$}{carets}", "", indent = span.start)
    }
}
