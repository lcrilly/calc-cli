mod format;
mod parser;
mod value;

use std::fmt;
use std::io::{self, BufRead, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

use format::Precision;

const USAGE: &str = "\
calc - evaluate arithmetic expressions without shell quoting

Usage:
  calc [--precision=N|max] EXPRESSION...
  echo EXPRESSION | calc [--precision=N|max]

Operators:  + -  x * ×  ÷ /  ^
Brackets:   {} or ()        (nestable; () needs quoting in the shell)
Functions:  √ or sqrt
Constants:  π or pi
Spaces are ignored and arguments are joined, so `calc 2 x {3 + 4}` works.
Adjacent terms multiply: 2π, 3√2, 2{1+1}.
With no expression arguments, each line of stdin is evaluated.

Options (must come first):
  -p, --precision=N|max  decimal places for non-terminating results
                         (default 5; max = full f64 precision)
  -h, --help             show this help
  -V, --version          show version";

#[derive(Debug)]
pub struct Error(String);

impl Error {
    pub fn new(msg: impl Into<String>) -> Error {
        Error(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn parse_precision(value: &str) -> Option<Precision> {
    match value {
        "max" => Some(Precision::Max),
        _ => value.parse().ok().map(Precision::Places),
    }
}

/// Splits a leading precision option off the arguments.
fn split_precision(args: &[String]) -> Result<(Precision, &[String]), Error> {
    let Some(first) = args.first() else {
        return Ok((Precision::Default, args));
    };
    let invalid = |v: &str| Error::new(format!("invalid precision '{v}' (expected an integer or 'max')"));
    let with_next = || {
        let v = args.get(1).ok_or_else(|| Error::new(format!("{first} requires a value")))?;
        Ok((parse_precision(v).ok_or_else(|| invalid(v))?, &args[2..]))
    };

    if let Some(rest) = first.strip_prefix("--precision") {
        return match rest.strip_prefix('=') {
            Some(v) => Ok((parse_precision(v).ok_or_else(|| invalid(v))?, &args[1..])),
            None if rest.is_empty() => with_next(),
            None => Err(Error::new(format!("unknown option '{first}'"))),
        };
    }
    if let Some(rest) = first.strip_prefix("-p") {
        if let Some(v) = rest.strip_prefix('=') {
            return Ok((parse_precision(v).ok_or_else(|| invalid(v))?, &args[1..]));
        }
        if rest.is_empty() {
            return with_next();
        }
        if let Some(p) = parse_precision(rest) {
            return Ok((p, &args[1..]));
        }
        // Anything else (e.g. "-pi") is an expression, not an option.
    }
    Ok((Precision::Default, args))
}

fn calculate(expression: &str, precision: Precision) -> Result<String, Error> {
    let eval = parser::evaluate(expression)?;
    Ok(format::format(eval.value, eval.decimal_input, precision))
}

fn run_stdin(precision: Precision) -> io::Result<ExitCode> {
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let mut status = ExitCode::SUCCESS;
    // Split on raw bytes so one badly encoded line doesn't end the whole run.
    for line in io::stdin().lock().split(b'\n') {
        let Ok(line) = String::from_utf8(line?) else {
            out.flush()?;
            eprintln!("calc: input is not valid UTF-8 (check your locale)");
            status = ExitCode::FAILURE;
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        match calculate(&line, precision) {
            Ok(result) => writeln!(out, "{result}")?,
            Err(e) => {
                // Keep stdout and stderr in order when both go to a terminal.
                out.flush()?;
                eprintln!("calc: {e}");
                status = ExitCode::FAILURE;
            }
        }
    }
    out.flush()?;
    Ok(status)
}

fn print(s: &str) -> io::Result<ExitCode> {
    writeln!(io::stdout(), "{s}")?;
    Ok(ExitCode::SUCCESS)
}

fn run(args: &[String]) -> io::Result<ExitCode> {
    match args.first().map(String::as_str) {
        Some("-h" | "--help") => return print(USAGE),
        Some("-V" | "--version") => return print(&format!("calc {}", env!("CARGO_PKG_VERSION"))),
        _ => {}
    }

    let (precision, rest) = match split_precision(args) {
        Ok(split) => split,
        Err(e) => {
            eprintln!("calc: {e}");
            return Ok(ExitCode::from(2));
        }
    };

    if rest.is_empty() {
        if io::stdin().is_terminal() {
            eprintln!("{USAGE}");
            return Ok(ExitCode::from(2));
        }
        return run_stdin(precision);
    }

    match calculate(&rest.concat(), precision) {
        Ok(result) => print(&result),
        Err(e) => {
            eprintln!("calc: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = match std::env::args_os().skip(1).map(|a| a.into_string()).collect() {
        Ok(args) => args,
        Err(_) => {
            eprintln!("calc: arguments must be valid UTF-8 (check your locale)");
            return ExitCode::from(2);
        }
    };

    match run(&args) {
        Ok(status) => status,
        // The reader went away (e.g. `calc … | head`); nothing left to do.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("calc: {e}");
            ExitCode::FAILURE
        }
    }
}
