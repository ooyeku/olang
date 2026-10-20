//! The expression engine: every elementwise column operation, fused.
//!
//! An [`Expr`] is a tree (in practice a DAG: subtrees are `Arc`-shared)
//! over columns and scalars. [`Plan::new`] compiles one or several
//! roots into a list of steps — common subexpressions once, a column
//! met twice read once, an Int widened once — and [`Plan::eval`] runs
//! the steps over the rows [`CHUNK`] at a time, so every intermediate
//! lives in a buffer that stays in L1 and only the outputs touch main
//! memory. [`Plan::reduce`] folds a root straight into a sum, mean,
//! minimum, maximum or count, with no output column at all.
//!
//! What fuses: arithmetic (`+ - * / %`, `pow`, negation), comparisons,
//! Kleene `& | ^ !`, every math function, `where`, `coalesce`, `clip`,
//! `between`, the null masks and the numeric casts. Nulls fuse: each
//! operand's validity is read as 64-bit words and combined word by word
//! (an AND for arithmetic, an OR for `coalesce`, Kleene logic for the
//! Booleans), and a column with no nulls costs nothing. Int fuses:
//! integer arithmetic stays Int and checked, and an Int operand of a
//! Float operation is widened once per chunk.
//!
//! The semantics are the eager kernels', value for value and error for
//! error: a valid position computes exactly the float operations the
//! eager kernel does, nulls propagate, a division by zero or an integer
//! overflow at a valid position raises the eager kernel's error (and
//! one under a null does not), and the domain-restricted math functions
//! raise `ods.map`'s messages. A parallel evaluation cuts the rows into
//! pieces of whole chunks, so it computes what the sequential one does,
//! bit for bit, and reports the first failing piece's error.

use crate::bitmap::Bitmap;
use crate::{DType, OdsError, ParClass, Scalar, Series, StrCol};
#[cfg(feature = "parallel")]
use rayon::prelude::*;
use std::collections::HashMap;
use std::mem::MaybeUninit;
use std::sync::Arc;

type Result<T> = std::result::Result<T, OdsError>;

/// Rows evaluated per pass over the steps: one buffer per live step
/// stays in L1.
pub const CHUNK: usize = 2048;
const WORDS: usize = CHUNK / 64;
/// Rows a parallel task takes: many chunks, one hand-off — a few tasks
/// a worker, of 16 to 128 chunks.
#[cfg(feature = "parallel")]
fn piece_rows(n: usize) -> usize {
    let workers = rayon::current_num_threads().max(1);
    (n / (workers * 4)).clamp(CHUNK * 16, CHUNK * 128) / CHUNK * CHUNK
}
/// A parallel reduction's pieces, summed as the sequential pass sums
/// them and added in order after (the eager `sum`'s piece size).
#[cfg(feature = "parallel")]
const REDUCE_PIECE: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    /// True division: Int / Int is Float, as the eager kernels divide.
    Div,
    /// The remainder, with the sign of the dividend (the language's `%`).
    Mod,
    /// `x ^ p` as a Float.
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// Kleene: `false & null` is false.
    And,
    /// Kleene: `true | null` is true.
    Or,
    Xor,
}

impl From<crate::ArithOp> for BinOp {
    fn from(op: crate::ArithOp) -> BinOp {
        match op {
            crate::ArithOp::Add => BinOp::Add,
            crate::ArithOp::Sub => BinOp::Sub,
            crate::ArithOp::Mul => BinOp::Mul,
            crate::ArithOp::Div => BinOp::Div,
        }
    }
}

impl From<crate::CmpOp> for BinOp {
    fn from(op: crate::CmpOp) -> BinOp {
        match op {
            crate::CmpOp::Eq => BinOp::Eq,
            crate::CmpOp::Ne => BinOp::Ne,
            crate::CmpOp::Lt => BinOp::Lt,
            crate::CmpOp::Le => BinOp::Le,
            crate::CmpOp::Gt => BinOp::Gt,
            crate::CmpOp::Ge => BinOp::Ge,
        }
    }
}

impl BinOp {
    fn word(self) -> &'static str {
        match self {
            BinOp::Add => "addition",
            BinOp::Sub => "subtraction",
            BinOp::Mul => "multiplication",
            BinOp::Div => "division",
            BinOp::Mod => "modulo",
            BinOp::Pow => "pow",
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => "comparison",
            BinOp::And => "&",
            BinOp::Or => "|",
            BinOp::Xor => "^",
        }
    }

    fn is_cmp(self) -> bool {
        matches!(
            self,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        )
    }

    fn is_logic(self) -> bool {
        matches!(self, BinOp::And | BinOp::Or | BinOp::Xor)
    }
}

/// The elementwise math functions. Each is the std `f64` method the
/// scalar `math.*` function calls, so a column computes what a loop over
/// `math.*` would, bit for bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Math {
    Abs,
    Sign,
    Exp,
    Exp2,
    Expm1,
    Ln,
    Ln1p,
    Log2,
    Log10,
    Sqrt,
    Cbrt,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    /// `1 / (1 + e^-x)`, computed without overflow for large `|x|`.
    Sigmoid,
    /// `ln(p / (1 - p))`, for `p` strictly between 0 and 1.
    Logit,
    Floor,
    Ceil,
    Trunc,
    Fract,
    Degrees,
    Radians,
}

impl Math {
    /// Every function by its name, in the order errors list them.
    pub const ALL: &'static [(&'static str, Math)] = &[
        ("abs", Math::Abs),
        ("sign", Math::Sign),
        ("exp", Math::Exp),
        ("exp2", Math::Exp2),
        ("expm1", Math::Expm1),
        ("ln", Math::Ln),
        ("ln1p", Math::Ln1p),
        ("log2", Math::Log2),
        ("log10", Math::Log10),
        ("sqrt", Math::Sqrt),
        ("cbrt", Math::Cbrt),
        ("sin", Math::Sin),
        ("cos", Math::Cos),
        ("tan", Math::Tan),
        ("asin", Math::Asin),
        ("acos", Math::Acos),
        ("atan", Math::Atan),
        ("sinh", Math::Sinh),
        ("cosh", Math::Cosh),
        ("tanh", Math::Tanh),
        ("sigmoid", Math::Sigmoid),
        ("logit", Math::Logit),
        ("floor", Math::Floor),
        ("ceil", Math::Ceil),
        ("trunc", Math::Trunc),
        ("fract", Math::Fract),
        ("degrees", Math::Degrees),
        ("radians", Math::Radians),
    ];

    pub fn parse(name: &str) -> Option<Math> {
        Self::ALL.iter().find(|(n, _)| *n == name).map(|(_, m)| *m)
    }

    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(_, m)| *m == self).map(|(n, _)| *n).unwrap_or("?")
    }

    /// Integer-valued functions keep an Int column Int.
    fn keeps_int(self) -> bool {
        matches!(
            self,
            Math::Abs | Math::Sign | Math::Floor | Math::Ceil | Math::Trunc
        )
    }

    /// The function's Float kernel.
    #[inline]
    fn f(self) -> fn(f64) -> f64 {
        match self {
            Math::Abs => f64::abs,
            Math::Sign => |x: f64| {
                if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    x
                }
            },
            Math::Exp => f64::exp,
            Math::Exp2 => f64::exp2,
            Math::Expm1 => f64::exp_m1,
            Math::Ln => f64::ln,
            Math::Ln1p => f64::ln_1p,
            Math::Log2 => f64::log2,
            Math::Log10 => f64::log10,
            Math::Sqrt => f64::sqrt,
            Math::Cbrt => f64::cbrt,
            Math::Sin => f64::sin,
            Math::Cos => f64::cos,
            Math::Tan => f64::tan,
            Math::Asin => f64::asin,
            Math::Acos => f64::acos,
            Math::Atan => f64::atan,
            Math::Sinh => f64::sinh,
            Math::Cosh => f64::cosh,
            Math::Tanh => f64::tanh,
            Math::Sigmoid => sigmoid,
            Math::Logit => |p: f64| (p / (1.0 - p)).ln(),
            Math::Floor => f64::floor,
            Math::Ceil => f64::ceil,
            Math::Trunc => f64::trunc,
            Math::Fract => f64::fract,
            Math::Degrees => f64::to_degrees,
            Math::Radians => f64::to_radians,
        }
    }

    /// The error a valid out-of-domain argument raises — `ods.map`'s
    /// words, which are the scalar `math.*` functions'.
    fn domain(self) -> Option<Domain> {
        Some(match self {
            Math::Sqrt => (
                |x| x < 0.0,
                "sqrt: cannot take square root of negative number",
            ),
            Math::Ln => (|x| x <= 0.0, "ln: input must be positive"),
            Math::Log2 => (|x| x <= 0.0, "log2: input must be positive"),
            Math::Log10 => (|x| x <= 0.0, "log10: input must be positive"),
            Math::Ln1p => (|x| x <= -1.0, "ln1p: input must be greater than -1"),
            Math::Asin => (
                |x| !(-1.0..=1.0).contains(&x),
                "asin: input must be in range [-1, 1]",
            ),
            Math::Acos => (
                |x| !(-1.0..=1.0).contains(&x),
                "acos: input must be in range [-1, 1]",
            ),
            Math::Logit => (
                |x| x <= 0.0 || x >= 1.0,
                "logit: input must be strictly between 0 and 1",
            ),
            _ => return None,
        })
    }

    /// Relative cost against one arithmetic operation, for the
    /// parallel threshold.
    fn cost(self) -> usize {
        match self {
            Math::Abs
            | Math::Sign
            | Math::Floor
            | Math::Ceil
            | Math::Trunc
            | Math::Fract
            | Math::Degrees
            | Math::Radians => 1,
            Math::Sqrt => 2,
            _ => 12,
        }
    }
}

/// A restricted domain: the test of a bad argument, and the error.
type Domain = (fn(f64) -> bool, &'static str);

#[inline]
fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

/// Which ends of `between` are inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Closed {
    Both,
    Left,
    Right,
    Neither,
}

impl Closed {
    pub fn parse(name: &str) -> Option<Closed> {
        Some(match name {
            "both" => Closed::Both,
            "left" => Closed::Left,
            "right" => Closed::Right,
            "neither" => Closed::Neither,
            _ => return None,
        })
    }
}

/// One elementwise operation; its operands are the node's children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// `[a, b]`
    Bin(BinOp),
    /// `[a]`: `-a`.
    Neg,
    /// `[a]`: Kleene `!a`.
    Not,
    /// `[a]`
    Math(Math),
    /// `[a]`: rounded half away from zero to `digits` decimals (negative
    /// rounds to tens, hundreds, …). Float stays Float.
    Round(i32),
    /// `[a]`: a Bool mask with no nulls.
    IsNull,
    /// `[a]`
    NotNull,
    /// `[a]`: NaN positions of a Float column; nulls stay null.
    IsNan,
    /// `[cond, a, b]`: `a` where `cond`, else `b`; a null `cond` is null.
    Where,
    /// `[a, b, …]`: the first valid operand at each position.
    Coalesce,
    /// `[x, lo, hi]`: a null bound is open.
    Clip,
    /// `[x, lo, hi]`: a null bound is open.
    Between(Closed),
    /// `[a]`: among Float, Int and Bool. Float to Int truncates, and a
    /// value an Int cannot hold becomes null (`Series::cast`'s rule).
    Cast(DType),
    /// A calendar operation on a Date or Datetime (`[a]`, or `[a, b]`
    /// for `MonthsBetween`). The flag says the operand is a Datetime
    /// (microseconds rather than days); the planner sets it from the
    /// operand, whatever the caller wrote.
    Temporal(TOp, bool),
}

/// The calendar operations of the expression engine. Each reads the
/// operand's integer storage (days, or microseconds) and computes with
/// civil-from-days arithmetic: no table, no parsing, no branch on the
/// calendar beyond a leap year.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TOp {
    Year,
    /// 1–4.
    Quarter,
    /// 1–12.
    Month,
    /// 1–31.
    Day,
    /// ISO: 1 = Monday … 7 = Sunday.
    Weekday,
    /// 1–366.
    DayOfYear,
    /// The ISO 8601 week, 1–53.
    Week,
    Hour,
    Minute,
    Second,
    /// Saturday or Sunday.
    IsWeekend,
    /// A Datetime's day as a Date (a Date itself).
    Date,
    /// A Date's midnight as a Datetime.
    ToDatetime,
    /// The integer storage: days, or microseconds.
    Raw,
    /// An Int of days as a Date (an error past a Date's range).
    AsDate,
    /// An Int of microseconds as a Datetime.
    AsDatetime,
    /// The start of the span holding it.
    Floor(crate::temporal::Every),
    /// Calendar months later, the day clamped to the month's end.
    AddMonths(i64),
    /// `[a, b]`: whole calendar months from `b` to `a`.
    MonthsBetween,
}

impl TOp {
    pub fn name(self) -> &'static str {
        match self {
            TOp::Year => "year",
            TOp::Quarter => "quarter",
            TOp::Month => "month",
            TOp::Day => "day",
            TOp::Weekday => "weekday",
            TOp::DayOfYear => "day_of_year",
            TOp::Week => "week",
            TOp::Hour => "hour",
            TOp::Minute => "minute",
            TOp::Second => "second",
            TOp::IsWeekend => "is_weekend",
            TOp::Date => "date",
            TOp::ToDatetime => "to_datetime",
            TOp::Raw => "raw",
            TOp::AsDate => "as_date",
            TOp::AsDatetime => "as_datetime",
            TOp::Floor(_) => "floor",
            TOp::AddMonths(_) => "add_months",
            TOp::MonthsBetween => "months_between",
        }
    }

    /// The component parts by their method names.
    pub fn part(name: &str) -> Option<TOp> {
        Some(match name {
            "year" => TOp::Year,
            "quarter" => TOp::Quarter,
            "month" => TOp::Month,
            "day" => TOp::Day,
            "weekday" => TOp::Weekday,
            "day_of_year" => TOp::DayOfYear,
            "week" => TOp::Week,
            "hour" => TOp::Hour,
            "minute" => TOp::Minute,
            "second" => TOp::Second,
            "is_weekend" => TOp::IsWeekend,
            "date" => TOp::Date,
            _ => return None,
        })
    }
}

