//! Splits input into tokens and works out which spaces separate list elements.
//!
//! A space separates list elements only between two complete operands (`1 2`, `2 π`),
//! or before a sign attached to an operand (`1 -2`); elsewhere it is ignored (`1 + 2`).

use crate::error::{Error, Span};
use crate::functions::{self, Func};
use crate::locale::NumberFormat;
use crate::value::Value;

/// How numbers in the input are written.
#[derive(Clone, Copy)]
pub struct Syntax {
    /// The user's own decimal mark. Where it is ',', plain input like `3,5` is ambiguous.
    pub local_decimal: char,
    /// Set by `--pretty`: read numbers with this decimal mark and grouping.
    pub pretty: Option<NumberFormat>,
}

impl Default for Syntax {
    fn default() -> Syntax {
        Syntax { local_decimal: '.', pretty: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(Value),
    Plus,
    Minus,
    Mul,
    Div,
    Mod,
    Pow,
    /// A superscript power: ² or ³.
    Sup(i64),
    /// Postfix factorial: `5!`.
    Bang,
    /// Postfix degrees, converting to radians: `30°`.
    Degree,
    Sqrt,
    Pi,
    /// An opening bracket, holding the character that closes it.
    Open(char),
    Close(char),
    Sep,
    Range,
    Stdin,
    Func(Func),
}

impl Tok {
    pub fn starts_operand(&self) -> bool {
        matches!(self, Tok::Num(_) | Tok::Pi | Tok::Open(_) | Tok::Sqrt | Tok::Func(_) | Tok::Stdin)
    }

    fn ends_operand(&self) -> bool {
        matches!(self, Tok::Num(_) | Tok::Pi | Tok::Close(_) | Tok::Stdin | Tok::Sup(_) | Tok::Bang | Tok::Degree)
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    pub space_before: bool,
}

impl Token {
    /// A token that isn't in the input, such as an implied bracket or stdin.
    pub fn synthetic(tok: Tok, at: usize) -> Token {
        Token { tok, span: at..at, space_before: false }
    }
}

/// Characters that group digits in any mode: `1_000`, `1'000`, `1’000`, and no-break spaces.
fn is_digit_grouping(c: char) -> bool {
    matches!(c, '_' | '\'' | '’' | '\u{a0}' | '\u{202f}' | '\u{2009}')
}

/// A function name or word operator at the start of `chars`.
fn word_at(chars: &[char]) -> Option<(Tok, usize)> {
    const WORDS: &[(&str, Tok)] = &[("sqrt", Tok::Sqrt), ("mod", Tok::Mod), ("pi", Tok::Pi)];
    let word: String = chars.iter().take_while(|c| c.is_alphabetic()).map(|c| c.to_ascii_lowercase()).collect();
    let func = Func::prefix_of(&word).map(|(f, len)| (Tok::Func(f), len));
    let other = WORDS.iter().find(|(w, _)| word.starts_with(w)).map(|(w, tok)| (tok.clone(), w.len()));
    func.into_iter().chain(other).max_by_key(|&(_, len)| len)
}

pub fn tokenize(input: &str, syntax: &Syntax) -> Result<Vec<Token>, Error> {
    let chars: Vec<char> = input.chars().collect();
    let digit_at = |j: usize| chars.get(j).is_some_and(char::is_ascii_digit);
    let decimal_mark = syntax.pretty.map_or('.', |f| f.decimal);

    let mut tokens: Vec<Token> = Vec::new();
    let mut space_before = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            space_before = true;
            i += 1;
            continue;
        }

        let (tok, len) = if c.is_ascii_digit() || (c == '.' && decimal_mark == '.' && digit_at(i + 1)) {
            let (value, len) = scan_number(&chars, i, syntax)?;
            (Tok::Num(value), len)
        } else if c == '…' {
            (Tok::Range, 1)
        } else if c == '.' && chars.get(i + 1) == Some(&'.') {
            // `1...10` too, so a third dot isn't read as the start of `.10`.
            (Tok::Range, if chars.get(i + 2) == Some(&'.') { 3 } else { 2 })
        } else if c == '!' && chars.get(i + 1) == Some(&'!') {
            // n!! could mean (n!)! or the double factorial, and an interactive shell
            // replaces an unquoted !! with the previous command anyway.
            return Err(Error::new("'!!' isn't supported: write {n!}! for the factorial of a factorial").at(i..i + 2));
        } else if c == ',' {
            check_comma(&chars, i, space_before, syntax)?;
            (Tok::Sep, 1)
        } else if matches!(c, 'x' | 'X') && matches!(chars.get(i + 1), Some('\u{304}' | '\u{305}')) {
            // x̄ is x plus a combining macron (or overline); there's no single character for it.
            (Tok::Func(Func::Mean), 2)
        } else if c.is_alphabetic() && let Some(word) = word_at(&chars[i..]) {
            word
        } else if let Some(f) = functions::symbol(c) {
            (Tok::Func(f), 1)
        } else {
            let tok = match c {
                '+' => Tok::Plus,
                '-' | '−' => Tok::Minus,
                // Middle dot, dot operator, bullet operator and bullet all read as multiplication.
                '*' | 'x' | 'X' | '×' | '·' | '⋅' | '∙' | '•' => Tok::Mul,
                '/' | '÷' => Tok::Div,
                '%' => Tok::Mod,
                '^' => Tok::Pow,
                '!' => Tok::Bang,
                // The degree sign, and the ordinal indicator that looks just like it (Option-0 on a Mac).
                '°' | 'º' => Tok::Degree,
                '²' => Tok::Sup(2),
                '³' => Tok::Sup(3),
                '√' => Tok::Sqrt,
                'π' => Tok::Pi,
                '@' => Tok::Stdin,
                '(' => Tok::Open(')'),
                '{' => Tok::Open('}'),
                '[' => Tok::Open(']'),
                ')' | '}' | ']' => Tok::Close(c),
                _ => return Err(Error::new(format!("unexpected character '{c}'")).at(i..i + 1)),
            };
            (tok, 1)
        };
        tokens.push(Token { tok, span: i..i + len, space_before });
        space_before = false;
        i += len;
    }
    insert_separators(&mut tokens, &chars)?;
    Ok(tokens)
}

/// Scans a number literal starting at `start`: returns its value and length in characters.
fn scan_number(chars: &[char], start: usize, syntax: &Syntax) -> Result<(Value, usize), Error> {
    let decimal_mark = syntax.pretty.map_or('.', |f| f.decimal);
    let digits_from = |j: usize| chars[j..].iter().take_while(|c| c.is_ascii_digit()).count();

    let mut plain = String::new();
    let mut seen_decimal = false;
    let mut j = start;
    while let Some(&c) = chars.get(j) {
        let between_digits = j > start && chars[j - 1].is_ascii_digit() && chars.get(j + 1).is_some_and(char::is_ascii_digit);
        if c.is_ascii_digit() {
            plain.push(c);
        } else if c == '.' && chars.get(j + 1) == Some(&'.') {
            break; // a range: 1..10
        } else if c == decimal_mark {
            plain.push('.');
            seen_decimal = true;
        } else if between_digits && is_digit_grouping(c) {
            // skip
        } else if between_digits
            && !seen_decimal
            && let Some(format) = syntax.pretty
            && format.group == Some(c)
            && format.is_group_size(digits_from(j + 1))
        {
            // skip
        } else {
            break;
        }
        j += 1;
    }
    let value = Value::parse(&plain).ok_or_else(|| {
        let literal: String = chars[start..j].iter().collect();
        Error::new(format!("invalid number '{literal}'")).at(start..j)
    })?;
    Ok((value, j - start))
}

/// Rejects a comma that probably isn't a list separator: `1,234` (grouping) and,
/// where the user writes decimals with a comma, `3,5`. `1, 234` is always a list.
fn check_comma(chars: &[char], i: usize, space_before: bool, syntax: &Syntax) -> Result<(), Error> {
    let before = chars[..i].iter().rev().take_while(|c| c.is_ascii_digit()).count();
    let after = chars[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
    if space_before || before == 0 || after == 0 || syntax.pretty.is_some() {
        return Ok(());
    }
    let span = i - before..i + 1 + after;
    let left: String = chars[span.start..i].iter().collect();
    let right: String = chars[i + 1..span.end].iter().collect();
    if after == 3 {
        return Err(Error::new(format!(
            "'{left},{right}' is ambiguous: write {left}{right} or {left}_{right}, use '{left}, {right}' for a list, \
             or --pretty to read grouped numbers"
        ))
        .at(span));
    }
    if syntax.local_decimal == ',' {
        return Err(Error::new(format!(
            "'{left},{right}' is ambiguous: calc reads '.' as the decimal mark, so write {left}.{right}, \
             use '{left}, {right}' for a list, or --pretty to read local numbers"
        ))
        .at(span));
    }
    Ok(())
}

/// Turns a space into a list separator where it sits between two operands (`1 2`),
/// or before a sign that is attached to the next operand (`1 -2`, but not `1 - 2`).
fn insert_separators(tokens: &mut Vec<Token>, chars: &[char]) -> Result<(), Error> {
    let text = |t: &Token| -> String { chars[t.span.clone()].iter().collect() };
    let mut at = Vec::new();
    for i in 1..tokens.len() {
        let (prev, token) = (&tokens[i - 1], &tokens[i]);
        if !token.space_before || !prev.tok.ends_operand() {
            continue;
        }
        let signed = matches!(token.tok, Tok::Plus | Tok::Minus)
            && tokens.get(i + 1).is_some_and(|next| !next.space_before && next.tok.starts_operand());
        if !token.tok.starts_operand() && !signed {
            continue;
        }
        // "1 000" was digit grouping before spaces made lists; don't let it quietly become 1, 0.
        if let (Tok::Num(_), Tok::Num(_)) = (&prev.tok, &token.tok) {
            let (left, right) = (text(prev), text(token));
            if right.len() == 3 && right.starts_with('0') && right.chars().all(|c| c.is_ascii_digit()) {
                return Err(Error::new(format!(
                    "'{left} {right}' looks like digit grouping: write {left}{right} or {left}_{right} \
                     (spaces separate list elements)"
                ))
                .at(prev.span.start..token.span.end));
            }
        }
        at.push(i);
    }
    for i in at.into_iter().rev() {
        let start = tokens[i].span.start;
        tokens.insert(i, Token { tok: Tok::Sep, span: start - 1..start, space_before: true });
    }
    Ok(())
}
