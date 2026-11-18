mod format;
mod functions;
mod locale;
mod parser;
mod value;

use std::fmt;
use std::io::{self, BufRead, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

use format::Precision;
use locale::NumberFormat;
use parser::{Expression, Syntax};
use value::Value;

const USAGE: &str = "\
calc - evaluate arithmetic expressions and lists of numbers

Usage:
  calc [OPTIONS] EXPRESSION...
  ... | calc [OPTIONS] [PARTIAL EXPRESSION...]

In most cases shell quoting and escaping can be avoided.

Operators:  + -  x * × · •  ÷ /  % mod  ^
Brackets:   {} or () or []
Roots:      √ or sqrt
Constants:  π or pi
Lists:      1 2 3  or  1,2,3   (1 -2 is a list; 1 - 2 is subtraction)
Ranges:     1..10 or 1…10
Functions:  sum ∑  product prod ∏  avg mean average x̄  median  min  max  stdev
            abs  floor  ceil  round  ln  log  exp  sin  cos  tan
            sum{1 2 3}, sum(1,2,3), or sum 1 2 3 (takes everything after it)

Arithmetic applies element by element: {1 2 3}x2 is 2 4 6. Lists print one
number per line. Adjacent terms multiply: 2π, 3√2, 2{1+1}. Group digits with _
(1_000). An unmatched closing bracket wraps everything before it: 2+4+8}/2 is 7.

Lines of stdin form one list: seq 10 | calc sum. Arguments that start with an
operator continue it, and arguments that end with an operator or function take
it as their last operand. @ places it explicitly: calc 'max{@}-min{@}'.

Options (before the expression):
  -p, --precision=N|max  decimal places for non-terminating results
                         (default 5; max = full f64 precision)
  -r, --rows             preserve rows: each line of stdin is its own list,
                         and gives one line of output
      --pretty[=always|auto|EXAMPLE]
                         read and write numbers in the local style (3,141.6 or
                         3.141,6); always is the default, auto only applies to
                         a terminal, and an EXAMPLE such as 1.234,5 sets the style
  -h, --help             show this help
  -V, --version          show version

Environment:
  CALC_PRETTY=auto|always|never   pretty output (never changes how input is read)
  CALC_NUMBER_FORMAT=EXAMPLE      number style, instead of the system locale";

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

#[derive(Clone, Copy, PartialEq)]
enum Pretty {
    Auto,
    Always,
    Never,
}

impl Pretty {
    fn parse(value: &str) -> Option<Pretty> {
        match value {
            "auto" => Some(Pretty::Auto),
            "always" => Some(Pretty::Always),
            "never" => Some(Pretty::Never),
            _ => None,
        }
    }
}

struct Options {
    precision: Precision,
    /// `--pretty` as given on the command line; it also changes how input is read.
    pretty: Option<Pretty>,
    /// From `--pretty=EXAMPLE`.
    format: Option<NumberFormat>,
    /// `-r`/`--rows`: preserve rows, evaluating each line of stdin as its own list.
    rows: bool,
}

enum Command<'a> {
    Help,
    Version,
    Run(Options, &'a [String]),
}

fn parse_precision(value: &str) -> Option<Precision> {
    match value {
        "max" => Some(Precision::Max),
        _ => value.parse().ok().map(Precision::Places),
    }
}

/// Reads a precision option at the start of `args`, returning it and how many arguments it used.
fn precision_option(args: &[String]) -> Result<Option<(Precision, usize)>, Error> {
    let Some(first) = args.first() else {
        return Ok(None);
    };
    let invalid = |v: &str| Error::new(format!("invalid precision '{v}' (expected an integer or 'max')"));
    let with_next = || {
        let v = args.get(1).ok_or_else(|| Error::new(format!("{first} requires a value")))?;
        Ok(Some((parse_precision(v).ok_or_else(|| invalid(v))?, 2)))
    };

    if let Some(rest) = first.strip_prefix("--precision") {
        return match rest.strip_prefix('=') {
            Some(v) => Ok(Some((parse_precision(v).ok_or_else(|| invalid(v))?, 1))),
            None if rest.is_empty() => with_next(),
            None => Err(Error::new(format!("unknown option '{first}'"))),
        };
    }
    if let Some(rest) = first.strip_prefix("-p") {
        if let Some(v) = rest.strip_prefix('=') {
            return Ok(Some((parse_precision(v).ok_or_else(|| invalid(v))?, 1)));
        }
        if rest.is_empty() {
            return with_next();
        }
        if let Some(p) = parse_precision(rest) {
            return Ok(Some((p, 1)));
        }
        // Anything else (e.g. "-pi") is an expression, not an option.
    }
    Ok(None)
}