/// The type a step's values are stored as: a Date or Datetime is its
/// integer, everything else itself.
#[inline]
fn phys(d: DType) -> DType {
    match d {
        DType::Date | DType::Datetime => DType::I64,
        d => d,
    }
}

fn temporal(d: Option<DType>) -> bool {
    matches!(d, Some(DType::Date | DType::Datetime))
}

impl Op {
    fn cost(self) -> usize {
        match self {
            Op::Math(m) => m.cost(),
            Op::Bin(BinOp::Pow) => 12,
            Op::Bin(BinOp::Div | BinOp::Mod) => 2,
            Op::Round(_) => 3,
            Op::Where | Op::Coalesce | Op::Clip | Op::Between(_) => 2,
            Op::Temporal(TOp::Raw | TOp::AsDate | TOp::AsDatetime | TOp::ToDatetime, _) => 1,
            Op::Temporal(_, _) => 4,
            _ => 1,
        }
    }
}

/// An elementwise expression over concrete columns.
#[derive(Clone, Debug)]
pub enum Expr {
    /// A column, of any dtype (a String column only as an operand of a
    /// comparison, a null mask, `where` or `coalesce`).
    Col(Series),
    /// A scalar broadcast to every row; `Scalar::Null` is a null column.
    Lit(Scalar),
    Op(Op, Vec<Arc<Expr>>),
}

impl Expr {
    pub fn col(s: Series) -> Arc<Expr> {
        Arc::new(Expr::Col(s))
    }

    pub fn lit(s: Scalar) -> Arc<Expr> {
        Arc::new(Expr::Lit(s))
    }

    pub fn op(op: Op, args: Vec<Arc<Expr>>) -> Arc<Expr> {
        Arc::new(Expr::Op(op, args))
    }

    pub fn bin(op: BinOp, a: Arc<Expr>, b: Arc<Expr>) -> Arc<Expr> {
        Arc::new(Expr::Op(Op::Bin(op), vec![a, b]))
    }

    /// The dtype a node computes, from its children's; `None` is a null
    /// literal, which takes the type its context gives it.
    pub fn dtype(&self) -> Result<Option<DType>> {
        match self {
            Expr::Col(s) => Ok(Some(s.dtype())),
            Expr::Lit(s) => Ok(scalar_dtype(s)),
            Expr::Op(op, args) => {
                let kids: Vec<Option<DType>> =
                    args.iter().map(|a| a.dtype()).collect::<Result<_>>()?;
                op_dtype(*op, &kids)
            }
        }
    }

    /// The column length the node's columns agree on (None: scalars
    /// only).
    pub fn len_hint(&self) -> Option<usize> {
        match self {
            Expr::Col(s) => Some(s.len()),
            Expr::Lit(_) => None,
            Expr::Op(_, args) => args.iter().find_map(|a| a.len_hint()),
        }
    }

    /// Whether evaluating the node can raise: a division, a remainder,
    /// checked Int arithmetic, or a function with a restricted domain.
    /// Callers that defer evaluation defer only what cannot.
    pub fn fallible(&self) -> bool {
        match self {
            Expr::Col(_) | Expr::Lit(_) => false,
            Expr::Op(op, args) => {
                let me = match op {
                    Op::Bin(BinOp::Div | BinOp::Mod) => true,
                    Op::Bin(BinOp::Add | BinOp::Sub | BinOp::Mul) | Op::Neg => {
                        args.iter().all(|a| {
                            matches!(a.dtype(), Ok(Some(DType::I64) | None))
                        }) || args.iter().any(|a| temporal(a.dtype().ok().flatten()))
                    }
                    Op::Temporal(TOp::AsDate | TOp::AddMonths(_), _) => true,
                    Op::Math(Math::Abs) | Op::Round(_) => {
                        matches!(args[0].dtype(), Ok(Some(DType::I64)))
                    }
                    Op::Math(m) => m.domain().is_some(),
                    _ => false,
                };
                me || args.iter().any(|a| a.fallible())
            }
        }
    }

    /// What computing a row costs, an arithmetic operation counting one
    /// (a shared subtree counted once): the measure a caller weighs
    /// recomputing against keeping a column.
    pub fn cost(&self) -> usize {
        fn walk(e: &Expr, seen: &mut std::collections::HashSet<usize>) -> usize {
            match e {
                Expr::Op(op, args) => {
                    let mut c = op.cost();
                    for a in args {
                        if seen.insert(Arc::as_ptr(a) as usize) {
                            c += walk(a, seen);
                        }
                    }
                    c
                }
                _ => 0,
            }
        }
        walk(self, &mut std::collections::HashSet::new())
    }

    /// Evaluate one expression into a column of `len` rows (`len` is
    /// only consulted when the expression reads no column).
    pub fn eval(self: &Arc<Self>, len: usize, par: bool) -> Result<Series> {
        let plan = Plan::new(std::slice::from_ref(self), Some(len))?;
        Ok(plan.eval(par)?.pop().expect("one root"))
    }
}

fn scalar_dtype(s: &Scalar) -> Option<DType> {
    match s {
        Scalar::F64(_) => Some(DType::F64),
        Scalar::I64(_) => Some(DType::I64),
        Scalar::Bool(_) => Some(DType::Bool),
        Scalar::Str(_) => Some(DType::Str),
        Scalar::Date(_) => Some(DType::Date),
        Scalar::Datetime(_) => Some(DType::Datetime),
        Scalar::Null => None,
    }
}

fn name_of(d: Option<DType>) -> String {
    match d {
        Some(d) => format!("Series[{}]", d),
        None => "()".to_string(),
    }
}

fn numeric(d: Option<DType>) -> bool {
    matches!(d, Some(DType::F64 | DType::I64) | None)
}

/// The type two values meet at: equal types, Int and Float at Float, a
/// null at the other's.
fn common(a: Option<DType>, b: Option<DType>, what: &str) -> Result<Option<DType>> {
    Ok(match (a, b) {
        (None, x) | (x, None) => x,
        (Some(x), Some(y)) if x == y => Some(x),
        (Some(DType::F64 | DType::I64), Some(DType::F64 | DType::I64)) => Some(DType::F64),
        (Some(x), Some(y)) => {
            return Err(OdsError::TypeMismatch(format!(
                "{}: the values are {} and {}, which have no common type",
                what, x, y
            )));
        }
    })
}

/// The result type of `op` over operands of these types.
pub fn op_dtype(op: Op, kids: &[Option<DType>]) -> Result<Option<DType>> {
    // A Category reads as its text.
    let kids: Vec<Option<DType>> = kids
        .iter()
        .map(|k| match k {
            Some(DType::Category) => Some(DType::Str),
            k => *k,
        })
        .collect();
    let kids = kids.as_slice();
    if let Some(t) = temporal_dtype(op, kids)? {
        return Ok(t);
    }
    let mismatch = |what: &str| {
        Err(OdsError::TypeMismatch(format!(
            "cannot apply {} to {}",
            what,
            kids.iter().map(|k| name_of(*k)).collect::<Vec<_>>().join(" and ")
        )))
    };
    match op {
        Op::Bin(b) if b.is_logic() => {
            if kids
                .iter()
                .all(|k| matches!(k, Some(DType::Bool) | None))
            {
                Ok(Some(DType::Bool))
            } else {
                mismatch(b.word())
            }
        }
        Op::Bin(b) if b.is_cmp() => {
            let ok = match (kids[0], kids[1]) {
                (a, b) if numeric(a) && numeric(b) => true,
                (Some(DType::Str), Some(DType::Str) | None)
                | (None, Some(DType::Str)) => true,
                (Some(DType::Date), Some(DType::Date) | None)
                | (None, Some(DType::Date))
                | (Some(DType::Datetime), Some(DType::Datetime) | None)
                | (None, Some(DType::Datetime)) => true,
                (Some(DType::Bool), Some(DType::Bool) | None)
                | (None, Some(DType::Bool)) => {
                    if !matches!(b, BinOp::Eq | BinOp::Ne) {
                        return Err(OdsError::TypeMismatch(
                            "Bool series support only == and != comparisons".to_string(),
                        ));
                    }
                    true
                }
                _ => false,
            };
            if ok {
                Ok(Some(DType::Bool))
            } else {
                Err(OdsError::TypeMismatch(format!(
                    "cannot compare {} with {}",
                    name_of(kids[0]),
                    name_of(kids[1])
                )))
            }
        }
        Op::Bin(b) => {
            if !(numeric(kids[0]) && numeric(kids[1])) {
                return mismatch(b.word());
            }
            Ok(match b {
                BinOp::Div | BinOp::Pow => Some(DType::F64),
                _ => match (kids[0], kids[1]) {
                    (Some(DType::I64) | None, Some(DType::I64) | None)
                        if kids.iter().any(|k| k.is_some()) =>
                    {
                        Some(DType::I64)
                    }
                    (None, None) => None,
                    _ => Some(DType::F64),
                },
            })
        }
        Op::Neg => {
            if numeric(kids[0]) {
                Ok(kids[0])
            } else {
                mismatch("negation")
            }
        }
        Op::Not => match kids[0] {
            Some(DType::Bool) | None => Ok(Some(DType::Bool)),
            _ => mismatch("!"),
        },
        Op::Math(m) => {
            if !numeric(kids[0]) {
                return Err(OdsError::TypeMismatch(format!(
                    "{}: a {} is not numeric",
                    m.name(),
                    name_of(kids[0])
                )));
            }
            Ok(match kids[0] {
                Some(DType::I64) if m.keeps_int() => Some(DType::I64),
                _ => Some(DType::F64),
            })
        }
        Op::Round(digits) => {
            if !numeric(kids[0]) {
                return mismatch("round");
            }
            Ok(match kids[0] {
                Some(DType::I64) => Some(DType::I64),
                _ => {
                    let _ = digits;
                    Some(DType::F64)
                }
            })
        }
        Op::IsNull | Op::NotNull => Ok(Some(DType::Bool)),
        Op::IsNan => {
            if numeric(kids[0]) {
                Ok(Some(DType::Bool))
            } else {
                mismatch("is_nan")
            }
        }
        Op::Where => {
            if !matches!(kids[0], Some(DType::Bool) | None) {
                return Err(OdsError::TypeMismatch(format!(
                    "where: the condition must be a Bool mask, got {}",
                    name_of(kids[0])
                )));
            }
            common(kids[1], kids[2], "where")
        }
        Op::Coalesce => {
            let mut t = None;
            for k in kids {
                t = common(t, *k, "coalesce")?;
            }
            Ok(t)
        }
        Op::Clip => {
            if !kids.iter().all(|k| numeric(*k)) {
                return mismatch("clip");
            }
            Ok(match kids[0] {
                Some(DType::I64)
                    if kids[1..]
                        .iter()
                        .all(|k| matches!(k, Some(DType::I64) | None)) =>
                {
                    Some(DType::I64)
                }
                _ => Some(DType::F64),
            })
        }
        Op::Between(_) => {
            let ok = kids.iter().all(|k| numeric(*k))
                || kids
                    .iter()
                    .all(|k| matches!(k, Some(DType::Str) | None))
                || kids
                    .iter()
                    .all(|k| matches!(k, Some(DType::Date) | None))
                || kids
                    .iter()
                    .all(|k| matches!(k, Some(DType::Datetime) | None));
            if ok {
                Ok(Some(DType::Bool))
            } else {
                mismatch("between")
            }
        }
        Op::Cast(to) => match (kids[0], to) {
            (_, DType::Str | DType::Date | DType::Datetime | DType::Category)
            | (Some(DType::Str | DType::Date | DType::Datetime | DType::Category), _) => {
                Err(OdsError::TypeMismatch(
                    "the expression engine casts among Float, Int and Bool".to_string(),
                ))
            }
            (Some(DType::F64), DType::Bool) => Err(OdsError::InvalidArgument(
                "cast: Float to Bool has no meaning for values like 0.5. \
                 Write the comparison you mean, e.g. `s != 0.0`"
                    .to_string(),
            )),
            _ => Ok(Some(to)),
        },
        Op::Temporal(..) => unreachable!("temporal_dtype"),
    }
}

