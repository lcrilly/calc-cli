mod cli;
mod error;
mod eval;
mod format;
mod functions;
mod input;
mod lexer;
mod locale;
mod num;
mod parser;
mod value;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use cli::{Command, When};
use error::Error;
use input::Printer;
use lexer::Syntax;
use parser::Expression;

/// The help text. `{name}` becomes the name the program was run as, and `{max}`
/// what `--precision=max` means in this build.
const USAGE: &str = "\
Evaluate arithmetic expressions on the command line or from stdin (without quoting)

Usage: {name} [OPTIONS] [EXPRESSION | PARTIAL EXPRESSION]

Expressions:
  Construct expressions with conventional infix operators, scientific symbols
  and common spreadsheet functions. Multiple options are provided to enable
  the use of shell-safe characters without the need for quoting or escaping.

  Operators:  +   -   x * × · •   ÷ /   mod %   ^   ² ³   √   ∑   ∏
  Brackets:   {} or () or []
  Symbols:    π (pi)  @ (stdin)   ° º (degrees to radians)
  Lists:      1 2 3  or  1,2,3
  Ranges:     1..10 or 1…10
  Functions:  sqrt √   sum ∑   product prod ∏   factorial fact !   ln  log  exp
              avg mean average x̄   median  mode  min  max  stdev  abs  round
              floor  ceil  sin  cos  tan  asin  acos  atan  sinh  cosh  tanh

  Stdin can be complete expressions or lists of values. Lines of stdin are
  merged to form one list. Partial expressions are applied to value(s) from
  stdin. Partials that start with an operator continue it and expressions that
  end with an operator or function take it as their last operand. Stdin can be
  explicitly referenced and used multiple times with @: {name} max@-min@

  Lists and ranges are applied value by value: {1 2 3}x2 → 2 4 6. Lists return
  one number per line. Adjacent terms multiply: 2π, 3√2, 2{1+1}. Group digits
  with _ (1_000). Unmatched closing bracket encapsulates prior numbers: 12+5}/2

Options:
  -p N, --precision=N|max
    Decimal places for non-terminating results (default: 5).
    max = {max}

  --pretty[=always|auto|never|SAMPLE]
    Read and write numbers with digit grouping in the local style. No value
    means 'always', 'auto' applies when output is a terminal, 'never' forces
    off, SAMPLE explicitly sets the style, e.g. 1.234,5

  -r, --rows
    Preserve rows: each line of stdin is treated as its own list (not merged).

  -h, --help
    Print help.

  -V, --version
    Print version.

Examples:
  {name} 0.1+0.2            # Accurate decimal arithmetic
  {name} {3x4}^2            # Shell-safe characters to avoid quoting/escaping
  {name} 15+4+3}/2          # Unmatched closing bracket wraps everything prior
  {name} avg 1,2,3,5,8      # Send lists to functions
  seq 1 10 | {name} sum     # Receive lists from stdin
  date +%s | {name} +3600   # Apply partial expressions to values from stdin

See README at https://github.com/lcrilly/calcx";

/// The help text for this build, under the name the program was run as, with
/// headings styled when `styled` (see `styles`).
fn usage(styled: bool) -> String {
    let max = if cfg!(feature = "precise") {
        "100 significant digits."
    } else {
        "full f64 precision (~16 significant digits)."
    };
    let name = cli::program_name();
    let text = align_comments(&USAGE.replace("{name}", name).replace("{max}", max));
    if styled { style_headings(&text, name) } else { text }
}

/// What to print when run with nothing to evaluate: an error, the usage line, and
/// where to find the rest, in the GNU style. The full help is on stdout via --help.
fn usage_hint(styled: bool) -> String {
    let name = cli::program_name();
    let usage = usage(styled).lines().find(|line| line.contains("Usage:")).unwrap_or_default().to_string();
    format!("{name}: no expression\n{usage}\nTry '{name} --help' for more information.")
}

/// True when text written to a stream with this terminal status may be styled:
/// only a terminal, and not when `NO_COLOR` is set or `TERM` is `dumb`.
fn styles(is_terminal: bool) -> bool {
    let set = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    is_terminal && set("NO_COLOR").is_none() && set("TERM").is_none_or(|t| t != "dumb")
}

