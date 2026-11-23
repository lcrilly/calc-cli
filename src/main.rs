mod cli;
mod error;
mod eval;
mod format;
mod functions;
mod input;
mod lexer;
mod locale;
mod parser;
mod value;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use cli::{Command, When};
use error::Error;
use input::Printer;
use lexer::Syntax;
use parser::Expression;

const USAGE: &str = "\
calc - evaluate arithmetic expressions on the command line
       operate on (lists of) numbers from stdin

Usage:
  calc [OPTIONS] EXPRESSION...
  ... | calc [OPTIONS] [PARTIAL EXPRESSION...]

Operators:  +  -  x * × · •  ÷ /  % mod  ^  ² ³  √  ∑  ∏  !
Brackets:   {} or () or []
Symbols:    π (pi)  @ (stdin)  ° º (degrees to radians)
Lists:      1 2 3  or  1,2,3
Ranges:     1..10 or 1…10
Functions:  sqrt √  sum ∑  product prod ∏  factorial fact !  ln  log  exp
            avg mean average x̄  median  mode  min  max  stdev  abs  round
            floor  ceil  sin  cos  tan  asin  acos  atan  sinh  cosh  tanh

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
                                 its own list (not merged) and produces the same
                                 number of output rows.
  -V, --version                  Show version

See README at https://github.com/lcrilly/calc-cli";

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
    let (options, rest) = match cli::parse_options(args) {
        Ok(Command::Help) => return print(USAGE),
        Ok(Command::Version) => return print(&format!("calc {}", env!("CARGO_PKG_VERSION"))),
        Ok(Command::Run(options, rest)) => (options, rest),
        Err(e) => return fail(&e, ""),
    };
    if let Err(e) = cli::check_brace_expansion(rest) {
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
    // A lone negative number continues stdin (`calc 2x5 | calc -1` is 9), but if stdin
    // turns out to be empty it is just the number, so `calc $x` still works under cron.
    let if_empty = (reads_stdin && expression.is_lone_negative())
        .then(|| Expression::parse(&source, &syntax).and_then(Expression::compile).ok())
        .flatten();
    if reads_stdin {
        expression.feed_stdin();
    }
    // Syntax errors show up here, before any stdin is read.
    let program = match expression.compile() {
        Ok(p) => p,
        Err(e) => return fail(&e, &source),
    };
    if reads_stdin {
        return input::run_stdin(&program, if_empty.as_ref(), &syntax, &printer, options.rows);
    }

    match program.evaluate(None) {
        Ok(values) => {
            printer.write_lines(&mut io::stdout().lock(), &values)?;
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
