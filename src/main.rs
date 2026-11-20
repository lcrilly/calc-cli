mod error;
mod eval;
mod format;
mod functions;
mod lexer;
mod locale;
mod parser;
mod value;

use std::io::{self, BufRead, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

use error::Error;
use format::Precision;
use lexer::Syntax;
use locale::NumberFormat;
use parser::{Expression, Program};
use value::Value;

const USAGE: &str = "\
calc - evaluate arithmetic expressions on the command line
       operate on (lists of) numbers from stdin

Usage:
  calc [OPTIONS] EXPRESSION...
  ... | calc [OPTIONS] [PARTIAL EXPRESSION...]

Operators:  +  -  x * × · •  ÷ /  % mod  ^  ² ³
Brackets:   {} or () or []
Symbols:    π (pi)  @ (stdin)
Lists:      1 2 3  or  1,2,3
Ranges:     1..10 or 1…10
Functions:  sqrt √  sum ∑  product prod ∏  avg mean average x̄  median  min  max
            stdev  abs  floor  ceil  round  ln  log  exp  sin  cos  tan

Arithmetic applies element by element: {1 2 3}x2 is 2 4 6. Lists print one
number per line. Adjacent terms multiply: 2π, 3√2, 2{1+1}. Group digits with _
(1_000). An unmatched closing bracket encapsulates prior numbers: 3+4+5}/2 → 6.

Stdin can be complete expressions or lists of values. Lines of stdin are merged
to form one list. Partial expressions are applied to stdin. Partials that start
with an operator continue it and arguments that end with an operator or function
take it as their last operand. @ explicitly references stdin: 'calc max@-min@' .

Options:
  -h, --help                     Show this help
  -p, --precision=N|max          Decimal places for non-terminating results
                                 (default 5; max = full f64 precision)
  --pretty[=always|auto|SAMPLE]  Read and write numbers in the local style,
                                 including digit grouping. always is the default,
                                 auto only applies to terminal output, SAMPLE
                                 explicitly sets the style, e.g. 1.234,5
  -r, --rows                     Preserve rows: each line of stdin is treated as
                                 its own list, and gives one line of output.
  -V, --version                  Show version

See README at https://github.com/lcrilly/calc-cli";

#[derive(Clone, Copy, PartialEq)]
enum When {
    Always,
    Auto,
    Never,
}

impl When {
    fn parse(value: &str) -> Option<When> {
        match value {
            "always" => Some(When::Always),
            "auto" => Some(When::Auto),
            "never" => Some(When::Never),
            _ => None,
        }
    }
}

/// `--pretty` as given on the command line; unlike `CALC_PRETTY` it also changes how input is read.
#[derive(Clone, Copy)]
struct Pretty {
    when: When,
    /// From `--pretty=EXAMPLE`.
    format: Option<NumberFormat>,
}

struct Options {
    precision: Precision,
    pretty: Option<Pretty>,
    /// `-r`/`--rows`: preserve rows, evaluating each line of stdin as its own list.
    rows: bool,
}

enum Command<'a> {
    Help,
    Version,
    Run(Options, &'a [String]),
}

fn parse_precision(value: &str) -> Result<Precision, Error> {
    match value {
        "max" => Ok(Precision::Max),
        _ => value
            .parse()
            .map(Precision::Places)
            .map_err(|_| Error::usage(format!("invalid precision '{value}' (expected an integer or 'max')"))),
    }
}

/// Reads a precision option at the start of `args`, returning it and how many arguments it used.
fn precision_option(args: &[String]) -> Result<Option<(Precision, usize)>, Error> {
    let Some(first) = args.first() else {
        return Ok(None);
    };
    let with_next = || {
        let value = args.get(1).ok_or_else(|| Error::usage(format!("{first} requires a value")))?;
        Ok(Some((parse_precision(value)?, 2)))
    };

    if let Some(rest) = first.strip_prefix("--precision") {
        return match rest.strip_prefix('=') {
            Some(value) => Ok(Some((parse_precision(value)?, 1))),
            None if rest.is_empty() => with_next(),
            None => Err(Error::usage(format!("unknown option '{first}'"))),
        };
    }
    if let Some(rest) = first.strip_prefix("-p") {
        if let Some(value) = rest.strip_prefix('=') {
            return Ok(Some((parse_precision(value)?, 1)));
        }
        if rest.is_empty() {
            return with_next();
        }
        if let Ok(p) = parse_precision(rest) {
            return Ok(Some((p, 1)));
        }
        // Anything else (e.g. "-pi") is an expression, not an option.
    }
    Ok(None)
}

