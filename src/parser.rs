//! Parses tokens into a syntax tree, which is checked once and can then be
//! evaluated any number of times.
//!
//! Grammar (lowest to highest precedence):
//!   list    := element (SEP element)* SEP?                -- SEP is ',' or a separating space
//!   element := expr ('..' expr)?
//!   expr    := term (('+' | '-') term)*
//!   term    := unary (('×' | '÷' | '%') unary | unary)*   -- adjacency is implicit ×
//!   unary   := ('+' | '-') unary | power
//!   power   := root (SUP | '!')* ('^' unary)?             -- right-associative
//!   root    := '√' ('+' | '-')* root | primary
//!   primary := atom '!'*                                  -- factorial binds tightest
//!   atom    := number | 'π' | '@' | '(' list ')' | call
//!   call    := FUNC '(' list ')' | FUNC list              -- without a bracket, takes the rest
//!
//! A closing bracket with no opening bracket implies one at the very start of the
//! expression: `2+4+8}/2` is `{2+4+8}/2`.

use crate::error::Error;
use crate::eval::{Expr, Node, Op};
use crate::functions::Func;
use crate::lexer::{self, Syntax, Tok, Token};
use crate::value::Value;

/// A tokenised expression, which may still be missing stdin.
pub struct Expression {
    source: String,
    tokens: Vec<Token>,
}

/// A parsed expression, ready to evaluate.
pub struct Program {
    source: String,
    root: Node,
}

/// Evaluates an expression that doesn't read stdin (a line of input).
pub fn evaluate(input: &str, syntax: &Syntax) -> Result<Vec<Value>, Error> {
    let expression = Expression::parse(input, syntax)?;
    if expression.is_empty() {
        return Err(Error::new("empty expression"));
    }
    if let Some(t) = expression.tokens.iter().find(|t| t.tok == Tok::Stdin) {
        return Err(Error::new("'@' can only be used in arguments").at(t.span.clone()));
    }
    expression.compile()?.evaluate(None)
}

impl Expression {
    pub fn parse(input: &str, syntax: &Syntax) -> Result<Expression, Error> {
        Ok(Expression { source: input.to_string(), tokens: lexer::tokenize(input, syntax)? })
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
        self.tokens.first().is_some_and(|t| matches!(t.tok, Tok::Plus | Tok::Mul | Tok::Div | Tok::Mod | Tok::Pow | Tok::Sup(_) | Tok::Bang))
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
        let end = self.source.chars().count();
        if self.tokens.is_empty() || self.awaits_operand() {
            self.tokens.push(Token::synthetic(Tok::Stdin, end));
        }
        if self.is_continuation() {
            self.tokens.insert(0, Token::synthetic(Tok::Stdin, 0));
        }
    }

    pub fn compile(mut self) -> Result<Program, Error> {
        infer_opening_brackets(&mut self.tokens);
        let mut parser = Parser { tokens: &self.tokens, i: 0, source: &self.source };
        let root = parser.list()?;
        if let Some(t) = parser.token() {
            return Err(match t.tok {
                Tok::Close(c) => Error::new(format!("unmatched '{c}'")).at(t.span.clone()),
                _ => parser.unexpected(t),
            });
        }
        Ok(Program { source: self.source, root })
    }
}

impl Program {
    /// The expression as written, for showing where an error is.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// True when stdin can be processed a line at a time with the same result as
    /// reading it all as one list.
    pub fn streams(&self) -> bool {
        self.root.is_elementwise()
    }

    pub fn evaluate(&self, stdin: Option<&[Value]>) -> Result<Vec<Value>, Error> {
        self.root.eval(stdin)
    }
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
            Tok::Close(c) if open.is_empty() => inferred.push(c),
            Tok::Close(c) if open.last() == Some(&c) => {
                open.pop();
            }
            Tok::Close(_) => break,
            _ => {}
        }
    }
    // The last unmatched closer is outermost, so its opener goes first.
    tokens.splice(0..0, inferred.into_iter().rev().map(|c| Token::synthetic(Tok::Open(c), 0)));
}

struct Parser<'a> {
    tokens: &'a [Token],
    i: usize,
    source: &'a str,
}