/// Splits the leading options off the arguments. Options end at the first argument
/// that isn't one, or at `--`.
fn parse_options(args: &[String]) -> Result<Command<'_>, Error> {
    let mut options = Options { precision: Precision::Default, pretty: None, format: None, rows: false };
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        match arg.as_str() {
            "--" => {
                i += 1;
                break;
            }
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-r" | "--rows" => options.rows = true,
            "--pretty" => options.pretty = Some(Pretty::Always),
            _ if arg.starts_with("--pretty=") => {
                let value = &arg["--pretty=".len()..];
                match Pretty::parse(value) {
                    Some(mode) => options.pretty = Some(mode),
                    None => {
                        options.pretty = Some(Pretty::Always);
                        options.format = Some(NumberFormat::from_example(value)?);
                    }
                }
            }
            _ => match precision_option(&args[i..])? {
                Some((precision, used)) => {
                    options.precision = precision;
                    i += used;
                    continue;
                }
                None if arg.starts_with("--") => return Err(Error::new(format!("unknown option '{arg}'"))),
                None => break,
            },
        }
        i += 1;
    }
    Ok(Command::Run(options, &args[i..]))
}

/// Formats results: plain by default, or in the locale's style when pretty.
struct Printer {
    precision: Precision,
    pretty: Option<NumberFormat>,
}

impl Printer {
    fn format(&self, value: Value, decimal_input: bool) -> String {
        let plain = format::format(value, decimal_input, self.precision);
        match self.pretty {
            Some(f) => f.apply(&plain),
            None => plain,
        }
    }

    fn write_lines(&self, out: &mut impl Write, values: &[Value], decimal_input: bool) -> io::Result<()> {
        for &v in values {
            writeln!(out, "{}", self.format(v, decimal_input))?;
        }
        Ok(())
    }

    fn write_row(&self, out: &mut impl Write, values: &[Value], decimal_input: bool) -> io::Result<()> {
        let row: Vec<String> = values.iter().map(|&v| self.format(v, decimal_input)).collect();
        writeln!(out, "{}", row.join(" "))
    }
}

fn report(out: &mut impl Write, message: impl fmt::Display) -> io::Result<()> {
    // Keep stdout and stderr in order when both go to a terminal.
    out.flush()?;
    eprintln!("calc: {message}");
    Ok(())
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

impl Iterator for StdinLines {
    type Item = io::Result<(usize, Result<String, Error>)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut line = Vec::new();
            match self.reader.read_until(b'\n', &mut line) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) => return Some(Err(e)),
            }
            self.n += 1;
            let line = String::from_utf8(line).map_err(|_| Error::new("input is not valid UTF-8 (check your locale)"));
            if line.as_ref().is_ok_and(|l| l.trim().is_empty()) {
                continue;
            }
            return Some(Ok((self.n, line)));
        }
    }
}

/// Evaluates the program with stdin. By default all of stdin is one list, but when the
/// program only works element by element each line is evaluated as it arrives, which
/// gives the same output while streaming. `--rows` evaluates every line on its own.
fn run_stdin(program: &parser::Program, syntax: &Syntax, printer: &Printer, rows: bool) -> io::Result<ExitCode> {
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let mut status = ExitCode::SUCCESS;

    if rows || program.streams() {
        let mut lines = StdinLines::new();
        while let Some(line) = lines.next() {
            let (n, line) = line?;
            let result = line
                .and_then(|l| parser::evaluate(&l, syntax))
                .and_then(|e| Ok((program.evaluate(Some(&e.values))?, e.decimal_input || program.decimal_input)));
            match result {
                Ok((values, decimal)) if rows => printer.write_row(&mut out, &values, decimal)?,
                Ok((values, decimal)) => printer.write_lines(&mut out, &values, decimal)?,
                Err(e) => {
                    report(&mut out, format_args!("line {n}: {e}"))?;
                    status = ExitCode::FAILURE;
                }
            }
            // Flush before waiting on more input, so a live feed (`tail -f`) sees each
            // result at once while a large file is still written in big batches.
            if !lines.has_buffered() {
                out.flush()?;
            }
        }
        out.flush()?;
        return Ok(status);
    }

    // The whole list is needed (sum, median, …): read everything, and give no result
    // if any line is bad, since a total that silently skips a line is worse than none.
    let mut values = Vec::new();
    let mut decimal = program.decimal_input;
    for line in StdinLines::new() {
        let (n, line) = line?;
        match line.and_then(|l| parser::evaluate(&l, syntax)) {
            Ok(e) => {
                values.extend(e.values);
                decimal |= e.decimal_input;
            }
            Err(e) => {
                report(&mut out, format_args!("line {n}: {e}"))?;
                status = ExitCode::FAILURE;
            }
        }
    }
    if status == ExitCode::SUCCESS {
        match program.evaluate(Some(&values)) {
            Ok(result) => printer.write_lines(&mut out, &result, decimal)?,
            Err(e) => {
                report(&mut out, e)?;
                status = ExitCode::FAILURE;
            }
        }
    }
    out.flush()?;
    Ok(status)
}

