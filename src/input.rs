//! Reading stdin and printing results.

use std::io::{self, BufRead, BufWriter, Write};
use std::process::ExitCode;

use crate::error::Error;
use crate::format::{self, Precision};
use crate::lexer::Syntax;
use crate::locale::NumberFormat;
use crate::parser::{self, Program};
use crate::value::Value;

/// Formats results: plain by default, or in the locale's style when pretty.
pub struct Printer {
    pub precision: Precision,
    pub pretty: Option<NumberFormat>,
}

impl Printer {
    fn format(&self, value: &Value) -> String {
        let decimal = format::format(value, self.precision);
        match self.pretty {
            Some(f) => f.apply(&decimal),
            None => decimal.to_string(),
        }
    }

    pub fn write_lines(&self, out: &mut impl Write, values: &[Value]) -> io::Result<()> {
        for v in values {
            writeln!(out, "{}", self.format(v))?;
        }
        Ok(())
    }

    fn write_row(&self, out: &mut impl Write, values: &[Value]) -> io::Result<()> {
        let row: Vec<String> = values.iter().map(|v| self.format(v)).collect();
        writeln!(out, "{}", row.join(" "))
    }
}

/// Reports errors on stderr, keeping them in order with stdout, and remembers whether any occurred.
#[derive(Default)]
struct Reporter {
    failed: bool,
}

impl Reporter {
    fn report(&mut self, out: &mut impl Write, line: Option<usize>, error: &Error, source: &str) -> io::Result<()> {
        self.failed = true;
        out.flush()?;
        match line {
            Some(n) => eprintln!("{}: line {n}: {}", crate::cli::program_name(), error.in_source(source)),
            None => eprintln!("{}: {}", crate::cli::program_name(), error.in_source(source)),
        }
        Ok(())
    }

    fn status(&self) -> ExitCode {
        if self.failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
    }
}

/// Non-blank lines of stdin with their line numbers; a badly encoded line is an error
/// rather than the end of input.
struct StdinLines {
    reader: io::BufReader<io::Stdin>,
    n: usize,
}

impl StdinLines {
    fn new() -> StdinLines {
        StdinLines { reader: io::BufReader::new(io::stdin()), n: 0 }
    }

    /// True when the next line can be read without waiting for more input.
    fn has_buffered(&self) -> bool {
        !self.reader.buffer().is_empty()
    }
}

/// A line of stdin: its number and its text, or why it couldn't be read.
struct Line {
    n: usize,
    text: Result<String, Error>,
}

impl Line {
    fn values(&self, syntax: &Syntax) -> Result<Vec<Value>, Error> {
        match &self.text {
            Ok(text) => parser::evaluate(text, syntax),
            Err(e) => Err(e.clone()),
        }
    }

    fn source(&self) -> &str {
        self.text.as_deref().unwrap_or("")
    }
}

impl Iterator for StdinLines {
    type Item = io::Result<Line>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut bytes = Vec::new();
            match self.reader.read_until(b'\n', &mut bytes) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) => return Some(Err(e)),
            }
            self.n += 1;
            let text = String::from_utf8(bytes).map_err(|_| Error::new("input is not valid UTF-8 (check your locale)"));
            if text.as_ref().is_ok_and(|t| t.trim().is_empty()) {
                continue;
            }
            return Some(Ok(Line { n: self.n, text }));
        }
    }
}

/// Evaluates the program with stdin. By default all of stdin is one list, but when the
/// program only works element by element each line is evaluated as it arrives, which
/// gives the same output while streaming. `--rows` evaluates every line on its own.
/// `if_empty` is evaluated instead when stdin has no lines at all.
pub fn run_stdin(
    program: &Program,
    if_empty: Option<&Program>,
    syntax: &Syntax,
    printer: &Printer,
    rows: bool,
) -> io::Result<ExitCode> {
    let mut out = BufWriter::new(io::stdout().lock());
    let mut errors = Reporter::default();

    if rows || program.streams() {
        let mut lines = StdinLines::new();
        let mut empty = true;
        while let Some(line) = lines.next() {
            let line = line?;
            empty = false;
            match line.values(syntax).map(|values| program.evaluate(Some(&values))) {
                Err(e) => errors.report(&mut out, Some(line.n), &e, line.source())?,
                Ok(Err(e)) => errors.report(&mut out, Some(line.n), &e, program.source())?,
                Ok(Ok(result)) if rows => printer.write_row(&mut out, &result)?,
                Ok(Ok(result)) => printer.write_lines(&mut out, &result)?,
            }
            // Flush before waiting on more input, so a live feed (`tail -f`) sees each
            // result at once while a large file is still written in big batches.
            if !lines.has_buffered() {
                out.flush()?;
            }
        }
        if empty && let Some(standalone) = if_empty {
            match standalone.evaluate(None) {
                Ok(result) => printer.write_lines(&mut out, &result)?,
                Err(e) => errors.report(&mut out, None, &e, standalone.source())?,
            }
        }
        out.flush()?;
        return Ok(errors.status());
    }

    // The whole list is needed (sum, median, …): read everything, and give no result
    // if any line is bad, since a total that silently skips a line is worse than none.
    let mut values = Vec::new();
    for line in StdinLines::new() {
        let line = line?;
        match line.values(syntax) {
            Ok(v) => values.extend(v),
            Err(e) => errors.report(&mut out, Some(line.n), &e, line.source())?,
        }
    }
    if !errors.failed {
        match program.evaluate(Some(&values)) {
            Ok(result) => printer.write_lines(&mut out, &result)?,
            Err(e) => errors.report(&mut out, None, &e, program.source())?,
        }
    }
    out.flush()?;
    Ok(errors.status())
}
