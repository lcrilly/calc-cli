//! Named functions. Every function takes one list (a single number is a list of one).

use crate::error::Error;
use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Func {
    // Aggregates: list → one number.
    Sum,
    Product,
    Mean,
    Median,
    Min,
    Max,
    Stdev,
    // Element by element.
    Abs,
    Floor,
    Ceil,
    Round,
    Ln,
    Log,
    Exp,
    Sin,
    Cos,
    Tan,
    Factorial,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Aggregate,
    Elementwise,
}

/// Names are matched case-insensitively and the longest wins, so `product` isn't `prod` × ….
const NAMES: &[(&str, Func)] = &[
    ("average", Func::Mean),
    ("product", Func::Product),
    ("median", Func::Median),
    ("floor", Func::Floor),
    ("round", Func::Round),
    ("stdev", Func::Stdev),
    ("ceil", Func::Ceil),
    ("mean", Func::Mean),
    ("prod", Func::Product),
    ("abs", Func::Abs),
    ("avg", Func::Mean),
    ("cos", Func::Cos),
    ("exp", Func::Exp),
    ("log", Func::Log),
    ("max", Func::Max),
    ("min", Func::Min),
    ("sin", Func::Sin),
    ("sum", Func::Sum),
    ("tan", Func::Tan),
    ("factorial", Func::Factorial),
    ("fact", Func::Factorial),
    ("ln", Func::Ln),
];

/// Single-character spellings: ∑ (macOS Option-W), Greek Σ, ∏ (Option-Shift-P), Greek Π.
pub fn symbol(c: char) -> Option<Func> {
    match c {
        '∑' | 'Σ' => Some(Func::Sum),
        '∏' | 'Π' => Some(Func::Product),
        _ => None,
    }
}

impl Func {
    /// The function whose name starts `word` (in lowercase), and the length of that name.
    pub fn prefix_of(word: &str) -> Option<(Func, usize)> {
        NAMES
            .iter()
            .filter(|(name, _)| word.starts_with(name))
            .max_by_key(|(name, _)| name.len())
            .map(|&(name, f)| (f, name.len()))
    }

    pub fn kind(self) -> Kind {
        use Func::*;
        match self {
            Sum | Product | Mean | Median | Min | Max | Stdev => Kind::Aggregate,
            Abs | Floor | Ceil | Round | Ln | Log | Exp | Sin | Cos | Tan | Factorial => Kind::Elementwise,
        }
    }

    pub fn apply(self, args: Vec<Value>) -> Result<Vec<Value>, Error> {
        use Func::*;
        let one = |v: Value| Ok(vec![v]);
        match self {
            Sum => one(sum(&args)?),
            Product => one(args.iter().try_fold(Value::from_int(1), |acc, &v| acc.mul(v))?),
            Mean => one(mean(&args)?),
            Median => {
                let sorted = sorted(args);
                let n = sorted.len();
                if n == 0 {
                    return Err(Error::new("median of an empty list"));
                }
                if n % 2 == 1 {
                    one(sorted[n / 2])
                } else {
                    one(sorted[n / 2 - 1].add(sorted[n / 2])?.div(Value::from_int(2))?)
                }
            }
            Min => one(args.into_iter().min_by(|a, b| a.total_cmp(*b)).ok_or_else(|| Error::new("min of an empty list"))?),
            Max => one(args.into_iter().max_by(|a, b| a.total_cmp(*b)).ok_or_else(|| Error::new("max of an empty list"))?),
            Stdev => {
                // Sample standard deviation (n − 1), as spreadsheets' STDEV.
                if args.len() < 2 {
                    return Err(Error::new("stdev needs at least two numbers"));
                }
                let m = mean(&args)?;
                let squares = args.iter().map(|&v| v.sub(m)?.pow(Value::from_int(2))).collect::<Result<Vec<_>, _>>()?;
                one(sum(&squares)?.div(Value::from_int(args.len() as i128 - 1))?.sqrt()?)
            }
            Abs => Ok(args.into_iter().map(Value::abs).collect()),
            Floor => Ok(args.into_iter().map(Value::floor).collect()),
            Ceil => Ok(args.into_iter().map(Value::ceil).collect()),
            Round => Ok(args.into_iter().map(Value::round).collect()),
            Ln => float(args, f64::ln),
            Log => float(args, f64::log10),
            Exp => float(args, f64::exp),
            Sin => float(args, f64::sin),
            Cos => float(args, f64::cos),
            Tan => float(args, f64::tan),
            Factorial => args.into_iter().map(Value::factorial).collect(),
        }
    }
}

fn sum(args: &[Value]) -> Result<Value, Error> {
    args.iter().try_fold(Value::from_int(0), |acc, &v| acc.add(v))
}

fn mean(args: &[Value]) -> Result<Value, Error> {
    if args.is_empty() {
        return Err(Error::new("mean of an empty list"));
    }
    sum(args)?.div(Value::from_int(args.len() as i128))
}

fn sorted(mut args: Vec<Value>) -> Vec<Value> {
    args.sort_by(|a, b| a.total_cmp(*b));
    args
}

fn float(args: Vec<Value>, f: fn(f64) -> f64) -> Result<Vec<Value>, Error> {
    args.into_iter().map(|v| crate::value::approx(f(v.to_f64()))).collect()
}