const HEADING: &str = "\x1b[1;4m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Makes headings bold and underlined: the first word of a line at column 0, when
/// it ends with ':'. After `Usage:` the program's name is bold.
fn style_headings(text: &str, name: &str) -> String {
    text.lines()
        .map(|line| {
            let (word, rest) = line.split_once(' ').map_or((line, None), |(w, r)| (w, Some(r)));
            if line.starts_with(' ') || !word.ends_with(':') {
                return line.to_string();
            }
            match rest {
                None => format!("{HEADING}{word}{RESET}"),
                Some(rest) => {
                    let rest = match rest.strip_prefix(name) {
                        Some(after) if word == "Usage:" => format!("{BOLD}{name}{RESET}{after}"),
                        _ => rest.to_string(),
                    };
                    format!("{HEADING}{word}{RESET} {rest}")
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Lines up the `#` comments in the examples, whatever length the program's name is.
fn align_comments(text: &str) -> String {
    let width = |s: &str| s.chars().count();
    let code = |line: &str| line.split_once(" # ").map(|(code, _)| code.trim_end().to_string());
    let column = text.lines().filter_map(code).map(|c| width(&c)).max().unwrap_or(0) + 3;
    text.lines()
        .map(|line| match (code(line), line.split_once(" # ")) {
            (Some(code), Some((_, comment))) => format!("{code}{}# {comment}", " ".repeat(column - width(&code))),
            _ => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn print(s: &str) -> io::Result<ExitCode> {
    writeln!(io::stdout(), "{s}")?;
    Ok(ExitCode::SUCCESS)
}

/// Reports an error, showing where it is in `source`.
fn fail(error: &Error, source: &str) -> io::Result<ExitCode> {
    eprintln!("{}: {}", cli::program_name(), error.in_source(source));
    Ok(ExitCode::from(error.exit_code()))
}

fn run(args: &[String]) -> io::Result<ExitCode> {
    let (options, rest) = match cli::parse_options(args) {
        Ok(Command::Help) => return print(&usage(styles(io::stdout().is_terminal()))),
        Ok(Command::Version) => {
            let build = if cfg!(feature = "precise") { " (precise)" } else { "" };
            return print(&format!("{} {}{build}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")));
        }
        Ok(Command::Run(options, rest)) => (options, rest),
        Err(e) => return fail(&e, ""),
    };
    if let Err(e) = cli::check_brace_expansion(rest) {
        return fail(&e, "");
    }

    // CALCX_PRETTY only changes output, so it never changes what a script's input means.
    let env_when = std::env::var("CALCX_PRETTY").ok().and_then(|v| When::parse(&v));
    let write_pretty = match options.pretty.map(|p| p.when).or(env_when).unwrap_or(When::Never) {
        When::Always => true,
        When::Auto => io::stdout().is_terminal(),
        When::Never => false,
    };
    let read_pretty = options.pretty.is_some_and(|p| p.when != When::Never);
    let number_format = options.pretty.and_then(|p| p.format).unwrap_or_else(locale::detected);
    let syntax = Syntax { local_decimal: number_format.decimal, pretty: read_pretty.then_some(number_format) };
    let printer = Printer { precision: options.precision, pretty: write_pretty.then_some(number_format) };

    num::set_precision(options.precision);

    let stdin_is_terminal = io::stdin().is_terminal();
    if rest.is_empty() && stdin_is_terminal {
        eprintln!("{}", usage_hint(styles(io::stderr().is_terminal())));
        return Ok(ExitCode::from(2));
    }

    // Arguments are joined with spaces, which separate list elements: `calcx 1 2 3`.
    let source = rest.join(" ");
    let mut expression = match Expression::parse(&source, &syntax) {
        Ok(e) => e,
        Err(e) => return fail(&e, &source),
    };
    // Only read stdin when the arguments say so or are missing an operand, so
    // `calcx 1+2` in a `while read` loop doesn't swallow the loop's input.
    let incomplete = expression.is_continuation() || expression.awaits_operand();
    let reads_stdin = expression.is_empty() || expression.reads_stdin() || (!stdin_is_terminal && incomplete);
    if reads_stdin && stdin_is_terminal {
        return fail(&Error::usage("'@' reads from stdin, but nothing is piped in"), "");
    }
    // A lone negative number continues stdin (`calcx 2x5 | calcx -1` is 9), but if stdin
    // turns out to be empty it is just the number, so `calcx $x` still works under cron.
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
            eprintln!("{}: arguments must be valid UTF-8 (check your locale)", cli::program_name());
            return ExitCode::from(2);
        }
    };

    match run(&args) {
        Ok(status) => status,
        // The reader went away (e.g. `calcx … | head`); nothing left to do.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}: {e}", cli::program_name());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_are_styled() {
        let text = "about calcx\n\nUsage: calcx [OPTIONS]\n\nOptions:\n  -h, --help\nSee README: here";
        let styled = style_headings(text, "calcx");
        let lines: Vec<&str> = styled.lines().collect();
        assert_eq!(lines[0], "about calcx");
        assert_eq!(lines[2], "\x1b[1;4mUsage:\x1b[0m \x1b[1mcalcx\x1b[0m [OPTIONS]");
        assert_eq!(lines[4], "\x1b[1;4mOptions:\x1b[0m");
        assert_eq!(lines[5], "  -h, --help");
        assert_eq!(lines[6], "See README: here"); // "See" doesn't end with ':'
    }

    #[test]
    fn bare_invocation_points_to_help() {
        let name = cli::program_name();
        assert_eq!(
            usage_hint(false),
            format!(
                "{name}: no expression\nUsage: {name} [OPTIONS] [EXPRESSION | PARTIAL EXPRESSION]\n\
                 Try '{name} --help' for more information."
            )
        );
        assert!(usage_hint(true).contains(&format!("{HEADING}Usage:{RESET} {BOLD}{name}{RESET}")));
    }

    #[test]
    fn every_heading_in_the_help_is_found() {
        let styled = style_headings(&usage(false), cli::program_name());
        for heading in ["Usage:", "Expressions:", "Options:", "Examples:"] {
            assert!(styled.contains(&format!("{HEADING}{heading}{RESET}")), "{heading}");
        }
    }
}