/// The result type of a calendar operation, or of arithmetic with a
/// Date or Datetime (None: neither, the numeric rules decide).
fn temporal_dtype(op: Op, kids: &[Option<DType>]) -> Result<Option<Option<DType>>> {
    use DType::{Date, Datetime, I64};
    let name = |k: &Option<DType>| name_of(*k);
    match op {
        Op::Temporal(t, _) => {
            let k = kids[0];
            let out = match t {
                TOp::AsDate | TOp::AsDatetime => {
                    if !matches!(k, Some(I64) | None) {
                        return Err(OdsError::TypeMismatch(format!(
                            "{}: needs an Int, got {}",
                            t.name(),
                            name(&k)
                        )));
                    }
                    Some(if t == TOp::AsDate { Date } else { Datetime })
                }
                TOp::MonthsBetween => {
                    let ok = matches!(
                        (kids[0], kids[1]),
                        (Some(Date) | None, Some(Date) | None)
                            | (Some(Datetime) | None, Some(Datetime) | None)
                    );
                    if !ok {
                        return Err(OdsError::TypeMismatch(format!(
                            "months_between: needs two Dates or two Datetimes, got {} and {}",
                            name(&kids[0]),
                            name(&kids[1])
                        )));
                    }
                    Some(I64)
                }
                _ if k.is_some() && !temporal(k) => {
                    return Err(OdsError::TypeMismatch(format!(
                        "{}: needs a Date or Datetime column, got {}{}",
                        t.name(),
                        name(&k),
                        if k == Some(DType::Str) {
                            " (read text as dates with to_date() or to_datetime())"
                        } else {
                            ""
                        }
                    )));
                }
                TOp::Hour | TOp::Minute | TOp::Second if k == Some(Date) => {
                    return Err(OdsError::TypeMismatch(format!(
                        "{}: a Date has no time of day (a Datetime does)",
                        t.name()
                    )));
                }
                TOp::Floor(e) if k == Some(Date) && e.below_day() => {
                    return Err(OdsError::InvalidArgument(
                        "floor: a Date cannot be cut finer than a day".to_string(),
                    ));
                }
                TOp::Year
                | TOp::Quarter
                | TOp::Month
                | TOp::Day
                | TOp::Weekday
                | TOp::DayOfYear
                | TOp::Week
                | TOp::Hour
                | TOp::Minute
                | TOp::Second
                | TOp::Raw => Some(I64),
                TOp::IsWeekend => Some(DType::Bool),
                TOp::Date => Some(Date),
                TOp::ToDatetime => Some(Datetime),
                TOp::Floor(_) | TOp::AddMonths(_) => k.or(Some(Date)),
            };
            Ok(Some(out))
        }
        Op::Bin(b) if !b.is_cmp() && !b.is_logic() && kids.iter().any(|k| temporal(*k)) => {
            let int = |k: Option<DType>| matches!(k, Some(I64) | None);
            Ok(Some(match (b, kids[0], kids[1]) {
                (BinOp::Add, Some(Date), y) if int(y) => Some(Date),
                (BinOp::Add, x, Some(Date)) if int(x) => Some(Date),
                (BinOp::Sub, Some(Date), y) if int(y) => Some(Date),
                (BinOp::Sub, Some(Date), Some(Date)) => Some(I64),
                (BinOp::Sub, Some(Datetime), Some(Datetime)) => Some(DType::F64),
                _ => {
                    return Err(OdsError::TypeMismatch(format!(
                        "cannot apply {} to {} and {}: a Date takes + and - of an Int of days, \
                         and Date - Date is days between; a Datetime moves by add_days and \
                         add_months, and Datetime - Datetime is seconds between",
                        b.word(),
                        name(&kids[0]),
                        name(&kids[1])
                    )));
                }
            }))
        }
        Op::Neg | Op::Math(_) | Op::Round(_) | Op::Clip | Op::IsNan if temporal(kids[0]) => {
            Err(OdsError::TypeMismatch(format!(
                "{} is not defined for {}",
                match op {
                    Op::Neg => "negation".to_string(),
                    Op::Math(m) => m.name().to_string(),
                    Op::Round(_) => "round".to_string(),
                    Op::Clip => "clip".to_string(),
                    _ => "is_nan".to_string(),
                },
                name_of(kids[0])
            )))
        }
        _ => Ok(None),
    }
}

/// Arithmetic with a Date or Datetime as arithmetic on their integers:
/// `d + n` is days added and range-checked, `d - d` days between, and
/// `t - t` seconds between. None for anything else.
fn rewrite_temporal(op: Op, args: &[Arc<Expr>], kids: &[Option<DType>]) -> Option<Arc<Expr>> {
    use DType::{Date, Datetime};
    let Op::Bin(b @ (BinOp::Add | BinOp::Sub)) = op else {
        return None;
    };
    let raw = |e: &Arc<Expr>| Expr::op(Op::Temporal(TOp::Raw, false), vec![e.clone()]);
    let as_date = |e: Arc<Expr>| Expr::op(Op::Temporal(TOp::AsDate, false), vec![e]);
    Some(match (kids[0], kids[1]) {
        (Some(Date), Some(Date)) => Expr::bin(BinOp::Sub, raw(&args[0]), raw(&args[1])),
        (Some(Datetime), Some(Datetime)) => Expr::bin(
            BinOp::Div,
            Expr::bin(BinOp::Sub, raw(&args[0]), raw(&args[1])),
            Expr::lit(Scalar::F64(crate::temporal::US_PER_SEC as f64)),
        ),
        (Some(Date), _) => as_date(Expr::bin(b, raw(&args[0]), args[1].clone())),
        (_, Some(Date)) => as_date(Expr::bin(b, args[0].clone(), raw(&args[1]))),
        _ => return None,
    })
}

// ---------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------

#[derive(Debug)]
enum Kind {
    Col(Series),
    Lit(Scalar),
    Op(Op),
}

#[derive(Debug)]
struct Step {
    kind: Kind,
    args: Vec<usize>,
    /// The values' type; a Date or Datetime is stored as its integer
    /// (`phys`), so it computes in Int registers and kernels.
    dt: DType,
    /// A Date, Datetime or Category column as it was given (the step
    /// holds its integers or its text): what a root that is the column
    /// gives back.
    orig: Option<Series>,
    /// Whether any position can be null (statically): no validity work
    /// is done for a step that cannot.
    may_null: bool,
}

/// What a step's identity is, for merging common subexpressions.
#[derive(Hash, PartialEq, Eq)]
enum Key {
    Col(usize, usize, usize, u8),
    Lit(u8, u64, String),
    Op(Op, Vec<usize>, DType),
}

fn series_key(s: &Series) -> Key {
    let (ptr, tag) = match s {
        Series::F64 { values, .. } => (Arc::as_ptr(values) as usize, 0),
        Series::I64 { values, .. } => (Arc::as_ptr(values) as usize, 1),
        Series::Bool { values, .. } => (Arc::as_ptr(values) as usize, 2),
        Series::Str { col, .. } => (col.spans().as_ptr() as usize, 3),
        Series::Date { values, .. } => (Arc::as_ptr(values) as usize, 4),
        Series::Datetime { values, .. } => (Arc::as_ptr(values) as usize, 5),
        Series::Cat { codes, .. } => (Arc::as_ptr(codes) as usize, 6),
    };
    let v = s.validity().map(|b| b.words_ptr()).unwrap_or(0);
    Key::Col(ptr, v, s.len(), tag)
}

fn lit_key(s: &Scalar, dt: DType) -> Key {
    let d = dt as u8;
    match s {
        Scalar::F64(x) => Key::Lit(d, x.to_bits(), String::new()),
        Scalar::I64(x) => Key::Lit(d | 16, *x as u64, String::new()),
        Scalar::Bool(b) => Key::Lit(d | 32, *b as u64, String::new()),
        Scalar::Str(t) => Key::Lit(d | 48, 0, t.clone()),
        Scalar::Null => Key::Lit(d | 64, 0, String::new()),
        Scalar::Date(x) => Key::Lit(d | 80, *x as u64, String::new()),
        Scalar::Datetime(x) => Key::Lit(d | 96, *x as u64, String::new()),
    }
}

/// A compiled set of expressions over the same rows.
#[derive(Debug)]
pub struct Plan {
    steps: Vec<Step>,
    roots: Vec<usize>,
    len: usize,
    /// For each step, the register holding it while it is live (None:
    /// a column or scalar read where it is, or a root written straight
    /// to its output).
    reg: Vec<Option<usize>>,
    /// Each register's dtype.
    regs: Vec<DType>,
    /// For each step that is a root, its output index.
    out_of: Vec<Option<usize>>,
    /// Roots no other step reads: written straight into their output.
    direct: Vec<bool>,
    cost: usize,
    leaves: usize,
}

struct Builder {
    steps: Vec<Step>,
    keys: HashMap<Key, usize>,
    seen: HashMap<(usize, Option<DType>), usize>,
    len: Option<usize>,
}

impl Builder {
    fn push(&mut self, key: Key, step: Step) -> usize {
        if let Some(&id) = self.keys.get(&key) {
            return id;
        }
        let id = self.steps.len();
        self.steps.push(step);
        self.keys.insert(key, id);
        id
    }

    fn check_len(&mut self, n: usize) -> Result<()> {
        match self.len {
            None => {
                self.len = Some(n);
                Ok(())
            }
            Some(m) if m == n => Ok(()),
            Some(m) => Err(OdsError::LengthMismatch { left: m, right: n }),
        }
    }

    /// A literal of type `dt` (an Int literal in Float context becomes
    /// a Float literal; a null literal takes `dt`).
    fn lit(&mut self, s: &Scalar, dt: DType) -> usize {
        let s = match (s, dt) {
            (Scalar::I64(x), DType::F64) => Scalar::F64(*x as f64),
            (Scalar::Bool(b), DType::F64) => Scalar::F64(if *b { 1.0 } else { 0.0 }),
            (Scalar::Bool(b), DType::I64) => Scalar::I64(*b as i64),
            // A Date or Datetime literal is its integer.
            (Scalar::Date(x), _) => Scalar::I64(*x as i64),
            (Scalar::Datetime(x), _) => Scalar::I64(*x),
            _ => s.clone(),
        };
        let may_null = matches!(s, Scalar::Null);
        self.push(
            lit_key(&s, dt),
            Step {
                kind: Kind::Lit(s),
                args: vec![],
                dt,
                orig: None,
                may_null,
            },
        )
    }

    /// The step computing `e`, converted to `want` when given (an Int
    /// widened to Float, a Bool to a number for arithmetic).
    fn node(&mut self, e: &Arc<Expr>, want: Option<DType>) -> Result<usize> {
        let memo = (Arc::as_ptr(e) as usize, want);
        if let Some(&id) = self.seen.get(&memo) {
            return Ok(id);
        }
        let id = match e.as_ref() {
            Expr::Lit(s) => {
                let dt = want.or(scalar_dtype(s)).unwrap_or(DType::F64);
                self.lit(s, dt)
            }
            Expr::Col(s) => {
                self.check_len(s.len())?;
                // A Date or Datetime is read as its integers, a Category
                // as its text; the column itself is kept for a root.
                let (store, dt, orig) = match s {
                    Series::Date { .. } | Series::Datetime { .. } => {
                        (s.physical().expect("temporal"), s.dtype(), Some(s.clone()))
                    }
                    Series::Cat { .. } => {
                        (s.decoded().expect("a category"), DType::Str, Some(s.clone()))
                    }
                    _ => (s.clone(), s.dtype(), None),
                };
                let id = self.push(
                    series_key(s),
                    Step {
                        kind: Kind::Col(store),
                        args: vec![],
                        dt,
                        orig,
                        may_null: s.validity().is_some(),
                    },
                );
                self.convert(id, want)?
            }
            Expr::Op(op, args) => {
                let kids: Vec<Option<DType>> =
                    args.iter().map(|a| a.dtype()).collect::<Result<_>>()?;
                let out = op_dtype(*op, &kids)?.unwrap_or(DType::F64);
                if let Some(e2) = rewrite_temporal(*op, args, &kids) {
                    let id = self.node(&e2, want)?;
                    self.seen.insert(memo, id);
                    return Ok(id);
                }
                // The planner's flag: the operand is a Datetime.
                let op = &match op {
                    Op::Temporal(t, _) => Op::Temporal(*t, kids[0] == Some(DType::Datetime)),
                    o => *o,
                };
                // The type the operation computes in.
                let operand: Vec<Option<DType>> = match op {
                    Op::Bin(b) if b.is_logic() => vec![Some(DType::Bool); 2],
                    Op::Bin(b) if b.is_cmp() => {
                        let t = common(kids[0], kids[1], "comparison")?.unwrap_or(DType::F64);
                        vec![Some(t); 2]
                    }
                    Op::Bin(BinOp::Div | BinOp::Pow) => vec![Some(DType::F64); 2],
                    Op::Bin(_) => vec![Some(out); 2],
                    Op::Neg | Op::Math(_) | Op::Round(_) => vec![Some(out)],
                    Op::Not => vec![Some(DType::Bool)],
                    Op::IsNull | Op::NotNull => vec![None],
                    Op::IsNan => vec![Some(DType::F64)],
                    Op::Where => vec![Some(DType::Bool), Some(out), Some(out)],
                    Op::Coalesce => vec![Some(out); args.len()],
                    Op::Clip => vec![Some(out); 3],
                    Op::Between(_) => {
                        let mut t = None;
                        for k in &kids {
                            t = common(t, *k, "between")?;
                        }
                        vec![Some(t.unwrap_or(DType::F64)); 3]
                    }
                    Op::Cast(_) => vec![None],
                    Op::Temporal(TOp::AsDate | TOp::AsDatetime, _) => vec![Some(DType::I64)],
                    Op::Temporal(..) => vec![None; args.len()],
                };
                // A String result is built row by row from its operands,
                // then read as a column: the fused kernels are numeric.
                if out == DType::Str {
                    let s = self.text_node(*op, args)?;
                    self.check_len(s.len())?;
                    let id = self.push(
                        series_key(&s),
                        Step {
                            may_null: s.validity().is_some(),
                            dt: DType::Str,
                            kind: Kind::Col(s),
                            args: vec![],
                            orig: None,
                        },
                    );
                    self.seen.insert(memo, id);
                    return Ok(id);
                }
                let mut ids = Vec::with_capacity(args.len());
                for (a, w) in args.iter().zip(operand.iter()) {
                    ids.push(self.node(a, *w)?);
                }
                // Same-type casts are the operand itself.
                if let Op::Cast(to) = op
                    && self.steps[ids[0]].dt == *to
                {
                    let id = self.convert(ids[0], want)?;
                    self.seen.insert(memo, id);
                    return Ok(id);
                }
                let may_null = match op {
                    Op::IsNull | Op::NotNull => false,
                    Op::Coalesce => ids.iter().all(|&i| self.steps[i].may_null),
                    Op::Clip | Op::Between(_) => self.steps[ids[0]].may_null,
                    Op::Where => {
                        self.steps[ids[0]].may_null
                            || self.steps[ids[1]].may_null
                            || self.steps[ids[2]].may_null
                    }
                    Op::Cast(DType::I64) if self.steps[ids[0]].dt == DType::F64 => true,
                    _ => ids.iter().any(|&i| self.steps[i].may_null),
                };
                let id = self.push(
                    Key::Op(*op, ids.clone(), out),
                    Step {
                        kind: Kind::Op(*op),
                        args: ids,
                        dt: out,
                        orig: None,
                        may_null,
                    },
                );
                self.convert(id, want)?
            }
        };
        self.seen.insert(memo, id);
        Ok(id)
    }

