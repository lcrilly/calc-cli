//! End-to-end tests that run the built binary.

use std::io::Write;
use std::process::{Command, Stdio};

struct Output {
    stdout: String,
    stderr: String,
    code: i32,
}

/// Runs calc with `args` and optional `stdin`, isolated from the user's locale settings.
fn calc_env(args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_calc"));
    command
        .args(args)
        .env_remove("CALC_PRETTY")
        .env_remove("CALC_NUMBER_FORMAT")
        .env("LC_ALL", "C")
        .envs(env.iter().copied())
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(input) = stdin {
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    Output {
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
        code: out.status.code().unwrap(),
    }
}

fn calc(args: &[&str], stdin: Option<&str>) -> Output {
    calc_env(args, stdin, &[])
}

fn ok(args: &[&str], stdin: Option<&str>) -> String {
    let out = calc(args, stdin);
    assert_eq!(out.code, 0, "{args:?}: {}", out.stderr);
    out.stdout
}

#[test]
fn readme_examples_still_hold() {
    assert_eq!(ok(&["2x{3+4}"], None), "14\n");
    assert_eq!(ok(&["2", "x", "{3", "+", "4}"], None), "14\n");
    assert_eq!(ok(&["-p=10", "√2"], None), "1.4142135624\n");
    assert_eq!(ok(&["--precision=max"], Some("2π\n")), "6.283185307179586\n");
    assert_eq!(ok(&["2^"], Some("1\n2\n3\n")), "2\n4\n8\n");
    assert_eq!(ok(&["+4}/2"], Some("1\n2\n3\n")), "2.5\n3\n3.5\n");
    assert_eq!(ok(&["^2"], Some("-2\n")), "4\n");
    assert_eq!(ok(&["²"], Some("-2\n3\n")), "4\n9\n");
}

#[test]
fn lists_on_the_command_line() {
    assert_eq!(ok(&["1", "2", "3"], None), "1\n2\n3\n");
    assert_eq!(ok(&["1", "-2"], None), "1\n-2\n");
    assert_eq!(ok(&["-2", "3"], None), "-2\n3\n");
    assert_eq!(ok(&["sum", "1..100"], None), "5050\n");
    assert_eq!(ok(&["∑1,2,3}x2"], None), "12\n");
}

#[test]
fn stdin_is_one_list() {
    assert_eq!(ok(&["sum"], Some("1\n2\n3\n4\n")), "10\n");
    assert_eq!(ok(&["sum"], Some("1 2\n3 4\n")), "10\n");
    assert_eq!(ok(&["max{@}-min{@}"], Some("3\n9\n4\n")), "6\n");
    assert_eq!(ok(&["sum"], Some("")), "0\n");
    assert_eq!(ok(&[], Some("1+1\n\n2π\n")), "2\n6.28319\n");
}

#[test]
fn syntax_errors_come_before_stdin() {
    // Reported once, against the arguments, not once per line of input.
    let out = calc(&["x{2"], Some("1\n2\n3\n"));
    assert_eq!((out.stdout.as_str(), out.code), ("", 1));
    assert_eq!(out.stderr, "calc: missing '}'\n  x{2\n   ^\n");
}

#[test]
fn errors_show_where() {
    assert_eq!(calc(&["2x{3+4"], None).stderr, "calc: missing '}'\n  2x{3+4\n    ^\n");
    let out = calc(&["12/"], Some("1\n0\n"));
    assert_eq!(out.stdout, "12\n");
    assert_eq!(out.stderr, "calc: line 2: division by zero\n  12/\n  ^^^\n");
}

#[test]
fn rows_are_preserved() {
    assert_eq!(ok(&["-r", "sum"], Some("1 2\n3 4\n")), "3\n7\n");
    assert_eq!(ok(&["--rows", "x10"], Some("1 2\n3 4\n")), "10 20\n30 40\n");
}

#[test]
fn complete_arguments_leave_stdin_alone() {
    assert_eq!(ok(&["1+2"], Some("5\n")), "3\n");
}

#[test]
fn bad_lines() {
    // Element by element: report the line, carry on.
    let out = calc(&["x2"], Some("1\nfoo\n3\n"));
    assert_eq!((out.stdout.as_str(), out.code), ("2\n6\n", 1));
    assert!(out.stderr.contains("line 2:"));
    // Aggregates: no result at all.
    let out = calc(&["sum"], Some("1\nfoo\n3\n"));
    assert_eq!((out.stdout.as_str(), out.code), ("", 1));
    assert!(out.stderr.contains("line 2:"));
}

#[test]
fn ambiguous_numbers_are_errors() {
    assert!(calc(&["1,234"], None).stderr.contains("ambiguous"));
    assert!(calc(&["1", "000", "x", "3"], None).stderr.contains("digit grouping"));
    assert_eq!(ok(&["1_000x3"], None), "3000\n");
    // With a decimal-comma locale, 3,5 is suspicious too.
    let out = calc_env(&["3,5"], None, &[("CALC_NUMBER_FORMAT", "1.234,5")]);
    assert!(out.stderr.contains("'.' as the decimal mark"));
}

#[test]
fn pretty() {
    assert_eq!(ok(&["--pretty=1,234.5", "-p1", "1000π"], None), "3,141.6\n");
    assert_eq!(ok(&["--pretty=1.234,5", "-p1", "1000π"], None), "3.141,6\n");
    assert_eq!(ok(&["--pretty=1,234.5", "+1"], Some("3,141.6\n")), "3,142.6\n");
    assert_eq!(ok(&["--pretty=1.234,5", "+1"], Some("3.141,6\n")), "3.142,6\n");
    let env = [("CALC_NUMBER_FORMAT", "1.234,5")];
    assert_eq!(calc_env(&["--pretty", "1234567/2"], None, &env).stdout, "617.283,5\n");
    // CALC_PRETTY changes output only; auto means only to a terminal.
    let env = [("CALC_NUMBER_FORMAT", "1,234.5"), ("CALC_PRETTY", "always")];
    assert_eq!(calc_env(&["1234567"], None, &env).stdout, "1,234,567\n");
    assert!(calc_env(&["1,234"], None, &env).stderr.contains("ambiguous"));
    let env = [("CALC_NUMBER_FORMAT", "1,234.5"), ("CALC_PRETTY", "auto")];
    assert_eq!(calc_env(&["1234567"], None, &env).stdout, "1234567\n");
    // The C locale has no grouping.
    assert_eq!(ok(&["--pretty", "1234567"], None), "1234567\n");
}

#[test]
fn shell_brace_expansion_is_caught() {
    // bash turns avg{1,2,3} into avg1 avg2 avg3.
    let out = calc(&["avg1", "avg2", "avg3"], None);
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("brace") || out.stderr.contains("expanded"));
    assert_eq!(ok(&["avg", "1,2,3"], None), "2\n");
}

#[test]
fn options() {
    assert_eq!(calc(&["--bogus", "1"], None).code, 2);
    assert_eq!(calc(&["--pretty=nonsense", "1"], None).code, 2);
    // After `--`, "-r" is an (invalid) expression rather than --rows.
    assert_eq!(calc(&["--", "-r"], None).code, 1);
    assert_eq!(ok(&["-p2", "--", "-1/3"], None), "-0.33\n");
}
