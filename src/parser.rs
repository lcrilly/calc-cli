//! Tokeniser and recursive-descent evaluator. Every value is a list; a single
//! number is a list of one, and arithmetic applies element by element.
//!
//! Grammar (lowest to highest precedence):
//!   list    := element (SEP element)* SEP?                -- SEP is ',' or a separating space
//!   element := expr ('..' expr)?
//!   expr    := term (('+' | '-') term)*
//!   term    := unary (('×' | '÷' | '%') unary | unary)*   -- adjacency is implicit ×
//!   unary   := ('+' | '-') unary | power
//!   power   := root ('^' unary)?                         -- right-associative
//!   root    := '√' ('+' | '-')* root | primary
//!   primary := number | 'π' | '@' | '(' list ')' | call
//!   call    := FUNC '(' list ')' | FUNC list              -- without a bracket, takes the rest
//!
//! A space separates list elements only between two complete operands (`1 2`, `2 π`),
//! or before a sign attached to an operand (`1 -2`); elsewhere it is ignored (`1 + 2`).
//! A closing bracket with no opening bracket implies one at the very start of the
//! expression: `2+4+8}/2` is `{2+4+8}/2`.

use crate::Error;
use crate::functions::{self, Func, Kind};
use crate::locale::{self, NumberFormat};
use crate::value::{Ratio, Value};

/// How numbers in the input are written.
#[derive(Clone, Copy, Default)]
pub struct Syntax {
    /// Set by `--pretty`: read numbers with this decimal mark and grouping.
    pub pretty: Option<NumberFormat>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Num(Value),
    Plus,
    Minus,
    Mul,
    Div,
    Mod,
    Pow,
    Sqrt,
    Pi,
    Open(char),
    Close(char),
    Sep,
    Range,
    Stdin,
    Func(Func),
}

#[derive(Clone)]
struct Token {
    tok: Tok,
    text: String,
    /// 1-based character position in the original input, for error messages.
    pos: usize,
    space_before: bool,
}

/// A tokenised expression, before separators and implied brackets are worked out.
pub struct Expression {
    tokens: Vec<Token>,
    decimal_input: bool,
}

/// An expression ready to evaluate, any number of times.
pub struct Program {
    tokens: Vec<Token>,
    pub decimal_input: bool,
}

pub struct Evaluation {
    pub values: Vec<Value>,
    /// True when any literal was written with a decimal point.
    pub decimal_input: bool,
}

/// Evaluates an expression that doesn't read stdin (an argument list or one line of input).
pub fn evaluate(input: &str, syntax: &Syntax) -> Result<Evaluation, Error> {
    let expression = Expression::parse(input, syntax)?;
    if expression.tokens.is_empty() {
        return Err(Error::new("empty expression"));
    }
    if expression.reads_stdin() {
        return Err(Error::new("'@' can only be used in arguments"));
    }
    let program = expression.compile()?;
    Ok(Evaluation { values: program.evaluate(None)?, decimal_input: program.decimal_input })
}

