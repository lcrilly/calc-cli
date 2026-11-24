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
    /// The most frequent value, or every value that ties for most frequent.
    Mode,
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
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
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
    ("asin", Func::Asin),
    ("acos", Func::Acos),
    ("atan", Func::Atan),
    ("sinh", Func::Sinh),
    ("cosh", Func::Cosh),
    ("tanh", Func::Tanh),
    ("mode", Func::Mode),
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
            Sum | Product | Mean | Median | Mode | Min | Max | Stdev => Kind::Aggregate,
            Abs | Floor | Ceil | Round | Ln | Log | Exp | Sin | Cos | Tan | Asin | Acos | Atan | Sinh | Cosh | Tanh | Factorial => {
                Kind::Elementwise
            }
        }
    }

    pub fn apply(self, args: Vec<Value>) -> Result<Vec<Value>, Error> {
        use Func::*;
        let one = |v: Value| Ok(vec![v]);
        match self {
            Sum => one(sum(&args)?),
            Product => one(args.into_iter().try_fold(Value::from_int(1), Value::mul)?),
            Mean => one(mean(&args)?),
            Median => {
                let sorted = sorted(args);
                let n = sorted.len();
                if n == 0 {
                    return Err(Error::new("median of an empty list"));
                }
                if n % 2 == 1 {
                    one(sorted[n / 2].clone())
                } else {
                    one(sorted[n / 2 - 1].clone().add(sorted[n / 2].clone())?.div(Value::from_int(2))?)
                }
            }
            Mode => {
                let sorted = sorted(args);
                // Runs of equal values, in ascending order; keep every run of the greatest length.
                let runs: Vec<&[Value]> = sorted.chunk_by(|a, b| a.total_cmp(b).is_eq()).collect();
                let most = runs.iter().map(|run| run.len()).max().ok_or_else(|| Error::new("mode of an empty list"))?;
                Ok(runs.into_iter().filter(|run| run.len() == most).map(|run| run[0].clone()).collect())
            }
            Min => one(args.into_iter().min_by(Value::total_cmp).ok_or_else(|| Error::new("min of an empty list"))?),
            Max => one(args.into_iter().max_by(Value::total_cmp).ok_or_else(|| Error::new("max of an empty list"))?),
            Stdev => {
                // Sample standard deviation (n − 1), as spreadsheets' STDEV.
                if args.len() < 2 {
                    return Err(Error::new("stdev needs at least two numbers"));
                }
                let m = mean(&args)?;
                let n = args.len() as i64;
                let squares = args.into_iter().map(|v| v.sub(m.clone())?.pow(Value::from_int(2))).collect::<Result<Vec<_>, _>>()?;
                one(sum(&squares)?.div(Value::from_int(n - 1))?.sqrt()?)
            }
            Abs => Ok(args.into_iter().map(Value::abs).collect()),
            Floor => Ok(args.into_iter().map(Value::floor).collect()),
            Ceil => Ok(args.into_iter().map(Value::ceil).collect()),
            Round => Ok(args.into_iter().map(Value::round).collect()),
            Ln => each(args, Value::ln),
            Log => each(args, Value::log10),
            Exp => each(args, Value::exp),
            Sin => each(args, Value::sin),
            Cos => each(args, Value::cos),
            Tan => each(args, Value::tan),
            Asin => each(args, Value::asin),
            Acos => each(args, Value::acos),
            Atan => each(args, Value::atan),
            Sinh => each(args, Value::sinh),
            Cosh => each(args, Value::cosh),
            Tanh => each(args, Value::tanh),
            Factorial => args.into_iter().map(Value::factorial).collect(),
        }
    }
}

fn sum(args: &[Value]) -> Result<Value, Error> {
    args.iter().try_fold(Value::from_int(0), |acc, v| acc.add(v.clone()))
}

fn mean(args: &[Value]) -> Result<Value, Error> {
    if args.is_empty() {
        return Err(Error::new("mean of an empty list"));
    }
    sum(args)?.div(Value::from_int(args.len() as i64))
}

fn sorted(mut args: Vec<Value>) -> Vec<Value> {
    args.sort_by(Value::total_cmp);
    args
}

fn each(args: Vec<Value>, f: fn(Value) -> Result<Value, Error>) -> Result<Vec<Value>, Error> {
    args.into_iter().map(f).collect()
}
