//! The syntax tree, and evaluating it. Every value is a list; a single number is a
//! list of one, and arithmetic applies element by element.

use crate::error::{Error, Span};
use crate::functions::{Func, Kind};
use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
}

impl Op {
    fn function(self) -> fn(Value, Value) -> Result<Value, Error> {
        match self {
            Op::Add => Value::add,
            Op::Sub => Value::sub,
            Op::Mul => Value::mul,
            Op::Div => Value::div,
            Op::Rem => Value::rem,
            Op::Pow => Value::pow,
        }
    }
}

#[derive(Debug)]
pub enum Expr {
    Num(Value),
    Stdin,
    Neg(Box<Node>),
    Sqrt(Box<Node>),
    Binary(Op, Box<Node>, Box<Node>),
    Call(Func, Box<Node>),
    List(Vec<Node>),
    Range(Box<Node>, Box<Node>),
}

/// An expression and the part of the input it came from.
#[derive(Debug)]
pub struct Node {
    pub expr: Expr,
    pub span: Span,
}

impl Node {
    pub fn new(expr: Expr, span: Span) -> Node {
        Node { expr, span }
    }

    pub fn binary(op: Op, a: Node, b: Node) -> Node {
        let span = a.span.start..b.span.end;
        Node::new(Expr::Binary(op, Box::new(a), Box::new(b)), span)
    }

    pub fn range(from: Node, to: Node) -> Node {
        let span = from.span.start..to.span.end;
        Node::new(Expr::Range(Box::new(from), Box::new(to)), span)
    }

    /// True when `pred` holds anywhere in the tree.
    pub fn any(&self, pred: &impl Fn(&Expr) -> bool) -> bool {
        pred(&self.expr)
            || match &self.expr {
                Expr::Num(_) | Expr::Stdin => false,
                Expr::Neg(a) | Expr::Sqrt(a) | Expr::Call(_, a) => a.any(pred),
                Expr::Binary(_, a, b) | Expr::Range(a, b) => a.any(pred) || b.any(pred),
                Expr::List(items) => items.iter().any(|n| n.any(pred)),
            }
    }

    /// True when nothing combines or reorders elements, so a list can be evaluated
    /// a piece at a time with the same result.
    pub fn is_elementwise(&self) -> bool {
        !self.any(&|e| match e {
            Expr::List(_) | Expr::Range(..) => true,
            Expr::Call(f, _) => f.kind() == Kind::Aggregate,
            _ => false,
        })
    }

    pub fn eval(&self, stdin: Option<&[Value]>) -> Result<Vec<Value>, Error> {
        self.eval_here(stdin).map_err(|e| e.at(self.span.clone()))
    }

    fn eval_here(&self, stdin: Option<&[Value]>) -> Result<Vec<Value>, Error> {
        match &self.expr {
            Expr::Num(v) => Ok(vec![*v]),
            Expr::Stdin => stdin.map(<[Value]>::to_vec).ok_or_else(|| Error::new("'@' needs input on stdin")),
            Expr::Neg(a) => Ok(a.eval(stdin)?.into_iter().map(Value::neg).collect()),
            Expr::Sqrt(a) => a.eval(stdin)?.into_iter().map(Value::sqrt).collect(),
            Expr::Binary(op, a, b) => zip(a.eval(stdin)?, b.eval(stdin)?, op.function()),
            Expr::Call(f, args) => f.apply(args.eval(stdin)?),
            Expr::List(items) => {
                let mut values = Vec::new();
                for item in items {
                    values.extend(item.eval(stdin)?);
                }
                Ok(values)
            }
            Expr::Range(from, to) => range(from.eval(stdin)?, to.eval(stdin)?),
        }
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

/// Ranges stop well short of exhausting memory.
const MAX_RANGE: usize = 10_000_000;

fn range(from: Vec<Value>, to: Vec<Value>) -> Result<Vec<Value>, Error> {
    let ([from], [to]) = (from.as_slice(), to.as_slice()) else {
        return Err(Error::new("a range needs a single number at each end"));
    };
    let (from, to) = (*from, *to);
    let ascending = from.total_cmp(to).is_le();
    let step = Value::from_int(if ascending { 1 } else { -1 });
    let mut out = Vec::new();
    let mut v = from;
    while if ascending { v.total_cmp(to).is_le() } else { v.total_cmp(to).is_ge() } {
        if out.len() == MAX_RANGE {
            return Err(Error::new(format!("range is longer than {MAX_RANGE} numbers")));
        }
        out.push(v);
        v = v.add(step)?;
    }
    Ok(out)
}