    /// `id` as type `want`: an implicit widening step when needed.
    fn convert(&mut self, id: usize, want: Option<DType>) -> Result<usize> {
        let Some(want) = want else { return Ok(id) };
        let have = self.steps[id].dt;
        if have == want {
            return Ok(id);
        }
        match (have, want) {
            (DType::I64, DType::F64) | (DType::Bool, DType::F64) | (DType::Bool, DType::I64) => {}
            _ => {
                return Err(OdsError::TypeMismatch(format!(
                    "a Series[{}] cannot stand where a {} is needed",
                    have, want
                )));
            }
        }
        if let Kind::Lit(s) = &self.steps[id].kind {
            let s = s.clone();
            return Ok(self.lit(&s, want));
        }
        let may_null = self.steps[id].may_null;
        Ok(self.push(
            Key::Op(Op::Cast(want), vec![id], want),
            Step {
                kind: Kind::Op(Op::Cast(want)),
                args: vec![id],
                dt: want,
                orig: None,
                may_null,
            },
        ))
    }

    /// A String-valued `where` or `coalesce`, row by row into one text
    /// buffer (the conditions evaluate as columns; a text operand is read
    /// where it is).
    fn text_node(&mut self, op: Op, args: &[Arc<Expr>]) -> Result<Series> {
        let len = self
            .len
            .or_else(|| args.iter().find_map(|a| a.len_hint()))
            .ok_or_else(|| {
                OdsError::InvalidArgument("a text expression needs a column".to_string())
            })?;
        enum T {
            Col(Series),
            Lit(Option<String>),
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(match a.as_ref() {
                Expr::Lit(Scalar::Str(t)) => T::Lit(Some(t.clone())),
                Expr::Lit(_) => T::Lit(None),
                _ => {
                    let s = a.eval(len, false)?;
                    T::Col(s.decoded().unwrap_or(s))
                }
            });
        }
        fn text(t: &T, i: usize) -> Option<&str> {
            match t {
                T::Lit(x) => x.as_deref(),
                T::Col(Series::Str { col, validity }) => {
                    validity.as_ref().is_none_or(|v| v.get(i)).then(|| col.get(i))
                }
                T::Col(_) => None,
            }
        }
        let pick = |i: usize| -> Option<&str> {
            match op {
                Op::Where => match &vals[0] {
                    T::Col(Series::Bool { values, validity }) => {
                        if !validity.as_ref().is_none_or(|v| v.get(i)) {
                            None
                        } else if values[i] {
                            text(&vals[1], i)
                        } else {
                            text(&vals[2], i)
                        }
                    }
                    T::Lit(_) | T::Col(_) => None,
                },
                _ => vals.iter().find_map(|t| text(t, i)),
            }
        };
        let mut buf = String::new();
        let mut spans = Vec::with_capacity(len);
        let mut valid = vec![true; len];
        for (i, ok) in valid.iter_mut().enumerate() {
            let start = buf.len() as u32;
            match pick(i) {
                Some(t) => buf.push_str(t),
                None => *ok = false,
            }
            spans.push((start, buf.len() as u32 - start));
        }
        assert!(
            u32::try_from(buf.len()).is_ok(),
            "text column exceeds 4 GB of backing text"
        );
        let validity = (!valid.iter().all(|&b| b)).then(|| Bitmap::from_bools(&valid));
        Ok(Series::Str {
            col: StrCol::from_spans(Arc::from(buf), spans),
            validity,
        })
    }
}

impl Plan {
    /// Compile `roots`. Their columns must agree on a length; `len`
    /// gives it when no root reads a column.
    pub fn new(roots: &[Arc<Expr>], len: Option<usize>) -> Result<Plan> {
        let mut b = Builder {
            steps: Vec::new(),
            keys: HashMap::new(),
            seen: HashMap::new(),
            len: None,
        };
        let mut root_ids = Vec::with_capacity(roots.len());
        for r in roots {
            root_ids.push(b.node(r, None)?);
        }
        let len = b.len.or(len).unwrap_or(0);
        let steps = b.steps;
        let n = steps.len();
        let mut last_use = vec![0usize; n];
        for (i, s) in steps.iter().enumerate() {
            last_use[i] = i;
            for &a in &s.args {
                last_use[a] = i;
            }
        }
        let mut out_of = vec![None; n];
        let direct: Vec<bool> = (0..n)
            .map(|i| root_ids.contains(&i) && last_use[i] == i)
            .collect();
        for (k, &r) in root_ids.iter().enumerate() {
            if out_of[r].is_none() {
                out_of[r] = Some(k);
            }
            // A root stays live through the chunk: it is copied out, or
            // folded, after every step has run.
            last_use[r] = n;
        }
        // Registers: a step that computes takes one from the free list
        // of its dtype; a step whose last reader has run gives its back.
        let mut reg = vec![None; n];
        let mut regs: Vec<DType> = Vec::new();
        let mut free: Vec<usize> = Vec::new();
        for i in 0..n {
            if let Kind::Op(_) = steps[i].kind {
                let dt = phys(steps[i].dt);
                let r = match free.iter().position(|&r| regs[r] == dt) {
                    Some(p) => free.swap_remove(p),
                    None => {
                        regs.push(dt);
                        regs.len() - 1
                    }
                };
                reg[i] = Some(r);
            }
            for &a in &steps[i].args {
                if last_use[a] == i
                    && let Some(r) = reg[a]
                    && !free.contains(&r)
                {
                    free.push(r);
                }
            }
        }
        let cost = steps
            .iter()
            .map(|s| match s.kind {
                Kind::Op(op) => op.cost(),
                _ => 0,
            })
            .sum();
        let leaves = steps
            .iter()
            .filter(|s| matches!(s.kind, Kind::Col(_)))
            .count();
        Ok(Plan {
            steps,
            roots: root_ids,
            len,
            reg,
            regs,
            out_of,
            direct,
            cost,
            leaves,
        })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The root's dtype.
    pub fn dtype(&self, root: usize) -> DType {
        self.steps[self.roots[root]].dt
    }

    /// How many operations the plan runs per row.
    pub fn ops(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s.kind, Kind::Op(_)))
            .count()
    }

    /// The fewest rows at which this plan fans out. One operation keeps
    /// its kernel class's measured threshold (`ParClass`); a longer plan
    /// costs the sum of its operations a row, so it crosses sooner.
    pub fn par_rows(&self) -> usize {
        let class = if self.leaves > 1 {
            ParClass::Elementwise
        } else {
            ParClass::ElementwiseScalar
        };
        let base = class.min_rows();
        if self.cost <= 1 {
            return base;
        }
        // The measured threshold of one operation between columns,
        // divided among the plan's operations (a transcendental function
        // counts a dozen): benches/expr.rs measures the crossover.
        (crate::PAR_ELEMENTWISE_ROWS * 4 / self.cost).clamp(64 * 1024, base)
    }

    /// The fewest rows at which a reduction over this plan fans out.
    pub fn par_reduce_rows(&self) -> usize {
        let base = ParClass::Reduce.min_rows();
        if self.cost == 0 {
            return base;
        }
        (crate::PAR_ELEMENTWISE_ROWS * 2 / (self.cost + 1)).clamp(64 * 1024, base)
    }

    /// The eager kernel a one-operation plan is: Float arithmetic
    /// between columns and scalars, done as the eager kernels do it.
    fn single_arith(&self, par: bool) -> Option<Result<Series>> {
        let [root] = self.roots.as_slice() else {
            return None;
        };
        let step = &self.steps[*root];
        let (Kind::Op(Op::Bin(b)), DType::F64) = (&step.kind, step.dt) else {
            return None;
        };
        let op = match b {
            BinOp::Add => crate::ArithOp::Add,
            BinOp::Sub => crate::ArithOp::Sub,
            BinOp::Mul => crate::ArithOp::Mul,
            BinOp::Div => crate::ArithOp::Div,
            _ => return None,
        };
        let side = |i: usize| match &self.steps[step.args[i]].kind {
            Kind::Col(s @ Series::F64 { .. }) => Some(Ok(s)),
            Kind::Lit(Scalar::F64(x)) => Some(Err(*x)),
            _ => None,
        };
        Some(match (side(0)?, side(1)?) {
            (Ok(x), Ok(y)) => x.arith(op, y, par),
            (Ok(x), Err(c)) => x.arith_scalar(op, Scalar::F64(c), false, par),
            (Err(c), Ok(y)) => y.arith_scalar(op, Scalar::F64(c), true, par),
            (Err(_), Err(_)) => return None,
        })
    }

    /// Evaluate every root into a column.
    pub fn eval(&self, par: bool) -> Result<Vec<Series>> {
        let n = self.len;
        if let Some(r) = self.single_arith(par) {
            return Ok(vec![r?]);
        }
        let nw = n.div_ceil(64);
        // A root that is a column or a scalar needs no pass; the others
        // are written into fresh buffers, every element exactly once.
        let mut outs: Vec<Option<Out>> = Vec::with_capacity(self.roots.len());
        for (k, &r) in self.roots.iter().enumerate() {
            let s = &self.steps[r];
            outs.push(match s.kind {
                Kind::Op(_) if self.out_of[r] == Some(k) => Some(Out {
                    vals: match phys(s.dt) {
                        DType::F64 => OutVals::F(Vec::with_capacity(n)),
                        DType::I64 => OutVals::I(Vec::with_capacity(n)),
                        DType::Bool => OutVals::B(Vec::with_capacity(n)),
                        _ => unreachable!("text roots are columns"),
                    },
                    valid: s.may_null.then(|| vec![0u64; nw]),
                }),
                _ => None,
            });
        }
        if outs.iter().any(|o| o.is_some()) {
            #[cfg(feature = "parallel")]
            if par && n > piece_rows(n) {
                let piece = piece_rows(n);
                let pieces = n.div_ceil(piece);
                let mut per_piece: Vec<Vec<Option<OutSlice<'_>>>> =
                    (0..pieces).map(|_| Vec::with_capacity(outs.len())).collect();
                for o in outs.iter_mut() {
                    match o {
                        None => per_piece.iter_mut().for_each(|p| p.push(None)),
                        Some(o) => {
                            let vals: Vec<OutValsMut<'_>> = match &mut o.vals {
                                OutVals::F(v) => v.spare_capacity_mut()[..n]
                                    .chunks_mut(piece)
                                    .map(OutValsMut::F)
                                    .collect(),
                                OutVals::I(v) => v.spare_capacity_mut()[..n]
                                    .chunks_mut(piece)
                                    .map(OutValsMut::I)
                                    .collect(),
                                OutVals::B(v) => v.spare_capacity_mut()[..n]
                                    .chunks_mut(piece)
                                    .map(OutValsMut::B)
                                    .collect(),
                            };
                            let mut valid: Vec<Option<&mut [u64]>> = match &mut o.valid {
                                Some(w) => w.chunks_mut(piece / 64).map(Some).collect(),
                                None => (0..pieces).map(|_| None).collect(),
                            };
                            for (p, v) in vals.into_iter().enumerate() {
                                per_piece[p].push(Some(OutSlice {
                                    vals: v,
                                    valid: valid[p].take(),
                                }));
                            }
                        }
                    }
                }
                let results: Vec<Result<()>> = per_piece
                    .into_par_iter()
                    .enumerate()
                    .map(|(p, mut outs)| {
                        let start = p * piece;
                        let end = (start + piece).min(n);
                        self.run(start, end, &mut outs, None)
                    })
                    .collect();
                for r in results {
                    r?;
                }
                return Ok(self.finish(outs));
            }
            let _ = par;
            let mut slices: Vec<Option<OutSlice<'_>>> = outs
                .iter_mut()
                .map(|o| {
                    o.as_mut().map(|o| OutSlice {
                        vals: match &mut o.vals {
                            OutVals::F(v) => OutValsMut::F(&mut v.spare_capacity_mut()[..n]),
                            OutVals::I(v) => OutValsMut::I(&mut v.spare_capacity_mut()[..n]),
                            OutVals::B(v) => OutValsMut::B(&mut v.spare_capacity_mut()[..n]),
                        },
                        valid: o.valid.as_deref_mut(),
                    })
                })
                .collect();
            self.run(0, n, &mut slices, None)?;
        }
        Ok(self.finish(outs))
    }

    fn finish(&self, outs: Vec<Option<Out>>) -> Vec<Series> {
        let n = self.len;
        let mut done: Vec<Series> = Vec::with_capacity(outs.len());
        for (k, o) in outs.into_iter().enumerate() {
            let r = self.roots[k];
            let s = match o {
                Some(o) => {
                    let validity = o.valid.and_then(|w| {
                        let bm = Bitmap::from_words(n, w);
                        (!bm.all_valid()).then_some(bm)
                    });
                    // SAFETY: `run` wrote every one of the `n` elements of
                    // each output (each chunk of each piece copies its
                    // root's register over its rows), and the capacity is
                    // `n`.
                    match o.vals {
                        OutVals::F(mut v) => {
                            unsafe { v.set_len(n) };
                            Series::F64 {
                                values: Arc::new(v),
                                validity,
                            }
                        }
                        OutVals::I(mut v) => {
                            unsafe { v.set_len(n) };
                            let s = Series::I64 {
                                values: Arc::new(v),
                                validity,
                            };
                            match self.steps[r].dt {
                                // `AsDate` checked the range of every Date
                                // a plan makes.
                                dt @ (DType::Date | DType::Datetime) => {
                                    Series::from_physical(dt, s).expect("a Date in range")
                                }
                                _ => s,
                            }
                        }
                        OutVals::B(mut v) => {
                            unsafe { v.set_len(n) };
                            Series::Bool {
                                values: Arc::new(v),
                                validity,
                            }
                        }
                    }
                }
                None => match &self.steps[r].kind {
                    Kind::Col(s) => self.steps[r].orig.clone().unwrap_or_else(|| s.clone()),
                    Kind::Lit(s) => broadcast(s, self.steps[r].dt, n),
                    // The same root asked for again: the first's column.
                    Kind::Op(_) => done[self.out_of[r].expect("a root")].clone(),
                },
            };
            done.push(s);
        }
        done
    }

    /// Evaluate rows `start..end`, copying the roots into `outs` (each
    /// slice covers exactly these rows) and, when `fold` is given,
    /// folding root 0 into it.
    fn run(
        &self,
        start: usize,
        end: usize,
        outs: &mut [Option<OutSlice<'_>>],
        mut fold: Option<&mut Fold>,
    ) -> Result<()> {
        let mut regs: Vec<Reg> = self.regs.iter().map(|&dt| Reg::new(dt)).collect();
        let placeholder = Arg {
            v: View::F(Src::C(0.0)),
            valid: Vw::All,
        };
        let mut s = start;
        while s < end {
            let n = CHUNK.min(end - s);
            let nw = n.div_ceil(64);
            let off = s - start;
            for i in 0..self.steps.len() {
                let Kind::Op(op) = self.steps[i].kind else {
                    continue;
                };
                let step = &self.steps[i];
                let r = self.reg[i].expect("every operation has a register");
                let out_k = self.out_of[i].filter(|&k| outs.get(k).is_some_and(|o| o.is_some()));
                // A root nothing else reads is written straight into its
                // output; anything else into its register (and a root
                // that is also read is copied out after).
                if let (true, Some(k)) = (self.direct[i], out_k) {
                    let o = outs[k].as_mut().expect("checked");
                    let mut scratch = [0u64; WORDS];
                    let has_nulls = {
                        let mut few = [placeholder; 4];
                        let many: Vec<Arg<'_>>;
                        let args: &[Arg<'_>] = if step.args.len() <= 4 {
                            for (j, &a) in step.args.iter().enumerate() {
                                few[j] = self.arg(a, s, n, &regs);
                            }
                            &few[..step.args.len()]
                        } else {
                            many = step.args.iter().map(|&a| self.arg(a, s, n, &regs)).collect();
                            &many
                        };
                        let valid: &mut [u64] = match o.valid.as_deref_mut() {
                            Some(w) => &mut w[off / 64..off / 64 + nw],
                            None => &mut scratch[..nw],
                        };
                        let vals = match &mut o.vals {
                            OutValsMut::F(v) => DVals::F(&mut v[off..off + n]),
                            OutValsMut::I(v) => DVals::I(&mut v[off..off + n]),
                            OutValsMut::B(v) => DVals::B(&mut v[off..off + n]),
                        };
                        let mut d = Dst { vals, valid };
                        exec(op, phys(step.dt), args, &mut d, n, step.may_null)?
                    };
                    if !has_nulls && let Some(w) = o.valid.as_deref_mut() {
                        fill_valid(&mut w[off / 64..off / 64 + nw], n);
                    }
                    continue;
                }
                let mut dst = std::mem::replace(&mut regs[r], Reg::placeholder());
                let has_nulls = {
                    let mut few = [placeholder; 4];
                    let many: Vec<Arg<'_>>;
                    let args: &[Arg<'_>] = if step.args.len() <= 4 {
                        for (j, &a) in step.args.iter().enumerate() {
                            few[j] = self.arg(a, s, n, &regs);
                        }
                        &few[..step.args.len()]
                    } else {
                        many = step.args.iter().map(|&a| self.arg(a, s, n, &regs)).collect();
                        &many
                    };
                    let mut d = Dst {
                        vals: dst.vals.slice(n),
                        valid: &mut dst.valid[..nw],
                    };
                    exec(op, phys(step.dt), args, &mut d, n, step.may_null)?
                };
                dst.all = !has_nulls;
                regs[r] = dst;
                if let Some(k) = out_k
                    && let Some(o) = outs.get_mut(k).and_then(|o| o.as_mut())
                {
                    copy_out(&regs[r], o, off, n);
                }
            }
            if let Some(f) = fold.as_deref_mut() {
                let root = self.roots[0];
                let a = self.arg(root, s, n, &regs);
                f.add(&a, n)?;
            }
            s += n;
        }
        Ok(())
    }

    /// Step `id`'s rows `s..s+n` as an operand.
    fn arg<'a>(&'a self, id: usize, s: usize, n: usize, regs: &'a [Reg]) -> Arg<'a> {
        let step = &self.steps[id];
        let nw = n.div_ceil(64);
        match &step.kind {
            Kind::Col(series) => {
                let valid = match series.validity() {
                    None => Vw::All,
                    Some(b) => Vw::W(&b.words()[s / 64..s / 64 + nw]),
                };
                let v = match series {
                    Series::F64 { values, .. } => View::F(Src::S(&values[s..s + n])),
                    Series::I64 { values, .. } => View::I(Src::S(&values[s..s + n])),
                    Series::Bool { values, .. } => View::B(Src::S(&values[s..s + n])),
                    Series::Str { col, .. } => View::T(TSrc::Col(col, s)),
                    Series::Datetime { values, .. } => View::I(Src::S(&values[s..s + n])),
                    Series::Date { .. } | Series::Cat { .. } => {
                        unreachable!("planned as their integers or their text")
                    }
                };
                Arg { v, valid }
            }
            Kind::Lit(sc) => {
                let v = match (sc, step.dt) {
                    (Scalar::F64(x), _) => View::F(Src::C(*x)),
                    (Scalar::I64(x), _) => View::I(Src::C(*x)),
                    (Scalar::Bool(x), _) => View::B(Src::C(*x)),
                    (Scalar::Str(t), _) => View::T(TSrc::C(t.as_str())),
                    (Scalar::Null, DType::F64) => View::F(Src::C(0.0)),
                    (Scalar::Null, DType::I64 | DType::Date | DType::Datetime) => {
                        View::I(Src::C(0))
                    }
                    (Scalar::Null, DType::Bool) => View::B(Src::C(false)),
                    (Scalar::Null, DType::Str | DType::Category) => View::T(TSrc::C("")),
                    (Scalar::Date(x), _) => View::I(Src::C(*x as i64)),
                    (Scalar::Datetime(x), _) => View::I(Src::C(*x)),
                };
                let valid = if matches!(sc, Scalar::Null) {
                    Vw::Null
                } else {
                    Vw::All
                };
                Arg { v, valid }
            }
            Kind::Op(_) => {
                let r = &regs[self.reg[id].expect("a live step has a register")];
                let valid = if r.all {
                    Vw::All
                } else {
                    Vw::W(&r.valid[..nw])
                };
                let v = match &r.vals {
                    RegVals::F(b) => View::F(Src::S(written(&b[..n]))),
                    RegVals::I(b) => View::I(Src::S(written(&b[..n]))),
                    RegVals::B(b) => View::B(Src::S(written(&b[..n]))),
                };
                Arg { v, valid }
            }
        }
    }

    /// Fold root 0 into one value without materializing it.
    pub fn reduce(&self, kind: Reduce, par: bool) -> Result<Scalar> {
        let dt = self.dtype(0);
        if matches!(dt, DType::Date | DType::Datetime) && matches!(kind, Reduce::Sum | Reduce::Mean)
        {
            return Err(OdsError::TypeMismatch(format!(
                "{} is not defined for {} series",
                if kind == Reduce::Sum { "sum" } else { "mean" },
                dt
            )));
        }
        let dense = !self.steps[self.roots[0]].may_null;
        let new_fold = || Fold::new(kind, dt, dense);
        let n = self.len;
        // Pieces of a fixed size per plan (so the partial sums, added in
        // order, are the same on any pool); a heavy plan cuts finer.
        #[cfg(feature = "parallel")]
        let piece = if self.cost >= 8 { REDUCE_PIECE / 4 } else { REDUCE_PIECE };
        #[cfg(feature = "parallel")]
        if par && n > piece {
            let pieces = n.div_ceil(piece);
            let folds: Vec<Result<Fold>> = (0..pieces)
                .into_par_iter()
                .map(|p| {
                    let mut f = new_fold();
                    let start = p * piece;
                    let end = (start + piece).min(n);
                    let mut outs: Vec<Option<OutSlice<'_>>> =
                        (0..self.roots.len()).map(|_| None).collect();
                    self.run(start, end, &mut outs, Some(&mut f))?;
                    Ok(f)
                })
                .collect();
            let mut total = new_fold();
            for f in folds {
                total.merge(f?)?;
            }
            return total.finish();
        }
        let _ = par;
        let mut f = new_fold();
        let mut outs: Vec<Option<OutSlice<'_>>> = (0..self.roots.len()).map(|_| None).collect();
        self.run(0, n, &mut outs, Some(&mut f))?;
        f.finish()
    }
}