/// Spots `avg{1,2,3}` after the shell's brace expansion turned it into `avg1 avg2 avg3`,
/// which would otherwise give a quietly wrong answer.
fn check_brace_expansion(args: &[String]) -> Result<(), Error> {
    let prefix = |arg: &String| {
        let lower = arg.to_lowercase();
        functions::NAMES
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| lower.strip_prefix(name).is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit())))
            .max_by_key(|name| name.len())
    };
    let Some(first) = args.first().and_then(prefix) else {
        return Ok(());
    };
    if args.len() >= 2 && args.iter().all(|a| prefix(a) == Some(first)) {
        return Err(Error::new(format!(
            "'{}' looks like the shell expanded {first}{{…,…}}: write {first} 1,2,3, {first}{{1 2 3}} or quote it",
            args.join(" ")
        )));
    }
    Ok(())
}

fn print(s: &str) -> io::Result<ExitCode> {
    writeln!(io::stdout(), "{s}")?;
    Ok(ExitCode::SUCCESS)
}

fn fail(status: u8, message: impl fmt::Display) -> io::Result<ExitCode> {
    eprintln!("calc: {message}");
    Ok(ExitCode::from(status))
}

fn run(args: &[String]) -> io::Result<ExitCode> {
    let (options, rest) = match parse_options(args) {
        Ok(Command::Help) => return print(USAGE),
        Ok(Command::Version) => return print(&format!("calc {}", env!("CARGO_PKG_VERSION"))),
        Ok(Command::Run(options, rest)) => (options, rest),
        Err(e) => return fail(2, e),
    };

    // CALC_PRETTY only changes output, so it never changes what a script's input means.
    let env_pretty = std::env::var("CALC_PRETTY").ok().and_then(|v| Pretty::parse(&v));
    let pretty = options.pretty.or(env_pretty).unwrap_or(Pretty::Never);
    let write_pretty = match pretty {
        Pretty::Always => true,
        Pretty::Auto => io::stdout().is_terminal(),
        Pretty::Never => false,
    };
    let read_pretty = options.pretty.is_some_and(|p| p != Pretty::Never);
    let number_format = || options.format.unwrap_or_else(locale::detected);
    let syntax = Syntax { pretty: read_pretty.then(number_format) };
    let printer = Printer { precision: options.precision, pretty: write_pretty.then(number_format) };

    if let Err(e) = check_brace_expansion(rest) {
        return fail(2, e);
    }
    let stdin_is_terminal = io::stdin().is_terminal();
    if rest.is_empty() && stdin_is_terminal {
        eprintln!("{USAGE}");
        return Ok(ExitCode::from(2));
    }

    // Arguments are joined with spaces, which separate list elements: `calc 1 2 3`.
    let mut expression = match Expression::parse(&rest.join(" "), &syntax) {
        Ok(e) => e,
        Err(e) => return fail(1, e),
    };
    // Only read stdin when the arguments say so or are missing an operand, so
    // `calc 1+2` in a `while read` loop doesn't swallow the loop's input.
    let incomplete = expression.is_continuation() || expression.awaits_operand();
    let reads_stdin = expression.is_empty() || expression.reads_stdin() || (!stdin_is_terminal && incomplete);
    if reads_stdin && stdin_is_terminal {
        return fail(2, "'@' reads from stdin, but nothing is piped in");
    }
    if reads_stdin {
        expression.feed_stdin();
    }
    let program = match expression.compile() {
        Ok(p) => p,
        Err(e) => return fail(1, e),
    };
    if reads_stdin {
        return run_stdin(&program, &syntax, &printer, options.rows);
    }

    match program.evaluate(None) {
        Ok(values) => {
            let mut out = io::stdout().lock();
            printer.write_lines(&mut out, &values, program.decimal_input)?;
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => fail(1, e),
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