impl<'a> Parser<'a> {
    fn token(&self) -> Option<&'a Token> {
        self.tokens.get(self.i)
    }

    fn peek(&self) -> Option<Tok> {
        self.token().map(|t| t.tok)
    }

    /// Where the next token starts, or the end of the input.
    fn here(&self) -> usize {
        match self.token() {
            Some(t) => t.span.start,
            None => self.tokens.last().map_or(0, |t| t.span.end),
        }
    }

    fn at_list_end(&self) -> bool {
        matches!(self.peek(), None | Some(Tok::Close(_)))
    }

    fn text(&self, t: &Token) -> String {
        self.source.chars().skip(t.span.start).take(t.span.len()).collect()
    }

    fn unexpected(&self, t: &Token) -> Error {
        Error::new(format!("unexpected '{}'", self.text(t))).at(t.span.clone())
    }

    fn list(&mut self) -> Result<Node, Error> {
        let start = self.here();
        let mut items = Vec::new();
        if !self.at_list_end() {
            loop {
                items.push(self.element()?);
                if self.peek() != Some(Tok::Sep) {
                    break;
                }
                self.i += 1;
                if self.at_list_end() {
                    break; // trailing comma
                }
            }
        }
        if items.len() == 1 {
            return Ok(items.remove(0));
        }
        let end = items.last().map_or(start, |n| n.span.end);
        Ok(Node::new(Expr::List(items), start..end))
    }

    fn element(&mut self) -> Result<Node, Error> {
        let from = self.expr()?;
        if self.peek() != Some(Tok::Range) {
            return Ok(from);
        }
        self.i += 1;
        Ok(Node::range(from, self.expr()?))
    }

    fn expr(&mut self) -> Result<Node, Error> {
        let mut node = self.term()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => Op::Add,
                Some(Tok::Minus) => Op::Sub,
                _ => return Ok(node),
            };
            self.i += 1;
            node = Node::binary(op, node, self.term()?);
        }
    }

    fn term(&mut self) -> Result<Node, Error> {
        let mut node = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Mul) => Op::Mul,
                Some(Tok::Div) => Op::Div,
                Some(Tok::Mod) => Op::Rem,
                // Implicit multiplication: 2π, 3√2, 2{1+1}, {1+1}{2+2}, 2sum{1 2}
                Some(tok) if tok.starts_operand() => {
                    node = Node::binary(Op::Mul, node, self.unary()?);
                    continue;
                }
                _ => return Ok(node),
            };
            self.i += 1;
            node = Node::binary(op, node, self.unary()?);
        }
    }

    fn unary(&mut self) -> Result<Node, Error> {
        match self.token() {
            Some(t) if t.tok == Tok::Plus => {
                self.i += 1;
                self.unary()
            }
            Some(t) if t.tok == Tok::Minus => {
                self.i += 1;
                let operand = self.unary()?;
                let span = t.span.start..operand.span.end;
                Ok(Node::new(Expr::Neg(Box::new(operand)), span))
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Node, Error> {
        let mut base = self.root()?;
        // ² and ³ bind like ^2 and ^3: -2² is -4, √4² is (√4)², 2²³ is (2²)³.
        // A factorial can follow them too: 2²! is (2²)!.
        while let Some(t) = self.token() {
            base = match t.tok {
                Tok::Sup(n) => Node::binary(Op::Pow, base, Node::new(Expr::Num(Value::from_int(n)), t.span.clone())),
                Tok::Bang => factorial(base, t),
                _ => break,
            };
            self.i += 1;
        }
        if self.peek() == Some(Tok::Pow) {
            self.i += 1;
            return Ok(Node::binary(Op::Pow, base, self.unary()?));
        }
        Ok(base)
    }

    fn root(&mut self) -> Result<Node, Error> {
        let Some(sqrt) = self.token().filter(|t| t.tok == Tok::Sqrt) else {
            return self.primary();
        };
        self.i += 1;
        let mut negate = false;
        while let Some(sign @ (Tok::Plus | Tok::Minus)) = self.peek() {
            negate ^= sign == Tok::Minus;
            self.i += 1;
        }
        let mut operand = self.root()?;
        if negate {
            let span = operand.span.clone();
            operand = Node::new(Expr::Neg(Box::new(operand)), span);
        }
        let span = sqrt.span.start..operand.span.end;
        Ok(Node::new(Expr::Sqrt(Box::new(operand)), span))
    }

    /// An atom and any factorials after it, so `√4!` is √(4!) and `-3!` is -(3!).
    fn primary(&mut self) -> Result<Node, Error> {
        let mut node = self.atom()?;
        while let Some(t) = self.token().filter(|t| t.tok == Tok::Bang) {
            self.i += 1;
            node = factorial(node, t);
        }
        Ok(node)
    }

    fn atom(&mut self) -> Result<Node, Error> {
        let Some(t) = self.token() else {
            let end = self.here();
            return Err(Error::new("unexpected end of expression").at(end..end));
        };
        self.i += 1;
        let span = t.span.clone();
        match t.tok {
            Tok::Num(value) => Ok(Node::new(Expr::Num(value), span)),
            Tok::Pi => Ok(Node::new(Expr::Num(Value::Approx(std::f64::consts::PI)), span)),
            Tok::Stdin => Ok(Node::new(Expr::Stdin, span)),
            Tok::Open(close) => {
                // Empty brackets are only meaningful as a function's list: sum{}.
                if let Some(next) = self.token().filter(|n| matches!(n.tok, Tok::Close(_))) {
                    return Err(self.unexpected(next));
                }
                self.bracketed(t, close)
            }
            Tok::Func(f) => {
                let args = match self.token() {
                    Some(open) if !open.space_before && let Tok::Open(close) = open.tok => {
                        self.i += 1;
                        self.bracketed(open, close)?
                    }
                    _ if self.at_list_end() => {
                        return Err(Error::new(format!("'{}' needs numbers after it", self.text(t))).at(span));
                    }
                    _ => self.list()?,
                };
                let span = span.start..args.span.end;
                Ok(Node::new(Expr::Call(f, Box::new(args)), span))
            }
            _ => Err(self.unexpected(t)),
        }
    }

    /// Parses a list up to `close`; the opening bracket `open` is already consumed.
    fn bracketed(&mut self, open: &Token, close: char) -> Result<Node, Error> {
        let mut inner = self.list()?;
        match self.token() {
            Some(t) if t.tok == Tok::Close(close) => {
                self.i += 1;
                inner.span = open.span.start..t.span.end;
                Ok(inner)
            }
            Some(t) => Err(Error::new(format!(
                "expected '{close}' to match the bracket at position {}",
                open.span.start + 1
            ))
            .at(t.span.clone())),
            None => Err(Error::new(format!("missing '{close}'")).at(open.span.clone())),
        }
    }
}

fn factorial(operand: Node, bang: &Token) -> Node {
    let span = operand.span.start..bang.span.end;
    Node::new(Expr::Call(Func::Factorial, Box::new(operand)), span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{Precision, format};
    use crate::locale::NumberFormat;

    fn eval_with(input: &str, syntax: &Syntax) -> String {
        match evaluate(input, syntax) {
            Ok(values) => values.iter().map(|&v| format(v, Precision::Default).to_string()).collect::<Vec<_>>().join(" "),
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
        assert_eq!(eval("}"), "error: unexpected '}'");
        assert_eq!(eval("{1+2"), "error: missing '}'");
    }

    #[test]
    fn errors_point_at_the_input() {
        let shown = |input: &str| match evaluate(input, &Syntax::default()) {
            Err(e) => e.in_source(input).to_string(),
            Ok(_) => panic!("{input} should fail"),
        };
        assert_eq!(shown("2x{3+4"), "missing '}'\n  2x{3+4\n    ^");
        assert_eq!(shown("1+2/0"), "division by zero\n  1+2/0\n    ^^^");
        assert_eq!(shown("1 q"), "unexpected character 'q'\n  1 q\n    ^");
        assert!(shown("1,234").ends_with("\n  1,234\n  ^^^^^"));
        assert!(shown("{1 2}+{1 2 3}").ends_with("^^^^^^^^^^^^^"));
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
        assert_eq!(eval("3,5"), "3 5");
        let comma = Syntax { local_decimal: ',', ..Syntax::default() };
        assert!(eval_with("3,5", &comma).contains("'.' as the decimal mark"));
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
        assert_eq!(eval("product 1 2 3 4"), "24");
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
        assert_eq!(eval("stdev 2 4 4 4 5 5 7 9"), "2.13809");
        assert_eq!(eval("2xmax{1 5}"), "10");
        for removed in ["count 1 2", "sort 3 1 2", "cumsum 1 2", "cumprod 1 2"] {
            assert!(eval(removed).starts_with("error"), "{removed}");
        }
        assert_eq!(eval("abs -3 + 1"), "2");
        assert_eq!(eval("round{2.5 -2.5 2.4}"), "3 -3 2");
        assert_eq!(eval("floor 7/2"), "3");
        assert_eq!(eval("ceil 7/2"), "4");
        assert_eq!(eval("log 1000"), "3");
        assert_eq!(eval("sum{}"), "0");
        assert!(eval("sum").contains("needs numbers after it"));
    }

    #[test]
    fn mode() {
        assert_eq!(eval("mode 1 2 2 3"), "2");
        assert_eq!(eval("mode 3 1 3 1 2"), "1 3"); // ties, in ascending order
        assert_eq!(eval("mode 1 2 3"), "1 2 3");
        assert_eq!(eval("mode 5"), "5");
        assert_eq!(eval("mode 0.5 1/2 2"), "0.5");
        assert_eq!(eval("mode{}"), "error: mode of an empty list");
        assert_eq!(eval("2xmode{1 4 4}"), "8");
    }

    #[test]
    fn inverse_and_hyperbolic_trig() {
        assert_eq!(eval("asin 1"), "1.5708");
        assert_eq!(eval("acos{1 0}"), "0 1.5708");
        assert_eq!(eval("atan 1"), "0.7854");
        same("4atan 1", "π");
        assert_eq!(eval("acos 2"), "error: undefined result");
        assert_eq!(eval("sinh 1"), "1.1752");
        assert_eq!(eval("cosh 0"), "1");
        assert_eq!(eval("tanh 1"), "0.76159");
        same("sin asin 0.5", "0.5");
        assert_eq!(eval("2sinh{0}"), "0");
    }

    #[test]
    fn ranges() {
        assert_eq!(eval("1..5"), "1 2 3 4 5");
        assert_eq!(eval("3..1"), "3 2 1");
        assert_eq!(eval("sum 1..100"), "5050");
        assert_eq!(eval("1..2x3"), "1 2 3 4 5 6");
    }

    #[test]
    fn superscript_powers() {
        same("3²", "9");
        same("2³", "8");
        same("-2²", "-2^2");
        same("√4²", "√4^2");
        same("{1+2}²", "9");
        same("2²3", "12");
        same("2²³", "64");
        same("2²^3", "4^3");
        assert_eq!(eval("{1 2 3}²"), "1 4 9");
        assert_eq!(eval("2² 3"), "4 3");
        assert_eq!(eval("π²"), "9.8696");
        assert!(eval("²").starts_with("error"));
        assert_eq!(eval("1…5"), "1 2 3 4 5");
        assert_eq!(eval("1...5"), "1 2 3 4 5");
        assert_eq!(eval("sum 1 … 4"), "10");
    }

    #[test]
    fn factorials() {
        assert_eq!(eval("5!"), "120");
        assert_eq!(eval("0!"), "1");
        same("-3!", "-6");
        same("2^3!", "64");
        same("3!²", "36");
        same("2²!", "24");
        same("√4!", "√24");
        same("2x3!", "12");
        same("3!4", "24");
        same("{3!}!", "720");
        assert_eq!(eval("{1 2 3 4}!"), "1 2 6 24");
        assert_eq!(eval("5! 3"), "120 3");
        assert_eq!(eval("fact 1..5"), "1 2 6 24 120");
        assert_eq!(eval("factorial(5)"), "120");
        assert_eq!(eval("FACT{5}x2"), "240");
        assert_eq!(eval("33!"), "8683317618811886495518194401280000000");
        assert_eq!(eval("34!"), "2.95233e38");
        assert_eq!(eval("170!"), "7.25742e306");
        assert_eq!(eval("171!"), "error: result out of range");
        assert!(eval("2.5!").contains("whole number"));
        assert!(eval("{-1}!").contains("whole number"));
        assert!(eval("π!").contains("whole number"));
        assert!(eval("3!!").contains("'!!' isn't supported"));
        assert!(eval("!").starts_with("error"));
    }

    #[test]
    fn decimals_print_as_themselves() {
        assert_eq!(eval("2.5x2"), "5");
        assert_eq!(eval("0.5 3"), "0.5 3");
        assert_eq!(eval("1.50"), "1.5");
    }

    #[test]
    fn streaming_is_decided_from_the_tree() {
        let program = |s: &str| Expression::parse(s, &Syntax::default()).unwrap().compile().unwrap();
        assert!(program("2x{3}+abs 1").streams());
        assert!(!program("sum 1").streams());
        assert!(!program("1 2").streams());
        assert!(!program("1..3").streams());
    }

    #[test]
    fn pretty_input() {
        let syntax = |example| Syntax { pretty: NumberFormat::from_example(example).ok(), ..Syntax::default() };
        let (en, de) = (syntax("1,234.5"), syntax("1.234,5"));
        assert_eq!(eval_with("3,141.6+1", &en), "3142.6");
        assert_eq!(eval_with("1,2", &en), "1 2");
        assert_eq!(eval_with("12,34,567", &syntax("12,34,567.8")), "1234567");
        assert_eq!(eval_with("3.141,6+1", &de), "3142.6");
        assert_eq!(eval_with("1,5 2,5", &de), "1.5 2.5");
        assert!(eval_with("3.14", &de).starts_with("error"));
        assert_eq!(eval("1\u{202f}234,5"), "1234 5");
    }
}