impl Expression {
    pub fn parse(input: &str, syntax: &Syntax) -> Result<Expression, Error> {
        let (tokens, decimal_input) = tokenize(input, syntax)?;
        Ok(Expression { tokens, decimal_input })
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// True when the expression uses `@` explicitly.
    pub fn reads_stdin(&self) -> bool {
        self.tokens.iter().any(|t| t.tok == Tok::Stdin)
    }

    /// True when the expression starts with an operator, so it continues a value read from stdin.
    /// A leading '-' is left out: `calc -2` is a negative number, and must not wait on stdin.
    pub fn is_continuation(&self) -> bool {
        self.tokens.first().is_some_and(|t| matches!(t.tok, Tok::Plus | Tok::Mul | Tok::Div | Tok::Mod | Tok::Pow))
    }

    /// True when the expression ends with an operator or function, so stdin completes it.
    pub fn awaits_operand(&self) -> bool {
        self.tokens.last().is_some_and(|t| {
            matches!(
                t.tok,
                Tok::Plus | Tok::Minus | Tok::Mul | Tok::Div | Tok::Mod | Tok::Pow | Tok::Sqrt | Tok::Func(_) | Tok::Range
            )
        })
    }

    /// Places stdin (`@`) wherever the expression lacks an operand, unless it already says where.
    pub fn feed_stdin(&mut self) {
        if self.reads_stdin() {
            return;
        }
        let stdin = || Token { tok: Tok::Stdin, text: "@".into(), pos: 0, space_before: false };
        if self.tokens.is_empty() {
            self.tokens.push(stdin());
            return;
        }
        if self.awaits_operand() {
            self.tokens.push(stdin());
        }
        if self.is_continuation() {
            self.tokens.insert(0, stdin());
        }
    }

    pub fn compile(mut self) -> Result<Program, Error> {
        insert_separators(&mut self.tokens)?;
        infer_opening_brackets(&mut self.tokens);
        Ok(Program { tokens: self.tokens, decimal_input: self.decimal_input })
    }
}

impl Program {
    /// True when stdin can be processed a line at a time with the same result as
    /// reading it all as one list: nothing combines or reorders elements.
    pub fn streams(&self) -> bool {
        self.tokens.iter().all(|t| match t.tok {
            Tok::Sep | Tok::Range => false,
            Tok::Func(f) => f.kind() == Kind::Elementwise,
            _ => true,
        })
    }

    pub fn evaluate(&self, stdin: Option<&[Value]>) -> Result<Vec<Value>, Error> {
        let mut parser = Parser { tokens: &self.tokens, i: 0, stdin };
        let values = parser.list()?;
        if let Some(t) = parser.tokens.get(parser.i) {
            return Err(match t.tok {
                Tok::Close(c) => Error::new(format!("unmatched '{c}' at position {}", t.pos)),
                _ => Error::new(format!("unexpected '{}' at position {}", t.text, t.pos)),
            });
        }
        Ok(values)
    }
}

/// Characters that group digits in any mode: `1_000`, `1'000`, `1’000`, and no-break spaces.
fn is_digit_grouping(c: char) -> bool {
    matches!(c, '_' | '\'' | '’' | '\u{a0}' | '\u{202f}' | '\u{2009}')
}

fn word_at(chars: &[char], i: usize) -> Option<(Tok, usize)> {
    const WORDS: &[(&str, Tok)] = &[("sqrt", Tok::Sqrt), ("mod", Tok::Mod), ("pi", Tok::Pi)];
    let matches = |word: &str| {
        word.chars().enumerate().all(|(k, w)| chars.get(i + k).is_some_and(|c| c.to_ascii_lowercase() == w))
    };
    functions::NAMES
        .iter()
        .map(|&(name, f)| (name, Tok::Func(f)))
        .chain(WORDS.iter().copied())
        .filter(|(name, _)| matches(name))
        .max_by_key(|(name, _)| name.len())
        .map(|(name, tok)| (tok, name.len()))
}

fn tokenize(input: &str, syntax: &Syntax) -> Result<(Vec<Token>, bool), Error> {
    let chars: Vec<char> = input.chars().collect();
    let digit_at = |j: usize| chars.get(j).is_some_and(char::is_ascii_digit);
    let decimal_mark = syntax.pretty.map_or('.', |f| f.decimal);

    let mut tokens: Vec<Token> = Vec::new();
    let mut decimal_input = false;
    let mut space_before = false;
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = (i + 1, chars[i]);
        if c.is_whitespace() {
            space_before = true;
            i += 1;
            continue;
        }

        let (tok, len) = if c.is_ascii_digit() || (c == '.' && decimal_mark == '.' && digit_at(i + 1)) {
            let (value, len, decimal) = scan_number(&chars, i, syntax)?;
            decimal_input |= decimal;
            (Tok::Num(value), len)
        } else if c == '…' {
            (Tok::Range, 1)
        } else if c == '.' && chars.get(i + 1) == Some(&'.') {
            // `1...10` too, so a third dot isn't read as the start of `.10`.
            (Tok::Range, if chars.get(i + 2) == Some(&'.') { 3 } else { 2 })
        } else if c == ',' {
            check_comma(&chars, i, space_before, syntax)?;
            (Tok::Sep, 1)
        } else if matches!(c, 'x' | 'X') && matches!(chars.get(i + 1), Some('\u{304}' | '\u{305}')) {
            // x̄ is x plus a combining macron (or overline); there's no single character for it.
            (Tok::Func(Func::Mean), 2)
        } else if c.is_alphabetic() && let Some(word) = word_at(&chars, i) {
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
                '√' => Tok::Sqrt,
                'π' => Tok::Pi,
                '@' => Tok::Stdin,
                '(' => Tok::Open(')'),
                '{' => Tok::Open('}'),
                '[' => Tok::Open(']'),
                ')' | '}' | ']' => Tok::Close(c),
                _ => return Err(Error::new(format!("unexpected character '{c}' at position {pos}"))),
            };
            (tok, 1)
        };
        let text = chars[i..i + len].iter().collect();
        tokens.push(Token { tok, text, pos, space_before });
        space_before = false;
        i += len;
    }
    Ok((tokens, decimal_input))
}

