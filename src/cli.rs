//! Command line options. Options come first and end at the first argument that
//! isn't one, or at `--`, so `calcx -2` and `calcx -pi` are expressions.

use std::path::Path;
use std::sync::OnceLock;

use crate::error::Error;
use crate::format::Precision;
use crate::functions::Func;
use crate::locale::NumberFormat;

/// The name the program was run as (the file name of argv[0]), so help and errors
/// match a renamed binary, a symlink or a snap. Falls back to the package name.
pub fn program_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        std::env::args_os()
            .next()
            .and_then(|arg0| Path::new(&arg0).file_name().map(|f| f.to_string_lossy().into_owned()))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| env!("CARGO_PKG_NAME").to_string())
    })
}

#[derive(Clone, Copy, PartialEq)]
pub enum When {
    Always,
    Auto,
    Never,
}

impl When {
    pub fn parse(value: &str) -> Option<When> {
        match value {
            "always" => Some(When::Always),
            "auto" => Some(When::Auto),
            "never" => Some(When::Never),
            _ => None,
        }
    }
}

/// `--pretty` as given on the command line; unlike `CALCX_PRETTY` it also changes how input is read.
#[derive(Clone, Copy)]
pub struct Pretty {
    pub when: When,
    /// From `--pretty=EXAMPLE`.
    pub format: Option<NumberFormat>,
}

pub struct Options {
    pub precision: Precision,
    pub pretty: Option<Pretty>,
    /// `-r`/`--rows`: preserve rows, evaluating each line of stdin as its own list.
    pub rows: bool,
}

pub enum Command<'a> {
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
pub fn parse_options(args: &[String]) -> Result<Command<'_>, Error> {
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
pub fn check_brace_expansion(args: &[String]) -> Result<(), Error> {
    // The function name an argument starts with, when a digit follows it: `avg1`.
    let prefix = |arg: &String| {
        let lower = arg.to_lowercase();
        let (_, len) = Func::prefix_of(&lower)?;
        lower[len..].starts_with(|c: char| c.is_ascii_digit()).then(|| lower[..len].to_string())
    };
    let Some(first) = args.first().and_then(prefix) else {
        return Ok(());
    };
    if args.len() >= 2 && args.iter().all(|a| prefix(a).as_ref() == Some(&first)) {
        return Err(Error::usage(format!(
            "'{}' looks like the shell expanded {first}{{…,…}}: write {first} 1,2,3, {first}{{1 2 3}} or quote it",
            args.join(" ")
        )));
    }
    Ok(())
}