/// Splits the leading options off the arguments.
fn parse_options(args: &[String]) -> Result<Command<'_>, Error> {
    let mut options = Options { precision: Precision::Default, pretty: None, rows: false };
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
            "--pretty" => options.pretty = Some(Pretty { when: When::Always, format: None }),
            _ if let Some(value) = arg.strip_prefix("--pretty=") => {
                options.pretty = Some(match When::parse(value) {
                    Some(when) => Pretty { when, format: None },
                    None => Pretty { when: When::Always, format: Some(NumberFormat::from_example(value)?) },
                });
            }
            _ => match precision_option(&args[i..])? {
                Some((precision, used)) => {
                    options.precision = precision;
                    i += used;
                    continue;
                }
                None if arg.starts_with("--") => return Err(Error::usage(format!("unknown option '{arg}'"))),
                None => break,
            },
        }
        i += 1;
    }
    Ok(Command::Run(options, &args[i..]))
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
        return Err(Error::usage(format!(
            "'{}' looks like the shell expanded {first}{{…,…}}: write {first} 1,2,3, {first}{{1 2 3}} or quote it",
            args.join(" ")
        )));
    }
    Ok(())
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
            Some(n) => eprintln!("calc: line {n}: {}", error.in_source(source)),
            None => eprintln!("calc: {}", error.in_source(source)),
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
    fn values(&self, syntax: &Syntax) -> Result<parser::Evaluation, Error> {
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
fn run_stdin(program: &Program, syntax: &Syntax, printer: &Printer, rows: bool) -> io::Result<ExitCode> {
    let mut out = BufWriter::new(io::stdout().lock());
    let mut errors = Reporter::default();

    if rows || program.streams() {
        let mut lines = StdinLines::new();
        while let Some(line) = lines.next() {
            let line = line?;
            match line.values(syntax).map(|e| (program.evaluate(Some(&e.values)), e.decimal_input || program.decimal_input)) {
                Err(e) => errors.report(&mut out, Some(line.n), &e, line.source())?,
                Ok((Err(e), _)) => errors.report(&mut out, Some(line.n), &e, program.source())?,
                Ok((Ok(result), decimal)) if rows => printer.write_row(&mut out, &result, decimal)?,
                Ok((Ok(result), decimal)) => printer.write_lines(&mut out, &result, decimal)?,
            }
            // Flush before waiting on more input, so a live feed (`tail -f`) sees each
            // result at once while a large file is still written in big batches.
            if !lines.has_buffered() {
                out.flush()?;
            }
        }
        out.flush()?;
        return Ok(errors.status());
    }

    // The whole list is needed (sum, median, …): read everything, and give no result
    // if any line is bad, since a total that silently skips a line is worse than none.
    let mut values = Vec::new();
    let mut decimal = program.decimal_input;
    for line in StdinLines::new() {
        let line = line?;
        match line.values(syntax) {
            Ok(e) => {
                values.extend(e.values);
                decimal |= e.decimal_input;
            }
            Err(e) => errors.report(&mut out, Some(line.n), &e, line.source())?,
        }
    }
    if !errors.failed {
        match program.evaluate(Some(&values)) {
            Ok(result) => printer.write_lines(&mut out, &result, decimal)?,
            Err(e) => errors.report(&mut out, None, &e, program.source())?,
        }
    }
    out.flush()?;
    Ok(errors.status())
}

fn print(s: &str) -> io::Result<ExitCode> {
    writeln!(io::stdout(), "{s}")?;
    Ok(ExitCode::SUCCESS)
}

/// Reports an error, showing where it is in `source`.
fn fail(error: &Error, source: &str) -> io::Result<ExitCode> {
    eprintln!("calc: {}", error.in_source(source));
    Ok(ExitCode::from(error.exit_code()))
}

fn run(args: &[String]) -> io::Result<ExitCode> {
    let (options, rest) = match parse_options(args) {
        Ok(Command::Help) => return print(USAGE),
        Ok(Command::Version) => return print(&format!("calc {}", env!("CARGO_PKG_VERSION"))),
        Ok(Command::Run(options, rest)) => (options, rest),
        Err(e) => return fail(&e, ""),
    };
    if let Err(e) = check_brace_expansion(rest) {
        return fail(&e, "");
    }

    // CALC_PRETTY only changes output, so it never changes what a script's input means.
    let env_when = std::env::var("CALC_PRETTY").ok().and_then(|v| When::parse(&v));
    let write_pretty = match options.pretty.map(|p| p.when).or(env_when).unwrap_or(When::Never) {
        When::Always => true,
        When::Auto => io::stdout().is_terminal(),
        When::Never => false,
    };
    let read_pretty = options.pretty.is_some_and(|p| p.when != When::Never);
    let number_format = options.pretty.and_then(|p| p.format).unwrap_or_else(locale::detected);
    let syntax = Syntax { local_decimal: number_format.decimal, pretty: read_pretty.then_some(number_format) };
    let printer = Printer { precision: options.precision, pretty: write_pretty.then_some(number_format) };

    let stdin_is_terminal = io::stdin().is_terminal();
    if rest.is_empty() && stdin_is_terminal {
        eprintln!("{USAGE}");
        return Ok(ExitCode::from(2));
    }

    // Arguments are joined with spaces, which separate list elements: `calc 1 2 3`.
    let source = rest.join(" ");
    let mut expression = match Expression::parse(&source, &syntax) {
        Ok(e) => e,
        Err(e) => return fail(&e, &source),
    };
    // Only read stdin when the arguments say so or are missing an operand, so
    // `calc 1+2` in a `while read` loop doesn't swallow the loop's input.
    let incomplete = expression.is_continuation() || expression.awaits_operand();
    let reads_stdin = expression.is_empty() || expression.reads_stdin() || (!stdin_is_terminal && incomplete);
    if reads_stdin && stdin_is_terminal {
        return fail(&Error::usage("'@' reads from stdin, but nothing is piped in"), "");
    }
    if reads_stdin {
        expression.feed_stdin();
    }
    // Syntax errors show up here, before any stdin is read.
    let program = match expression.compile() {
        Ok(p) => p,
        Err(e) => return fail(&e, &source),
    };
    if reads_stdin {
        return run_stdin(&program, &syntax, &printer, options.rows);
    }

    match program.evaluate(None) {
        Ok(values) => {
            printer.write_lines(&mut io::stdout().lock(), &values, program.decimal_input)?;
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => fail(&e, &source),
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