/// Scans a number literal starting at `start`: returns its value, length in characters,
/// and whether it has a decimal part.
fn scan_number(chars: &[char], start: usize, syntax: &Syntax) -> Result<(Value, usize, bool), Error> {
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
    let literal: String = chars[start..j].iter().collect();
    let value = parse_number(&plain)
        .ok_or_else(|| Error::new(format!("invalid number '{literal}' at position {}", start + 1)))?;
    Ok((value, j - start, seen_decimal))
}

/// Rejects a comma that probably isn't a list separator: `1,234` (grouping) and,
/// where the locale writes decimals with a comma, `3,5`. `1, 234` is always a list.
fn check_comma(chars: &[char], i: usize, space_before: bool, syntax: &Syntax) -> Result<(), Error> {
    let before = chars[..i].iter().rev().take_while(|c| c.is_ascii_digit()).count();
    let after = chars[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
    if space_before || before == 0 || after == 0 || syntax.pretty.is_some() {
        return Ok(());
    }
    let shown: String = chars[i - before..i + 1 + after].iter().collect();
    let (left, right) = shown.split_once(',').unwrap();
    if after == 3 {
        return Err(Error::new(format!(
            "'{shown}' is ambiguous: write {left}{right} or {left}_{right}, use '{left}, {right}' for a list, \
             or --pretty to read grouped numbers"
        )));
    }
    if locale::detected().decimal == ',' {
        return Err(Error::new(format!(
            "'{shown}' is ambiguous: calc reads '.' as the decimal mark, so write {left}.{right}, \
             use '{left}, {right}' for a list, or --pretty to read local numbers"
        )));
    }
    Ok(())
}

fn ends_operand(tok: Tok) -> bool {
    matches!(tok, Tok::Num(_) | Tok::Pi | Tok::Close(_) | Tok::Stdin)
}

fn starts_operand(tok: Tok) -> bool {
    matches!(tok, Tok::Num(_) | Tok::Pi | Tok::Open(_) | Tok::Sqrt | Tok::Func(_) | Tok::Stdin)
}

/// Turns a space into a list separator where it sits between two operands (`1 2`),
/// or before a sign that is attached to the next operand (`1 -2`, but not `1 - 2`).
fn insert_separators(tokens: &mut Vec<Token>) -> Result<(), Error> {
    let mut at = Vec::new();
    for i in 1..tokens.len() {
        let (prev, token) = (&tokens[i - 1], &tokens[i]);
        if !token.space_before || !ends_operand(prev.tok) {
            continue;
        }
        let signed = matches!(token.tok, Tok::Plus | Tok::Minus)
            && tokens.get(i + 1).is_some_and(|next| !next.space_before && starts_operand(next.tok));
        if !starts_operand(token.tok) && !signed {
            continue;
        }
        // "1 000" was digit grouping before spaces made lists; don't let it quietly become 1, 0.
        if let (Tok::Num(_), Tok::Num(_)) = (prev.tok, token.tok)
            && token.text.len() == 3
            && token.text.starts_with('0')
            && token.text.chars().all(|c| c.is_ascii_digit())
        {
            return Err(Error::new(format!(
                "'{} {}' looks like digit grouping: write {0}{1} or {0}_{1} (spaces separate list elements)",
                prev.text, token.text
            )));
        }
        at.push(i);
    }
    for i in at.into_iter().rev() {
        let pos = tokens[i].pos;
        tokens.insert(i, Token { tok: Tok::Sep, text: " ".into(), pos, space_before: true });
    }
    Ok(())
}

/// Adds an opening bracket at the start for each closing bracket that has none.
///
/// An unmatched closer can only occur when every bracket before it is balanced,
/// so the inferred opener always pairs with exactly that closer. Later closers
/// get outer brackets: `1+2)x3}` becomes `{(1+2)x3}`. A closer that mismatches
/// an open bracket (`{1+2)`) is left for the parser to report.
fn infer_opening_brackets(tokens: &mut Vec<Token>) {
    let mut open = Vec::new();
    let mut inferred = Vec::new();
    for token in tokens.iter() {
        match token.tok {
            Tok::Open(close) => open.push(close),
            Tok::Close(c) if open.is_empty() => inferred.push((c, token.pos)),
            Tok::Close(c) if open.last() == Some(&c) => {
                open.pop();
            }
            Tok::Close(_) => break,
            _ => {}
        }
    }
    let opening = |c: char| match c {
        ')' => '(',
        '}' => '{',
        _ => '[',
    };
    // The last unmatched closer is outermost, so its opener goes first.
    tokens.splice(
        0..0,
        inferred.into_iter().rev().map(|(c, pos)| Token {
            tok: Tok::Open(c),
            text: opening(c).to_string(),
            pos,
            space_before: false,
        }),
    );
}

/// Parses a plain decimal literal exactly; falls back to f64 if it doesn't fit in i128.
fn parse_number(literal: &str) -> Option<Value> {
    let (int_part, frac_part) = literal.split_once('.').unwrap_or((literal, ""));
    if frac_part.contains('.') || (int_part.is_empty() && frac_part.is_empty()) {
        return None;
    }
    let exact = || {
        let digits: i128 = format!("{int_part}{frac_part}").parse().ok()?;
        let scale = 10i128.checked_pow(u32::try_from(frac_part.len()).ok()?)?;
        Ratio::new(digits, scale)
    };
    match exact() {
        Some(r) => Some(Value::Exact(r)),
        None => literal.parse().ok().map(Value::Approx),
    }
}

/// Applies `f` element by element. A list of one pairs with every element of the other.
fn zip(a: Vec<Value>, b: Vec<Value>, f: fn(Value, Value) -> Result<Value, Error>) -> Result<Vec<Value>, Error> {
    match (a.len(), b.len()) {
        (m, n) if m == n => a.into_iter().zip(b).map(|(x, y)| f(x, y)).collect(),
        (1, _) => b.into_iter().map(|y| f(a[0], y)).collect(),
        (_, 1) => a.into_iter().map(|x| f(x, b[0])).collect(),
        (m, n) => Err(Error::new(format!("can't combine lists of {m} and {n} numbers"))),
    }
}

fn map(a: Vec<Value>, f: fn(Value) -> Result<Value, Error>) -> Result<Vec<Value>, Error> {
    a.into_iter().map(f).collect()
}

/// Ranges stop well short of exhausting memory.
const MAX_RANGE: usize = 10_000_000;

fn range(from: Vec<Value>, to: Vec<Value>) -> Result<Vec<Value>, Error> {
    let ([from], [to]) = (from.as_slice(), to.as_slice()) else {
        return Err(Error::new("a range needs a single number at each end"));
    };
    let (from, to) = (*from, *to);
    let ascending = from.cmp(to).is_le();
    let step = Value::from_int(if ascending { 1 } else { -1 });
    let mut out = Vec::new();
    let mut v = from;
    while if ascending { v.cmp(to).is_le() } else { v.cmp(to).is_ge() } {
        if out.len() == MAX_RANGE {
            return Err(Error::new(format!("range is longer than {MAX_RANGE} numbers")));
        }
        out.push(v);
        v = v.add(step)?;
    }
    Ok(out)
}

struct Parser<'a> {
    tokens: &'a [Token],
    i: usize,
    stdin: Option<&'a [Value]>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Tok> {
        self.tokens.get(self.i).map(|t| t.tok)
    }

    fn at_list_end(&self) -> bool {
        matches!(self.peek(), None | Some(Tok::Close(_)))
    }

    fn list(&mut self) -> Result<Vec<Value>, Error> {
        let mut values = Vec::new();
        if self.at_list_end() {
            return Ok(values);
        }
        loop {
            values.extend(self.element()?);
            if self.peek() != Some(Tok::Sep) {
                return Ok(values);
            }
            self.i += 1;
            if self.at_list_end() {
                return Ok(values); // trailing comma
            }
        }
    }

    fn element(&mut self) -> Result<Vec<Value>, Error> {
        let from = self.expr()?;
        if self.peek() == Some(Tok::Range) {
            self.i += 1;
            return range(from, self.expr()?);
        }
        Ok(from)
    }

    fn expr(&mut self) -> Result<Vec<Value>, Error> {
        let mut value = self.term()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => Value::add,
                Some(Tok::Minus) => Value::sub,
                _ => return Ok(value),
            };
            self.i += 1;
            value = zip(value, self.term()?, op)?;
        }
    }

    fn term(&mut self) -> Result<Vec<Value>, Error> {
        let mut value = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Mul) => Value::mul,
                Some(Tok::Div) => Value::div,
                Some(Tok::Mod) => Value::rem,
                // Implicit multiplication: 2π, 3√2, 2{1+1}, {1+1}{2+2}, 2sum{1 2}
                Some(tok) if starts_operand(tok) => {
                    value = zip(value, self.unary()?, Value::mul)?;
                    continue;
                }
                _ => return Ok(value),
            };
            self.i += 1;
            value = zip(value, self.unary()?, op)?;
        }
    }

    fn unary(&mut self) -> Result<Vec<Value>, Error> {
        match self.peek() {
            Some(Tok::Plus) => {
                self.i += 1;
                self.unary()
            }
            Some(Tok::Minus) => {
                self.i += 1;
                map(self.unary()?, Value::neg)
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Vec<Value>, Error> {
        let base = self.root()?;
        if self.peek() == Some(Tok::Pow) {
            self.i += 1;
            return zip(base, self.unary()?, Value::pow);
        }
        Ok(base)
    }

    fn root(&mut self) -> Result<Vec<Value>, Error> {
        if self.peek() != Some(Tok::Sqrt) {
            return self.primary();
        }
        self.i += 1;
        let mut negate = false;
        while let Some(sign @ (Tok::Plus | Tok::Minus)) = self.peek() {
            negate ^= sign == Tok::Minus;
            self.i += 1;
        }
        let operand = self.root()?;
        let operand = if negate { map(operand, Value::neg)? } else { operand };
        map(operand, Value::sqrt)
    }

    fn primary(&mut self) -> Result<Vec<Value>, Error> {
        let Some(token) = self.tokens.get(self.i) else {
            return Err(Error::new("unexpected end of expression"));
        };
        self.i += 1;
        match token.tok {
            Tok::Num(value) => Ok(vec![value]),
            Tok::Pi => Ok(vec![Value::Approx(std::f64::consts::PI)]),
            Tok::Stdin => self.stdin.map(<[Value]>::to_vec).ok_or_else(|| Error::new("'@' needs input on stdin")),
            Tok::Open(close) => {
                // Empty brackets are only meaningful as a function's list: sum{}.
                if let Some(t) = self.tokens.get(self.i).filter(|t| matches!(t.tok, Tok::Close(_))) {
                    return Err(Error::new(format!("unexpected '{}' at position {}", t.text, t.pos)));
                }
                self.bracketed(close, token.pos)
            }
            Tok::Func(f) => {
                let args = match self.tokens.get(self.i) {
                    Some(t) if !t.space_before && let Tok::Open(close) = t.tok => {
                        self.i += 1;
                        self.bracketed(close, t.pos)?
                    }
                    _ if self.at_list_end() => {
                        return Err(Error::new(format!(
                            "'{}' at position {} needs numbers after it",
                            token.text, token.pos
                        )));
                    }
                    _ => self.list()?,
                };
                f.apply(args)
            }
            _ => Err(Error::new(format!("unexpected '{}' at position {}", token.text, token.pos))),
        }
    }

    /// Parses a list up to `close`; the opening bracket at `pos` is already consumed.
    fn bracketed(&mut self, close: char, pos: usize) -> Result<Vec<Value>, Error> {
        let values = self.list()?;
        match self.tokens.get(self.i) {
            Some(t) if t.tok == Tok::Close(close) => {
                self.i += 1;
                Ok(values)
            }
            Some(t) => Err(Error::new(format!(
                "expected '{close}' to match bracket at position {pos}, found '{}' at position {}",
                t.text, t.pos
            ))),
            None => Err(Error::new(format!("missing '{close}' for bracket at position {pos}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{Precision, format};

    fn eval_with(input: &str, syntax: &Syntax) -> String {
        match evaluate(input, syntax) {
            Ok(e) => {
                e.values.iter().map(|&v| format(v, e.decimal_input, Precision::Default)).collect::<Vec<_>>().join(" ")
            }
            Err(e) => format!("error: {e}"),
        }
    }

    fn eval(input: &str) -> String {
        eval_with(input, &Syntax::default())
    }

    fn same(a: &str, b: &str) {
        assert_eq!(eval(a), eval(b), "{a} vs {b}");
    }

    #[test]
    fn unmatched_closer_wraps_prefix() {
        same("2+4+8}/2", "7");
        same("1+2x3}", "7");
        same("-2}^2", "4");
        same("2+4}3", "18");
        same("2}{3}", "6");
    }

    #[test]
    fn nesting() {
        same("{1+2}+3}x2", "12");
        same("1+2}x3}+1", "{{1+2}x3}+1");
        same("1+2)x3}", "{(1+2)x3}");
        same("1+{2x3}}x2", "14");
        same("{1}+4}/2", "2.5");
    }

    #[test]
    fn errors_remain() {
        assert!(eval("{1+2)").starts_with("error: expected '}'"));
        assert!(eval("{1+2)x3}").starts_with("error"));
        assert!(eval("}").starts_with("error: unexpected '}' at position 1"));
        assert!(eval("{1+2").starts_with("error: missing '}'"));
    }

    #[test]
    fn spaces_and_commas_make_lists() {
        assert_eq!(eval("1 2 3"), "1 2 3");
        assert_eq!(eval("1,2,3,"), "1 2 3");
        assert_eq!(eval("1, 234"), "1 234");
        assert_eq!(eval("2 x {3 + 4}"), "14");
        assert_eq!(eval("1 + 2 3"), "3 3");
        assert_eq!(eval("1 -2"), "1 -2");
        assert_eq!(eval("1 - 2"), "-1");
        assert_eq!(eval("1 -2 - 3"), "1 -5");
        assert_eq!(eval("2π"), "6.28319");
        assert_eq!(eval("2 π"), "2 3.14159");
        assert_eq!(eval("1_000x3"), "3000");
        assert!(eval("1 000 x 3").contains("looks like digit grouping"));
        assert!(eval("1,234").contains("ambiguous"));
        assert!(eval("1,234.5").contains("ambiguous"));
    }

    #[test]
    fn lists_broadcast() {
        assert_eq!(eval("{1 2 3}x2"), "2 4 6");
        assert_eq!(eval("2^{1 2 3}"), "2 4 8");
        assert_eq!(eval("{1 2}+{10 20}"), "11 22");
        assert_eq!(eval("√{1 4 9}"), "1 2 3");
        assert!(eval("{1 2}+{1 2 3}").contains("can't combine lists of 2 and 3"));
        assert_eq!(eval("1 2 3}x2"), "2 4 6");
    }

    #[test]
    fn functions() {
        assert_eq!(eval("sum 1 2 3"), "6");
        assert_eq!(eval("SUM(1,2,3)"), "6");
        assert_eq!(eval("∑1,2,3}x2"), "12");
        assert_eq!(eval("sum{1 2 3}x2"), "12");
        assert_eq!(eval("2sum{1 2}"), "6");
        assert_eq!(eval("Σ1 2 3"), "6");
        assert_eq!(eval("∏1 2 3 4"), "24");
        assert_eq!(eval("avg 1 2 3 4"), "2.5");
        assert_eq!(eval("x\u{304} 1 2 3 4"), "2.5");
        assert_eq!(eval("x\u{305}{1 2 3 4}"), "2.5");
        assert_eq!(eval("2x\u{304}{1 3}"), "4");
        assert_eq!(eval("2x{1 3}"), "2 6");
        for dot in ["·", "⋅", "∙", "•"] {
            assert_eq!(eval(&format!("2{dot}3")), "6", "{dot}");
        }
        assert_eq!(eval("median 3 1 2"), "2");
        assert_eq!(eval("median 4 1 3 2"), "2.5");
        assert_eq!(eval("max 3 -7 2"), "3");
        assert_eq!(eval("min 3 -7 2"), "-7");
        assert_eq!(eval("count 5 5 5"), "3");
        assert_eq!(eval("stdev 2 4 4 4 5 5 7 9"), "2.13809");
        assert_eq!(eval("2xmax{1 5}"), "10");
        assert_eq!(eval("cumsum 1..5"), "1 3 6 10 15");
        assert_eq!(eval("cumprod 1..5"), "1 2 6 24 120");
        assert_eq!(eval("sort 3 1 2"), "1 2 3");
        assert_eq!(eval("abs -3 + 1"), "2");
        assert_eq!(eval("round{2.5 -2.5 2.4}"), "3.0 -3.0 2.0");
        assert_eq!(eval("floor 7/2"), "3");
        assert_eq!(eval("ceil 7/2"), "4");
        assert_eq!(eval("log 1000"), "3");
        assert_eq!(eval("sum{}"), "0");
        assert!(eval("sum").contains("needs numbers after it"));
    }

    #[test]
    fn ranges() {
        assert_eq!(eval("1..5"), "1 2 3 4 5");
        assert_eq!(eval("3..1"), "3 2 1");
        assert_eq!(eval("sum 1..100"), "5050");
        assert_eq!(eval("1..2x3"), "1 2 3 4 5 6");
        assert_eq!(eval("1…5"), "1 2 3 4 5");
        assert_eq!(eval("1...5"), "1 2 3 4 5");
        assert_eq!(eval("sum 1 … 4"), "10");
    }

    #[test]
    fn pretty_input() {
        let en = Syntax { pretty: Some(NumberFormat::from_example("1,234.5").unwrap()) };
        let de = Syntax { pretty: Some(NumberFormat::from_example("1.234,5").unwrap()) };
        assert_eq!(eval_with("3,141.6+1", &en), "3142.6");
        assert_eq!(eval_with("1,2", &en), "1 2");
        assert_eq!(eval_with("12,34,567", &Syntax { pretty: NumberFormat::from_example("12,34,567.8").ok() }), "1234567");
        assert_eq!(eval_with("3.141,6+1", &de), "3142.6");
        assert_eq!(eval_with("1,5 2,5", &de), "1.5 2.5");
        assert!(eval_with("3.14", &de).starts_with("error"));
        assert_eq!(eval("1\u{202f}234,5"), "1234 5");
    }
}