fn broadcast(s: &Scalar, dt: DType, n: usize) -> Series {
    match (s, dt) {
        (Scalar::I64(x), DType::Date) => Series::from_dates(vec![*x as i32; n]),
        (Scalar::I64(x), DType::Datetime) => Series::from_datetimes(vec![*x; n]),
        (Scalar::F64(x), _) => Series::from_f64(vec![*x; n]),
        (Scalar::I64(x), _) => Series::from_i64(vec![*x; n]),
        (Scalar::Bool(x), _) => Series::from_bool(vec![*x; n]),
        (Scalar::Str(t), _) => Series::from_str_values(vec![t.clone(); n]),
        (Scalar::Null, DType::I64) => Series::from_i64_options(vec![None; n]),
        (Scalar::Null, DType::Bool) => Series::from_bool_options(vec![None; n]),
        (Scalar::Null, DType::Str | DType::Category) => Series::from_str_options(vec![None; n]),
        (Scalar::Null, DType::F64) => Series::from_f64_options(vec![None; n]),
        (Scalar::Null, DType::Date) => Series::from_date_options(vec![None; n]),
        (Scalar::Null, DType::Datetime) => Series::from_datetime_options(vec![None; n]),
        (Scalar::Date(x), _) => Series::from_dates(vec![*x; n]),
        (Scalar::Datetime(x), _) => Series::from_datetimes(vec![*x; n]),
    }
}

// ---------------------------------------------------------------------
// Buffers and views
// ---------------------------------------------------------------------

struct Out {
    vals: OutVals,
    valid: Option<Vec<u64>>,
}

enum OutVals {
    F(Vec<f64>),
    I(Vec<i64>),
    B(Vec<bool>),
}

enum OutValsMut<'a> {
    F(&'a mut [MaybeUninit<f64>]),
    I(&'a mut [MaybeUninit<i64>]),
    B(&'a mut [MaybeUninit<bool>]),
}

struct OutSlice<'a> {
    vals: OutValsMut<'a>,
    valid: Option<&'a mut [u64]>,
}

/// A step's buffer for one chunk. Elements are written before they are
/// read: every kernel writes all `n` of its chunk, and a register is read
/// only by later steps of the same chunk.
enum RegVals {
    F(Vec<MaybeUninit<f64>>),
    I(Vec<MaybeUninit<i64>>),
    B(Vec<MaybeUninit<bool>>),
}

impl RegVals {
    fn slice(&mut self, n: usize) -> DVals<'_> {
        match self {
            RegVals::F(v) => DVals::F(&mut v[..n]),
            RegVals::I(v) => DVals::I(&mut v[..n]),
            RegVals::B(v) => DVals::B(&mut v[..n]),
        }
    }
}

/// The first `n` elements of a written buffer as values.
#[inline]
fn written<T>(s: &[MaybeUninit<T>]) -> &[T] {
    // SAFETY: callers pass only the part of a register its step wrote in
    // this chunk (see `RegVals`); `MaybeUninit<T>` has `T`'s layout.
    unsafe { &*(s as *const [MaybeUninit<T>] as *const [T]) }
}

struct Reg {
    vals: RegVals,
    valid: [u64; WORDS],
    all: bool,
}

impl Reg {
    fn new(dt: DType) -> Reg {
        Reg {
            vals: match dt {
                DType::F64 => RegVals::F(vec![MaybeUninit::uninit(); CHUNK]),
                DType::I64 => RegVals::I(vec![MaybeUninit::uninit(); CHUNK]),
                DType::Bool => RegVals::B(vec![MaybeUninit::uninit(); CHUNK]),
                _ => unreachable!("no text registers"),
            },
            valid: [0; WORDS],
            all: true,
        }
    }

    fn placeholder() -> Reg {
        Reg {
            vals: RegVals::F(Vec::new()),
            valid: [0; WORDS],
            all: true,
        }
    }
}

fn copy_out(r: &Reg, o: &mut OutSlice<'_>, off: usize, n: usize) {
    match (&r.vals, &mut o.vals) {
        (RegVals::F(a), OutValsMut::F(b)) => b[off..off + n].copy_from_slice(&a[..n]),
        (RegVals::I(a), OutValsMut::I(b)) => b[off..off + n].copy_from_slice(&a[..n]),
        (RegVals::B(a), OutValsMut::B(b)) => b[off..off + n].copy_from_slice(&a[..n]),
        _ => unreachable!("a root's register has its dtype"),
    }
    if let Some(w) = o.valid.as_deref_mut() {
        let nw = n.div_ceil(64);
        let w = &mut w[off / 64..off / 64 + nw];
        if r.all {
            fill_valid(w, n);
        } else {
            w.copy_from_slice(&r.valid[..nw]);
        }
    }
}

