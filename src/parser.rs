//! Tokeniser and recursive-descent evaluator.
//!
//! Grammar (lowest to highest precedence):
//!   expr    := term (('+' | '-') term)*
//!   term    := unary (('×' | '÷') unary | unary)*     -- adjacency is implicit ×
//!   unary   := ('+' | '-') unary | power
//!   power   := root ('^' unary)?                        -- right-associative
//!   root    := '√' ('+' | '-')* root | primary
//!   primary := number | 'π' | '(' expr ')'

use crate::Error;
use crate::value::{Ratio, Value};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Num(Value),
    Plus,
    Minus,
    Mul,
    Div,
    Pow,
    Sqrt,
    Pi,
    Open(char),
    Close(char),
}

struct Token {
    tok: Tok,
    text: String,
    /// 1-based character position in the original input, for error messages.
    pos: usize,
}

pub struct Evaluation {
    pub value: Value,
    /// True when any literal was written with a decimal point.
    pub decimal_input: bool,
}

pub fn evaluate(input: &str) -> Result<Evaluation, Error> {
    let (tokens, decimal_input) = tokenize(input)?;
    if tokens.is_empty() {
        return Err(Error::new("empty expression"));
    }
    let mut parser = Parser { tokens, i: 0 };
    let value = parser.expr()?;
    if let Some(t) = parser.tokens.get(parser.i) {
        return Err(match t.tok {
            Tok::Close(c) => Error::new(format!("unmatched '{c}' at position {}", t.pos)),
            _ => Error::new(format!("unexpected '{}' at position {}", t.text, t.pos)),
        });
    }
    Ok(Evaluation { value, decimal_input })
}

fn tokenize(input: &str) -> Result<(Vec<Token>, bool), Error> {
    // Whitespace has no meaning anywhere, including inside numbers ("1 000" is 1000).
    let chars: Vec<(usize, char)> = input
        .chars()
        .enumerate()
        .filter(|(_, c)| !c.is_whitespace())
        .map(|(i, c)| (i + 1, c))
        .collect();
    let starts_with = |i: usize, word: &str| {
        word.chars().enumerate().all(|(k, w)| chars.get(i + k).is_some_and(|&(_, c)| c == w))
    };

    let mut tokens = Vec::new();
    let mut decimal_input = false;
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];

        if c.is_ascii_digit() || c == '.' {
            let literal: String = chars[i..]
                .iter()
                .map(|&(_, c)| c)
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            i += literal.chars().count();
            if literal.contains('.') {
                decimal_input = true;
            }
            let value = parse_number(&literal)
                .ok_or_else(|| Error::new(format!("invalid number '{literal}' at position {pos}")))?;
            tokens.push(Token { tok: Tok::Num(value), text: literal, pos });
            continue;
        }

        let (tok, len) = if starts_with(i, "sqrt") {
            (Tok::Sqrt, 4)
        } else if starts_with(i, "pi") {
            (Tok::Pi, 2)
        } else {
            let tok = match c {
                '+' => Tok::Plus,
                '-' | '−' => Tok::Minus,
                '*' | 'x' | 'X' | '×' => Tok::Mul,
                '/' | '÷' => Tok::Div,
                '^' => Tok::Pow,
                '√' => Tok::Sqrt,
                'π' => Tok::Pi,
                '(' => Tok::Open(')'),
                '{' => Tok::Open('}'),
                '[' => Tok::Open(']'),
                ')' | '}' | ']' => Tok::Close(c),
                _ => return Err(Error::new(format!("unexpected character '{c}' at position {pos}"))),
            };
            (tok, 1)
        };
        let text = chars[i..i + len].iter().map(|&(_, c)| c).collect();
        tokens.push(Token { tok, text, pos });
        i += len;
    }
    Ok((tokens, decimal_input))
}

/// Parses a decimal literal exactly; falls back to f64 if it doesn't fit in i128.
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

struct Parser {
    tokens: Vec<Token>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<Tok> {
        self.tokens.get(self.i).map(|t| t.tok)
    }

    fn expr(&mut self) -> Result<Value, Error> {
        let mut value = self.term()?;
        loop {
            match self.peek() {
                Some(Tok::Plus) => {
                    self.i += 1;
                    value = value.add(self.term()?)?;
                }
                Some(Tok::Minus) => {
                    self.i += 1;
                    value = value.sub(self.term()?)?;
                }
                _ => return Ok(value),
            }
        }
    }

    fn term(&mut self) -> Result<Value, Error> {
        let mut value = self.unary()?;
        loop {
            match self.peek() {
                Some(Tok::Mul) => {
                    self.i += 1;
                    value = value.mul(self.unary()?)?;
                }
                Some(Tok::Div) => {
                    self.i += 1;
                    value = value.div(self.unary()?)?;
                }
                // Implicit multiplication: 2π, 3√2, 2{1+1}, {1+1}{2+2}
                Some(Tok::Num(_) | Tok::Pi | Tok::Sqrt | Tok::Open(_)) => {
                    value = value.mul(self.unary()?)?;
                }
                _ => return Ok(value),
            }
        }
    }

    fn unary(&mut self) -> Result<Value, Error> {
        match self.peek() {
            Some(Tok::Plus) => {
                self.i += 1;
                self.unary()
            }
            Some(Tok::Minus) => {
                self.i += 1;
                self.unary()?.neg()
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Value, Error> {
        let base = self.root()?;
        if self.peek() == Some(Tok::Pow) {
            self.i += 1;
            return base.pow(self.unary()?);
        }
        Ok(base)
    }

    fn root(&mut self) -> Result<Value, Error> {
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
        if negate { operand.neg()?.sqrt() } else { operand.sqrt() }
    }

    fn primary(&mut self) -> Result<Value, Error> {
        let Some(token) = self.tokens.get(self.i) else {
            return Err(Error::new("unexpected end of expression"));
        };
        let (tok, pos) = (token.tok, token.pos);
        match tok {
            Tok::Num(value) => {
                self.i += 1;
                Ok(value)
            }
            Tok::Pi => {
                self.i += 1;
                Ok(Value::Approx(std::f64::consts::PI))
            }
            Tok::Open(close) => {
                self.i += 1;
                let value = self.expr()?;
                match self.tokens.get(self.i) {
                    Some(t) if t.tok == Tok::Close(close) => {
                        self.i += 1;
                        Ok(value)
                    }
                    Some(t) => Err(Error::new(format!(
                        "expected '{close}' to match bracket at position {pos}, found '{}' at position {}",
                        t.text, t.pos
                    ))),
                    None => Err(Error::new(format!("missing '{close}' for bracket at position {pos}"))),
                }
            }
            _ => Err(Error::new(format!("unexpected '{}' at position {pos}", token.text))),
        }
    }
}