/// Every position of `n` valid (tail bits clear).
fn fill_valid(w: &mut [u64], n: usize) {
    for (i, x) in w.iter_mut().enumerate() {
        *x = word_mask(i, n);
    }
}

#[inline]
fn word_mask(i: usize, n: usize) -> u64 {
    let lo = i * 64;
    if lo + 64 <= n {
        u64::MAX
    } else {
        (1u64 << (n - lo)) - 1
    }
}

/// An operand's values over a chunk: a slice, or a constant.
#[derive(Clone, Copy)]
enum Src<'a, T> {
    S(&'a [T]),
    C(T),
}

impl<T: Copy> Src<'_, T> {
    #[inline]
    fn at(&self, i: usize) -> T {
        match self {
            Src::S(s) => s[i],
            Src::C(c) => *c,
        }
    }
}

#[derive(Clone, Copy)]
enum TSrc<'a> {
    Col(&'a StrCol, usize),
    C(&'a str),
}

impl TSrc<'_> {
    #[inline]
    fn at(&self, i: usize) -> &str {
        match self {
            TSrc::Col(c, s) => c.get(s + i),
            TSrc::C(t) => t,
        }
    }
}

#[derive(Clone, Copy)]
enum View<'a> {
    F(Src<'a, f64>),
    I(Src<'a, i64>),
    B(Src<'a, bool>),
    T(TSrc<'a>),
}

/// An operand's validity over a chunk.
#[derive(Clone, Copy)]
enum Vw<'a> {
    All,
    /// Every position null (a null literal).
    Null,
    W(&'a [u64]),
}

impl Vw<'_> {
    #[inline]
    fn word(&self, i: usize, n: usize) -> u64 {
        match self {
            Vw::All => word_mask(i, n),
            Vw::Null => 0,
            Vw::W(w) => w[i],
        }
    }

    #[inline]
    fn get(&self, i: usize) -> bool {
        match self {
            Vw::All => true,
            Vw::Null => false,
            Vw::W(w) => w[i / 64] >> (i % 64) & 1 == 1,
        }
    }
}

#[derive(Clone, Copy)]
struct Arg<'a> {
    v: View<'a>,
    valid: Vw<'a>,
}

enum DVals<'a> {
    F(&'a mut [MaybeUninit<f64>]),
    I(&'a mut [MaybeUninit<i64>]),
    B(&'a mut [MaybeUninit<bool>]),
}

struct Dst<'a> {
    vals: DVals<'a>,
    valid: &'a mut [u64],
}

// ---------------------------------------------------------------------
// Kernels
// ---------------------------------------------------------------------

/// `out[i] = f(a[i], b[i])` for slices or constants on either side;
/// generic, so each operator is its own loop and vectorizes.
#[inline(always)]
fn map2<A: Copy, B: Copy, R>(
    a: Src<A>,
    b: Src<B>,
    out: &mut [MaybeUninit<R>],
    mut f: impl FnMut(A, B) -> R,
) {
    let n = out.len();
    match (a, b) {
        (Src::S(x), Src::S(y)) => {
            let (x, y) = (&x[..n], &y[..n]);
            for i in 0..n {
                out[i] = MaybeUninit::new(f(x[i], y[i]));
            }
        }
        (Src::S(x), Src::C(c)) => {
            let x = &x[..n];
            for i in 0..n {
                out[i] = MaybeUninit::new(f(x[i], c));
            }
        }
        (Src::C(c), Src::S(y)) => {
            let y = &y[..n];
            for i in 0..n {
                out[i] = MaybeUninit::new(f(c, y[i]));
            }
        }
        (Src::C(c), Src::C(d)) => {
            for o in out.iter_mut() {
                *o = MaybeUninit::new(f(c, d));
            }
        }
    }
}

#[inline(always)]
fn map1<A: Copy, R>(a: Src<A>, out: &mut [MaybeUninit<R>], mut f: impl FnMut(A) -> R) {
    match a {
        Src::S(x) => {
            let x = &x[..out.len()];
            for (o, &v) in out.iter_mut().zip(x) {
                *o = MaybeUninit::new(f(v));
            }
        }
        Src::C(c) => {
            for o in out.iter_mut() {
                *o = MaybeUninit::new(f(c));
            }
        }
    }
}

/// Whether `bad` holds anywhere in an operand (a branch-free scan the
/// compiler vectorizes; `any_valid` then asks where it counts).
#[inline(always)]
fn any_of<T: Copy>(a: Src<T>, n: usize, bad: impl Fn(T) -> bool) -> bool {
    match a {
        Src::S(x) => x[..n].iter().fold(false, |z, &v| z | bad(v)),
        Src::C(c) => n > 0 && bad(c),
    }
}

/// Combine the operands' validity with AND into `out`; whether any
/// position is null.
fn and_valid(args: &[Arg<'_>], out: &mut [u64], n: usize) -> bool {
    if args.iter().all(|a| matches!(a.valid, Vw::All)) {
        return false;
    }
    let mut any_null = false;
    for (i, w) in out.iter_mut().enumerate() {
        let mut x = word_mask(i, n);
        for a in args {
            x &= a.valid.word(i, n);
        }
        any_null |= x != word_mask(i, n);
        *w = x;
    }
    any_null
}

/// Whether any valid position (by `valid`) satisfies `bad`.
fn any_valid(valid: &[u64], n: usize, mut bad: impl FnMut(usize) -> bool) -> bool {
    (0..n).any(|i| valid[i / 64] >> (i % 64) & 1 == 1 && bad(i))
}

fn f_of<'a>(a: &Arg<'a>) -> Src<'a, f64> {
    match a.v {
        View::F(s) => s,
        _ => unreachable!("planned as Float"),
    }
}

fn i_of<'a>(a: &Arg<'a>) -> Src<'a, i64> {
    match a.v {
        View::I(s) => s,
        _ => unreachable!("planned as Int"),
    }
}

fn b_of<'a>(a: &Arg<'a>) -> Src<'a, bool> {
    match a.v {
        View::B(s) => s,
        _ => unreachable!("planned as Bool"),
    }
}

/// Run one operation over a chunk. Returns whether the result holds a
/// null (its validity is in `dst.valid` when it does).
fn exec(op: Op, dt: DType, args: &[Arg<'_>], dst: &mut Dst<'_>, n: usize, may_null: bool) -> Result<bool> {
    let nw = n.div_ceil(64);
    match op {
        Op::Bin(b) if b.is_logic() => return logic(b, args, dst, n),
        Op::Where => return where_(args, dst, n),
        Op::Coalesce => return coalesce(args, dst, n),
        Op::IsNull | Op::NotNull => {
            let DVals::B(out) = &mut dst.vals else {
                unreachable!()
            };
            let want = matches!(op, Op::NotNull);
            match args[0].valid {
                Vw::All => out.fill(MaybeUninit::new(want)),
                Vw::Null => out.fill(MaybeUninit::new(!want)),
                Vw::W(words) => {
                    for (w, &bits) in words.iter().enumerate() {
                        let lo = w * 64;
                        let hi = (lo + 64).min(n);
                        for (j, o) in out[lo..hi].iter_mut().enumerate() {
                            *o = MaybeUninit::new(((bits >> j) & 1 == 1) == want);
                        }
                    }
                }
            }
            return Ok(false);
        }
        _ => {}
    }
    // Everything below propagates nulls: the result is null where any
    // operand is (a clip or between bound is open, not null).
    let has_nulls = if !may_null {
        false
    } else {
        match op {
            Op::Clip | Op::Between(_) => and_valid(&args[..1], dst.valid, n),
            _ => and_valid(args, dst.valid, n),
        }
    };
    let mut vbuf = [u64::MAX; WORDS];
    if has_nulls {
        vbuf[..nw].copy_from_slice(&dst.valid[..nw]);
    }
    let valid: &[u64] = &vbuf[..nw];
    match op {
        Op::Bin(b) if b.is_cmp() => compare(b, args, dst),
        Op::Bin(b) => match dt {
            DType::F64 => f_arith(b, args, dst, valid, n)?,
            DType::I64 => i_arith(b, args, dst, valid, n)?,
            _ => unreachable!("arithmetic is numeric"),
        },
        Op::Neg => match &mut dst.vals {
            DVals::F(out) => map1(f_of(&args[0]), out, |x| -x),
            DVals::I(out) => {
                let a = i_of(&args[0]);
                if any_of(a, n, |x| x == i64::MIN) && any_valid(valid, n, |i| a.at(i) == i64::MIN) {
                    return Err(OdsError::IntegerOverflow("negation"));
                }
                map1(a, out, i64::wrapping_neg);
            }
            _ => unreachable!(),
        },
        Op::Math(m) => match &mut dst.vals {
            DVals::F(out) => {
                let a = f_of(&args[0]);
                if let Some(msg) = domain_error(m, a, valid, n) {
                    return Err(OdsError::InvalidArgument(msg.to_string()));
                }
                math_f(m, a, out);
            }
            DVals::I(out) => {
                let a = i_of(&args[0]);
                match m {
                    Math::Abs => {
                        if any_of(a, n, |x| x == i64::MIN)
                            && any_valid(valid, n, |i| a.at(i) == i64::MIN)
                        {
                            return Err(OdsError::IntegerOverflow("absolute value"));
                        }
                        map1(a, out, i64::wrapping_abs);
                    }
                    Math::Sign => map1(a, out, i64::signum),
                    _ => map1(a, out, |x| x),
                }
            }
            _ => unreachable!(),
        },
        Op::Round(d) => match &mut dst.vals {
            DVals::F(out) => {
                let a = f_of(&args[0]);
                if d == 0 {
                    map1(a, out, f64::round);
                } else if d > 0 {
                    let s = 10f64.powi(d);
                    map1(a, out, |x| {
                        let r = (x * s).round() / s;
                        if r.is_finite() { r } else { x }
                    });
                } else {
                    let s = 10f64.powi(-d);
                    map1(a, out, |x| (x / s).round() * s);
                }
            }
            DVals::I(out) => {
                let a = i_of(&args[0]);
                if d >= 0 {
                    map1(a, out, |x| x);
                } else {
                    let s = 10i128.checked_pow((-d) as u32).unwrap_or(i128::MAX);
                    let mut bad = false;
                    map1(a, out, |x| {
                        let x = x as i128;
                        let half = s / 2;
                        let r = if x >= 0 {
                            (x + half) / s * s
                        } else {
                            -((-x + half) / s * s)
                        };
                        if r > i64::MAX as i128 || r < i64::MIN as i128 {
                            bad = true;
                            0
                        } else {
                            r as i64
                        }
                    });
                    if bad {
                        return Err(OdsError::IntegerOverflow("round"));
                    }
                }
            }
            _ => unreachable!(),
        },
        Op::IsNan => {
            let DVals::B(out) = &mut dst.vals else {
                unreachable!()
            };
            map1(f_of(&args[0]), out, f64::is_nan);
        }
        Op::Not => {
            let DVals::B(out) = &mut dst.vals else {
                unreachable!()
            };
            map1(b_of(&args[0]), out, |x| !x);
        }
        Op::Clip => clip(args, dst, n),
        Op::Between(c) => between(c, args, dst, n),
        Op::Cast(to) => {
            if let Some(nulls) = cast(to, &args[0], dst, n, has_nulls) {
                return Ok(nulls);
            }
        }
        Op::Temporal(t, micros) => temporal_op(t, micros, args, dst, valid, n)?,
        Op::Where | Op::Coalesce | Op::IsNull | Op::NotNull => {
            unreachable!("handled above")
        }
    }
    Ok(has_nulls)
}

/// A calendar operation over a chunk of integers (days, or
/// microseconds when `micros`).
fn temporal_op(t: TOp, micros: bool, args: &[Arg<'_>], dst: &mut Dst<'_>, valid: &[u64], n: usize) -> Result<()> {
    use crate::temporal as tm;
    let x = i_of(&args[0]);
    if let DVals::B(out) = &mut dst.vals {
        debug_assert_eq!(t, TOp::IsWeekend);
        if micros {
            map1(x, out, |v| tm::iso_weekday(tm::day_of_us(v)) >= 6);
        } else {
            map1(x, out, |v| tm::iso_weekday(v) >= 6);
        }
        return Ok(());
    }
    let DVals::I(out) = &mut dst.vals else {
        unreachable!("calendar answers are Int or Bool")
    };
    // Each component as its own loop over days, so it inlines and the
    // compiler sees one straight-line body per element.
    macro_rules! by_day {
        ($f:expr) => {{
            let f = $f;
            if micros {
                map1(x, out, |v| f(tm::day_of_us(v)) as i64)
            } else {
                map1(x, out, |v| f(v) as i64)
            }
        }};
    }
    match t {
        TOp::Year => by_day!(tm::year_of),
        TOp::Quarter => by_day!(|z| (tm::civil_from_days(z).1 - 1) / 3 + 1),
        TOp::Month => by_day!(|z| tm::civil_from_days(z).1),
        TOp::Day => by_day!(|z| tm::civil_from_days(z).2),
        TOp::Weekday => by_day!(tm::iso_weekday),
        TOp::DayOfYear => by_day!(tm::day_of_year),
        TOp::Week => by_day!(tm::iso_week),
        TOp::Hour => map1(x, out, |v| tm::time_of_us(v) / tm::US_PER_HOUR),
        TOp::Minute => map1(x, out, |v| tm::time_of_us(v) / tm::US_PER_MIN % 60),
        TOp::Second => map1(x, out, |v| tm::time_of_us(v) / tm::US_PER_SEC % 60),
        TOp::Date => {
            if micros {
                map1(x, out, tm::day_of_us)
            } else {
                map1(x, out, |v| v)
            }
        }
        TOp::ToDatetime => {
            if micros {
                map1(x, out, |v| v)
            } else {
                map1(x, out, |v| v * tm::US_PER_DAY)
            }
        }
        TOp::Raw | TOp::AsDatetime => map1(x, out, |v| v),
        TOp::AsDate => {
            let bad = |v: i64| i32::try_from(v).is_err();
            if any_of(x, n, bad) && any_valid(valid, n, |i| bad(x.at(i))) {
                return Err(OdsError::InvalidArgument(
                    "a day past the dates a Date holds".to_string(),
                ));
            }
            map1(x, out, |v| v)
        }
        TOp::Floor(e) => {
            if micros {
                map1(x, out, |v| e.floor_us(v))
            } else {
                map1(x, out, |v| e.floor_day(v))
            }
        }
        TOp::AddMonths(k) => {
            let mut bad = false;
            let lim = |d: i64| i32::try_from(d).is_ok();
            if micros {
                map1(x, out, |v| {
                    match tm::add_months(tm::day_of_us(v), k).filter(|&d| lim(d)) {
                        Some(d) => d * tm::US_PER_DAY + tm::time_of_us(v),
                        None => {
                            bad = true;
                            0
                        }
                    }
                })
            } else {
                map1(x, out, |v| match tm::add_months(v, k).filter(|&d| lim(d)) {
                    Some(d) => d,
                    None => {
                        bad = true;
                        0
                    }
                })
            }
            if bad
                && any_valid(valid, n, |i| {
                    let v = x.at(i);
                    let d = if micros { tm::day_of_us(v) } else { v };
                    tm::add_months(d, k).filter(|&d| lim(d)).is_none()
                })
            {
                return Err(OdsError::InvalidArgument(
                    "add_months: past the dates a Date holds".to_string(),
                ));
            }
        }
        TOp::MonthsBetween => {
            let y = i_of(&args[1]);
            if micros {
                map2(x, y, out, |a, b| tm::months_between(tm::day_of_us(a), tm::day_of_us(b)))
            } else {
                map2(x, y, out, tm::months_between)
            }
        }
        TOp::IsWeekend => unreachable!("a Bool answer"),
    }
    Ok(())
}

/// The domain error a valid out-of-range argument raises, if any: one
/// vectorized scan, and the exact positions only when it finds one.
fn domain_error(m: Math, a: Src<f64>, valid: &[u64], n: usize) -> Option<&'static str> {
    let (_, msg) = m.domain()?;
    let found = match m {
        Math::Sqrt => any_of(a, n, |x| x < 0.0),
        Math::Ln | Math::Log2 | Math::Log10 => any_of(a, n, |x| x <= 0.0),
        Math::Ln1p => any_of(a, n, |x| x <= -1.0),
        Math::Asin | Math::Acos => any_of(a, n, |x| !(-1.0..=1.0).contains(&x)),
        Math::Logit => any_of(a, n, |x| x <= 0.0 || x >= 1.0),
        _ => false,
    };
    let (bad, _) = m.domain()?;
    (found && any_valid(valid, n, |i| bad(a.at(i)))).then_some(msg)
}

fn math_f(m: Math, a: Src<f64>, out: &mut [MaybeUninit<f64>]) {
    // The cheap functions inline into their loop; the rest are calls
    // whatever the loop looks like.
    match m {
        Math::Abs => map1(a, out, f64::abs),
        Math::Floor => map1(a, out, f64::floor),
        Math::Ceil => map1(a, out, f64::ceil),
        Math::Trunc => map1(a, out, f64::trunc),
        Math::Fract => map1(a, out, f64::fract),
        Math::Sqrt => map1(a, out, f64::sqrt),
        Math::Sign => map1(a, out, |x| {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                x
            }
        }),
        Math::Degrees => map1(a, out, f64::to_degrees),
        Math::Radians => map1(a, out, f64::to_radians),
        Math::Exp => map1(a, out, f64::exp),
        Math::Ln => map1(a, out, f64::ln),
        Math::Log2 => map1(a, out, f64::log2),
        Math::Log10 => map1(a, out, f64::log10),
        Math::Sigmoid => map1(a, out, sigmoid),
        _ => map1(a, out, m.f()),
    }
}

fn f_arith(b: BinOp, args: &[Arg<'_>], dst: &mut Dst<'_>, valid: &[u64], n: usize) -> Result<()> {
    let DVals::F(out) = &mut dst.vals else {
        unreachable!()
    };
    let (x, y) = (f_of(&args[0]), f_of(&args[1]));
    match b {
        BinOp::Add => map2(x, y, out, |a, b| a + b),
        BinOp::Sub => map2(x, y, out, |a, b| a - b),
        BinOp::Mul => map2(x, y, out, |a, b| a * b),
        BinOp::Pow => map2(x, y, out, f64::powf),
        BinOp::Div | BinOp::Mod => {
            // The language raises on a zero divisor; only positions that
            // compute (every operand valid) are checked.
            if any_of(y, n, |v| v == 0.0) && any_valid(valid, n, |i| y.at(i) == 0.0) {
                return Err(if b == BinOp::Div {
                    OdsError::DivisionByZero
                } else {
                    OdsError::InvalidArgument("Modulo by zero".to_string())
                });
            }
            if b == BinOp::Div {
                map2(x, y, out, |a, b| a / b);
            } else {
                map2(x, y, out, |a, b| a % b);
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn i_arith(b: BinOp, args: &[Arg<'_>], dst: &mut Dst<'_>, valid: &[u64], n: usize) -> Result<()> {
    let DVals::I(out) = &mut dst.vals else {
        unreachable!()
    };
    let (x, y) = (i_of(&args[0]), i_of(&args[1]));
    let mut bad = false;
    match b {
        BinOp::Add => map2(x, y, out, |a, b| {
            let (r, o) = a.overflowing_add(b);
            bad |= o;
            r
        }),
        BinOp::Sub => map2(x, y, out, |a, b| {
            let (r, o) = a.overflowing_sub(b);
            bad |= o;
            r
        }),
        BinOp::Mul => map2(x, y, out, |a, b| {
            let (r, o) = a.overflowing_mul(b);
            bad |= o;
            r
        }),
        BinOp::Mod => {
            if any_of(y, n, |v| v == 0) && any_valid(valid, n, |i| y.at(i) == 0) {
                return Err(OdsError::InvalidArgument("Modulo by zero".to_string()));
            }
            map2(x, y, out, |a, b| {
                if b == 0 {
                    0
                } else {
                    let (r, o) = a.overflowing_rem(b);
                    bad |= o;
                    r
                }
            });
        }
        _ => unreachable!("Div and Pow compute in Float"),
    }
    if bad {
        let check = |f: fn(i64, i64) -> Option<i64>| {
            any_valid(valid, n, |i| f(x.at(i), y.at(i)).is_none())
        };
        let overflowed = match b {
            BinOp::Add => check(i64::checked_add),
            BinOp::Sub => check(i64::checked_sub),
            BinOp::Mul => check(i64::checked_mul),
            _ => check(|a, b| if b == 0 { Some(0) } else { a.checked_rem(b) }),
        };
        if overflowed {
            return Err(OdsError::IntegerOverflow(b.word()));
        }
    }
    Ok(())
}

fn compare(b: BinOp, args: &[Arg<'_>], dst: &mut Dst<'_>) {
    let DVals::B(out) = &mut dst.vals else {
        unreachable!()
    };
    macro_rules! cmp {
        ($x:expr, $y:expr) => {
            match b {
                BinOp::Eq => map2($x, $y, out, |a, b| a == b),
                BinOp::Ne => map2($x, $y, out, |a, b| a != b),
                BinOp::Lt => map2($x, $y, out, |a, b| a < b),
                BinOp::Le => map2($x, $y, out, |a, b| a <= b),
                BinOp::Gt => map2($x, $y, out, |a, b| a > b),
                BinOp::Ge => map2($x, $y, out, |a, b| a >= b),
                _ => unreachable!(),
            }
        };
    }
    match (args[0].v, args[1].v) {
        (View::F(x), View::F(y)) => cmp!(x, y),
        (View::I(x), View::I(y)) => cmp!(x, y),
        (View::B(x), View::B(y)) => cmp!(x, y),
        (View::T(x), View::T(y)) => {
            for (i, o) in out.iter_mut().enumerate() {
                let (a, c) = (x.at(i), y.at(i));
                *o = MaybeUninit::new(match b {
                    BinOp::Eq => a == c,
                    BinOp::Ne => a != c,
                    BinOp::Lt => a < c,
                    BinOp::Le => a <= c,
                    BinOp::Gt => a > c,
                    _ => a >= c,
                });
            }
        }
        _ => unreachable!("planned to a common type"),
    }
}

/// Kleene logic: a known `false` settles `&`, a known `true` settles
/// `|`; `^` is null where either side is.
fn logic(b: BinOp, args: &[Arg<'_>], dst: &mut Dst<'_>, n: usize) -> Result<bool> {
    let (x, y) = (b_of(&args[0]), b_of(&args[1]));
    let (vx, vy) = (args[0].valid, args[1].valid);
    let DVals::B(out) = &mut dst.vals else {
        unreachable!()
    };
    let dense = matches!(vx, Vw::All) && matches!(vy, Vw::All);
    if dense || b == BinOp::Xor {
        match b {
            BinOp::And => map2(x, y, out, |a, b| a & b),
            BinOp::Or => map2(x, y, out, |a, b| a | b),
            _ => map2(x, y, out, |a, b| a ^ b),
        }
        if dense {
            return Ok(false);
        }
        return Ok(and_valid(args, dst.valid, n));
    }
    let mut any_null = false;
    for (i, o) in out.iter_mut().enumerate() {
        let (a, va, c, vc) = (x.at(i), vx.get(i), y.at(i), vy.get(i));
        let (v, ok) = if b == BinOp::And {
            if (va && !a) || (vc && !c) {
                (false, true)
            } else {
                (true, va && vc)
            }
        } else if (va && a) || (vc && c) {
            (true, true)
        } else {
            (false, va && vc)
        };
        *o = MaybeUninit::new(v);
        if i % 64 == 0 {
            dst.valid[i / 64] = 0;
        }
        if ok {
            dst.valid[i / 64] |= 1 << (i % 64);
        } else {
            any_null = true;
        }
    }
    Ok(any_null)
}

/// A constant operand spread over a buffer, a slice operand as itself:
/// the loops below then index plain slices and vectorize.
#[inline(always)]
fn plain<'a, T: Copy>(s: Src<'a, T>, buf: &'a mut [T]) -> &'a [T] {
    match s {
        Src::S(x) => &x[..buf.len()],
        Src::C(c) => {
            buf.fill(c);
            buf
        }
    }
}

/// The Bool values of a chunk as validity-shaped words.
fn pack(c: &[bool], w: usize) -> u64 {
    let lo = w * 64;
    let hi = (lo + 64).min(c.len());
    let mut x = 0u64;
    for (j, &b) in c[lo..hi].iter().enumerate() {
        x |= (b as u64) << j;
    }
    x
}

fn where_(args: &[Arg<'_>], dst: &mut Dst<'_>, n: usize) -> Result<bool> {
    let mut cb = [false; CHUNK];
    let c = plain(b_of(&args[0]), &mut cb[..n]);
    macro_rules! pick {
        ($out:expr, $get:ident) => {{
            let out = &mut $out[..n];
            match ($get(&args[1]), $get(&args[2])) {
                (Src::S(a), Src::S(b)) => {
                    let (a, b) = (&a[..n], &b[..n]);
                    for i in 0..n {
                        out[i] = MaybeUninit::new(if c[i] { a[i] } else { b[i] });
                    }
                }
                (Src::S(a), Src::C(b)) => {
                    let a = &a[..n];
                    for i in 0..n {
                        out[i] = MaybeUninit::new(if c[i] { a[i] } else { b });
                    }
                }
                (Src::C(a), Src::S(b)) => {
                    let b = &b[..n];
                    for i in 0..n {
                        out[i] = MaybeUninit::new(if c[i] { a } else { b[i] });
                    }
                }
                (Src::C(a), Src::C(b)) => {
                    for i in 0..n {
                        out[i] = MaybeUninit::new(if c[i] { a } else { b });
                    }
                }
            }
        }};
    }
    match &mut dst.vals {
        DVals::F(out) => pick!(out, f_of),
        DVals::I(out) => pick!(out, i_of),
        DVals::B(out) => pick!(out, b_of),
    }
    if args.iter().all(|a| matches!(a.valid, Vw::All)) {
        return Ok(false);
    }
    let mut any_null = false;
    let (vc, va, vb) = (args[0].valid, args[1].valid, args[2].valid);
    for w in 0..n.div_ceil(64) {
        let cw = pack(c, w);
        let x = vc.word(w, n) & ((cw & va.word(w, n)) | (!cw & vb.word(w, n))) & word_mask(w, n);
        any_null |= x != word_mask(w, n);
        dst.valid[w] = x;
    }
    Ok(any_null)
}

fn coalesce(args: &[Arg<'_>], dst: &mut Dst<'_>, n: usize) -> Result<bool> {
    // The last operand everywhere, then each earlier one over it where
    // it is valid, a word of validity at a time: a full word copies the
    // block, an empty one skips it, a mixed one selects branch-free.
    let nw = n.div_ceil(64);
    let last = args.len() - 1;
    macro_rules! fill {
        ($out:expr, $get:ident, $zero:expr) => {{
            let mut buf = [$zero; CHUNK];
            let buf = &mut buf[..n];
            match $get(&args[last]) {
                Src::S(x) => buf.copy_from_slice(&x[..n]),
                Src::C(c) => buf.fill(c),
            }
            for a in args[..last].iter().rev() {
                let s = $get(a);
                match (a.valid, s) {
                    (Vw::Null, _) => {}
                    (Vw::All, Src::S(x)) => buf.copy_from_slice(&x[..n]),
                    (Vw::All, Src::C(c)) => buf.fill(c),
                    (Vw::W(words), _) => {
                        let mut cb = [$zero; 64];
                        for (w, &bits) in words[..nw].iter().enumerate() {
                            let lo = w * 64;
                            let hi = (lo + 64).min(n);
                            if bits == 0 {
                                continue;
                            }
                            let x: &[_] = match s {
                                Src::S(x) => &x[lo..hi],
                                Src::C(c) => {
                                    cb.fill(c);
                                    &cb[..hi - lo]
                                }
                            };
                            if bits == u64::MAX {
                                buf[lo..hi].copy_from_slice(x);
                            } else {
                                for (j, (o, &v)) in buf[lo..hi].iter_mut().zip(x).enumerate() {
                                    *o = if (bits >> j) & 1 == 1 { v } else { *o };
                                }
                            }
                        }
                    }
                }
            }
            for (o, &v) in $out[..n].iter_mut().zip(buf.iter()) {
                *o = MaybeUninit::new(v);
            }
        }};
    }
    match &mut dst.vals {
        DVals::F(out) => fill!(out, f_of, 0.0f64),
        DVals::I(out) => fill!(out, i_of, 0i64),
        DVals::B(out) => fill!(out, b_of, false),
    }
    if args.iter().any(|a| matches!(a.valid, Vw::All)) {
        return Ok(false);
    }
    let mut any_null = false;
    for w in 0..nw {
        let mut x = 0;
        for a in args {
            x |= a.valid.word(w, n);
        }
        any_null |= x != word_mask(w, n);
        dst.valid[w] = x;
    }
    Ok(any_null)
}

fn clip(args: &[Arg<'_>], dst: &mut Dst<'_>, n: usize) {
    let (vlo, vhi) = (args[1].valid, args[2].valid);
    macro_rules! go {
        ($out:expr, $get:ident) => {{
            let out = &mut $out[..n];
            let x = $get(&args[0]);
            let (lo, hi) = ($get(&args[1]), $get(&args[2]));
            // Low bound first, then high: a crossed pair gives the high
            // one (NumPy's clip). An open bound clips nothing.
            match (lo, hi, vlo, vhi) {
                (Src::C(l), Src::C(h), Vw::All, Vw::All) => map1(x, out, |v| {
                    let v = if v < l { l } else { v };
                    if v > h { h } else { v }
                }),
                (Src::C(l), _, Vw::All, Vw::Null) => map1(x, out, |v| if v < l { l } else { v }),
                (_, Src::C(h), Vw::Null, Vw::All) => map1(x, out, |v| if v > h { h } else { v }),
                (_, _, Vw::Null, Vw::Null) => map1(x, out, |v| v),
                _ => {
                    for (i, o) in out.iter_mut().enumerate() {
                        let mut v = x.at(i);
                        if vlo.get(i) && v < lo.at(i) {
                            v = lo.at(i);
                        }
                        if vhi.get(i) && v > hi.at(i) {
                            v = hi.at(i);
                        }
                        *o = MaybeUninit::new(v);
                    }
                }
            }
        }};
    }
    match &mut dst.vals {
        DVals::F(out) => go!(out, f_of),
        DVals::I(out) => go!(out, i_of),
        DVals::B(_) => unreachable!(),
    }
}

fn between(c: Closed, args: &[Arg<'_>], dst: &mut Dst<'_>, n: usize) {
    let DVals::B(out) = &mut dst.vals else {
        unreachable!()
    };
    let out = &mut out[..n];
    let (vlo, vhi) = (args[1].valid, args[2].valid);
    let (lc, hc) = match c {
        Closed::Both => (true, true),
        Closed::Left => (true, false),
        Closed::Right => (false, true),
        Closed::Neither => (false, false),
    };
    macro_rules! go {
        ($get:ident) => {{
            let x = $get(&args[0]);
            let (lo, hi) = ($get(&args[1]), $get(&args[2]));
            match (lo, hi, vlo, vhi) {
                // Constant bounds: one comparison pair a row, no branch.
                (Src::C(l), Src::C(h), Vw::All | Vw::Null, Vw::All | Vw::Null) => {
                    let ol = matches!(vlo, Vw::Null);
                    let oh = matches!(vhi, Vw::Null);
                    macro_rules! with {
                        ($above:expr, $below:expr) => {
                            map1(x, out, |v| (ol | $above(v)) & (oh | $below(v)))
                        };
                    }
                    match (lc, hc) {
                        (true, true) => with!(|v| v >= l, |v| v <= h),
                        (true, false) => with!(|v| v >= l, |v| v < h),
                        (false, true) => with!(|v| v > l, |v| v <= h),
                        (false, false) => with!(|v| v > l, |v| v < h),
                    }
                }
                _ => {
                    for (i, o) in out.iter_mut().enumerate() {
                        let v = x.at(i);
                        let above = !vlo.get(i) || if lc { v >= lo.at(i) } else { v > lo.at(i) };
                        let below = !vhi.get(i) || if hc { v <= hi.at(i) } else { v < hi.at(i) };
                        *o = MaybeUninit::new(above && below);
                    }
                }
            }
        }};
    }
    match args[0].v {
        View::F(_) => go!(f_of),
        View::I(_) => go!(i_of),
        View::T(x) => {
            let (View::T(lo), View::T(hi)) = (args[1].v, args[2].v) else {
                unreachable!()
            };
            for (i, o) in out.iter_mut().enumerate() {
                let v = x.at(i);
                let above = !vlo.get(i) || if lc { v >= lo.at(i) } else { v > lo.at(i) };
                let below = !vhi.get(i) || if hc { v <= hi.at(i) } else { v < hi.at(i) };
                *o = MaybeUninit::new(above && below);
            }
        }
        View::B(_) => unreachable!(),
    }
}

/// A numeric cast. Float to Int can make nulls, so it returns its own
/// null answer; the others propagate (None).
fn cast(to: DType, a: &Arg<'_>, dst: &mut Dst<'_>, n: usize, has_nulls: bool) -> Option<bool> {
    let _ = to;
    match (a.v, &mut dst.vals) {
        (View::I(x), DVals::F(out)) => map1(x, out, |v| v as f64),
        (View::B(x), DVals::F(out)) => map1(x, out, |v| if v { 1.0 } else { 0.0 }),
        (View::B(x), DVals::I(out)) => map1(x, out, |v| v as i64),
        (View::I(x), DVals::B(out)) => map1(x, out, |v| v != 0),
        (View::F(x), DVals::I(out)) => {
            let fits = |t: f64| t.is_finite() && t >= i64::MIN as f64 && t <= i64::MAX as f64;
            let lost = any_of(x, n, |v| !fits(v.trunc()));
            map1(x, out, |v| {
                let t = v.trunc();
                if fits(t) { t as i64 } else { 0 }
            });
            if !lost {
                return Some(has_nulls);
            }
            if !has_nulls {
                fill_valid(dst.valid, n);
            }
            for i in 0..n {
                if !fits(x.at(i).trunc()) {
                    dst.valid[i / 64] &= !(1u64 << (i % 64));
                }
            }
            let any_null = (0..n.div_ceil(64)).any(|w| dst.valid[w] != word_mask(w, n));
            return Some(any_null);
        }
        _ => unreachable!("planned casts are numeric"),
    }
    None
}

// ---------------------------------------------------------------------
// Reductions
// ---------------------------------------------------------------------

/// What [`Plan::reduce`] folds a root into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduce {
    /// Float: the eager `sum`'s order; Int: checked; Bool: the count of
    /// true.
    Sum,
    /// `sum / count`; a Bool mask's share of true.
    Mean,
    Min,
    Max,
    /// Valid positions.
    Count,
}

struct Fold {
    kind: Reduce,
    dt: DType,
    dense: bool,
    acc8: [f64; 8],
    rem: f64,
    acc: f64,
    lanes: [f64; 4],
    k: usize,
    isum: i64,
    count: usize,
    fbest: Option<f64>,
    ibest: Option<i64>,
}

impl Fold {
    fn new(kind: Reduce, dt: DType, dense: bool) -> Fold {
        Fold {
            kind,
            dt,
            dense,
            acc8: [0.0; 8],
            rem: 0.0,
            acc: 0.0,
            lanes: [0.0; 4],
            k: 0,
            isum: 0,
            count: 0,
            fbest: None,
            ibest: None,
        }
    }

    fn add(&mut self, a: &Arg<'_>, n: usize) -> Result<()> {
        let nw = n.div_ceil(64);
        let count = match a.valid {
            Vw::All => n,
            Vw::Null => 0,
            Vw::W(w) => w[..nw].iter().map(|x| x.count_ones() as usize).sum(),
        };
        self.count += count;
        if self.kind == Reduce::Count {
            return Ok(());
        }
        let want_min = self.kind == Reduce::Min;
        match a.v {
            View::F(src) => {
                let buf;
                let x: &[f64] = match src {
                    Src::S(s) => &s[..n],
                    Src::C(c) => {
                        buf = vec![c; n];
                        &buf
                    }
                };
                match self.kind {
                    Reduce::Sum | Reduce::Mean => {
                        if self.dense {
                            let (ch, rem) = x.as_chunks::<8>();
                            for c in ch {
                                for k in 0..8 {
                                    self.acc8[k] += c[k];
                                }
                            }
                            self.rem += rem.iter().sum::<f64>();
                        } else {
                            for (b, block) in x.chunks(64).enumerate() {
                                let mut w = a.valid.word(b, n);
                                if w == u64::MAX && block.len() == 64 {
                                    self.acc += crate::f64_sum_dense(block);
                                } else {
                                    while w != 0 {
                                        let tz = w.trailing_zeros() as usize;
                                        self.lanes[self.k & 3] += block[tz];
                                        self.k += 1;
                                        w &= w - 1;
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        for (i, &v) in x.iter().enumerate() {
                            if a.valid.get(i) {
                                self.fbest = Some(match self.fbest {
                                    None => v,
                                    Some(b) => {
                                        if want_min == (v.total_cmp(&b) == std::cmp::Ordering::Less) {
                                            v
                                        } else {
                                            b
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
            }
            View::I(src) => {
                for i in 0..n {
                    if !a.valid.get(i) {
                        continue;
                    }
                    let v = src.at(i);
                    match self.kind {
                        Reduce::Sum | Reduce::Mean => {
                            self.isum = self
                                .isum
                                .checked_add(v)
                                .ok_or(OdsError::IntegerOverflow("addition"))?;
                        }
                        _ => {
                            self.ibest = Some(match self.ibest {
                                None => v,
                                Some(b) => {
                                    if want_min == (v < b) {
                                        v
                                    } else {
                                        b
                                    }
                                }
                            });
                        }
                    }
                }
            }
            View::B(src) => match self.kind {
                Reduce::Sum | Reduce::Mean => {
                    self.isum += match (src, a.valid) {
                        (_, Vw::Null) => 0,
                        (Src::C(c), _) => {
                            if c {
                                count as i64
                            } else {
                                0
                            }
                        }
                        (Src::S(x), Vw::All) => {
                            x[..n].iter().map(|&b| b as u32).sum::<u32>() as i64
                        }
                        (Src::S(x), Vw::W(w)) => (0..nw)
                            .map(|k| (pack(&x[..n], k) & w[k]).count_ones() as i64)
                            .sum(),
                    };
                }
                _ => {
                    return Err(OdsError::TypeMismatch(
                        "min/max are not defined for Bool series".to_string(),
                    ));
                }
            },
            View::T(_) => {
                return Err(OdsError::TypeMismatch(
                    "a String expression has no numeric reduction".to_string(),
                ));
            }
        }
        Ok(())
    }

    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    fn merge(&mut self, o: Fold) -> Result<()> {
        self.count += o.count;
        self.acc += o.float_sum();
        self.isum = self
            .isum
            .checked_add(o.isum)
            .ok_or(OdsError::IntegerOverflow("addition"))?;
        let want_min = self.kind == Reduce::Min;
        if let Some(v) = o.fbest {
            self.fbest = Some(match self.fbest {
                None => v,
                Some(b) if want_min == (v.total_cmp(&b) == std::cmp::Ordering::Less) => v,
                Some(b) => b,
            });
        }
        if let Some(v) = o.ibest {
            self.ibest = Some(match self.ibest {
                None => v,
                Some(b) if want_min == (v < b) => v,
                Some(b) => b,
            });
        }
        Ok(())
    }

    fn float_sum(&self) -> f64 {
        if self.dense {
            self.acc8.iter().sum::<f64>() + self.rem + self.acc
        } else {
            self.acc + self.lanes.iter().sum::<f64>()
        }
    }

    fn finish(self) -> Result<Scalar> {
        Ok(match (self.kind, self.dt) {
            (Reduce::Count, _) => Scalar::I64(self.count as i64),
            (Reduce::Sum, DType::F64) => Scalar::F64(self.float_sum()),
            (Reduce::Sum, _) => Scalar::I64(self.isum),
            (Reduce::Mean, _) if self.count == 0 => Scalar::Null,
            (Reduce::Mean, DType::F64) => Scalar::F64(self.float_sum() / self.count as f64),
            (Reduce::Mean, _) => Scalar::F64(self.isum as f64 / self.count as f64),
            (_, DType::F64) => self.fbest.map(Scalar::F64).unwrap_or(Scalar::Null),
            (_, DType::Date) => self
                .ibest
                .map(|d| Scalar::Date(d as i32))
                .unwrap_or(Scalar::Null),
            (_, DType::Datetime) => self.ibest.map(Scalar::Datetime).unwrap_or(Scalar::Null),
            (_, _) => self.ibest.map(Scalar::I64).unwrap_or(Scalar::Null),
        })
    }
}

