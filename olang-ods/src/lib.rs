//! ods — the numerical engine behind olang's data stack.
//!
//! Pure buffers and kernels with **no dependency on olang**: the language
//! glue lives in the olang crate (`src/ods/`) and calls in here. See
//! `docs/design/ods.md` in the olang repository for the full design.
//!
//! Semantics the kernels guarantee (chosen to mirror olang's scalar
//! operators, so operator interception cannot diverge from the language):
//!
//! - **Nulls propagate** through elementwise kernels: a position computes
//!   only when every operand is valid there, so a zero divisor under a
//!   null is not an error. **Reductions skip nulls** (`mean` of
//!   `[1, null, 3]` is 2).
//! - Integer arithmetic is **checked** — overflow and division by zero
//!   report the interpreter's exact error wording. Float division by zero
//!   errors too, as in the language ("Division by zero"), rather than
//!   producing infinities.
//! - `Int ⊕ Float` widens to `Float`, as in the language.
//! - Buffers are `Arc`-shared; kernels that can reuse a uniquely-owned
//!   buffer do so via `Arc::make_mut` (copy-on-write).
//! - Every parallel kernel takes a caller-provided `par` flag and has a
//!   sequential twin: the caller owns parallelism policy (the olang
//!   runtime routes its configured threshold through here), and disabling
//!   the `parallel` feature only changes speed, never results.

pub mod bitmap;
#[cfg(feature = "stats")]
pub mod dist;
pub mod frame;
pub mod plot;
#[cfg(feature = "stats")]
pub mod stats;

pub use bitmap::{Bitmap, merge_validity};
pub use frame::{AggOp, AggSpec, Frame, JoinHow, RankMethod};

#[cfg(feature = "parallel")]
use rayon::prelude::*;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The engine-wide parallel switch. Kernels that decide to fan out on
/// their own (argsort, the join probe, the group-by scatter) consult it
/// in addition to their size thresholds, so the host language's
/// `set_parallel(false)` governs the data stack the same way it governs
/// explicit `par_map`. Kernels that take a `par` flag are already
/// governed by their caller and do not check it.
static PARALLEL_ENABLED: AtomicBool = AtomicBool::new(true);

/// Flip the engine-wide parallel switch (see [`parallel_enabled`]).
pub fn set_parallel_enabled(on: bool) {
    PARALLEL_ENABLED.store(on, Ordering::Relaxed);
}

/// Is the engine allowed to fan out on its own?
pub fn parallel_enabled() -> bool {
    PARALLEL_ENABLED.load(Ordering::Relaxed)
}

/// Below this many rows, argsort stays sequential — the merge passes of a
/// parallel stable sort cost more than they save.
#[cfg(feature = "parallel")]
const PAR_SORT_ROWS: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DType {
    F64,
    I64,
    Bool,
    Str,
}

impl fmt::Display for DType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DType::F64 => write!(f, "Float"),
            DType::I64 => write!(f, "Int"),
            DType::Bool => write!(f, "Bool"),
            DType::Str => write!(f, "String"),
        }
    }
}

/// A single element crossing the engine boundary.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    F64(f64),
    I64(i64),
    Bool(bool),
    Str(String),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl ArithOp {
    fn word(self) -> &'static str {
        match self {
            ArithOp::Add => "addition",
            ArithOp::Sub => "subtraction",
            ArithOp::Mul => "multiplication",
            ArithOp::Div => "division",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OdsError {
    LengthMismatch { left: usize, right: usize },
    DivisionByZero,
    IntegerOverflow(&'static str),
    IndexOutOfBounds { index: i64, length: usize },
    TypeMismatch(String),
    InvalidArgument(String),
}

impl fmt::Display for OdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OdsError::LengthMismatch { left, right } => {
                write!(f, "series length mismatch: {} vs {}", left, right)
            }
            // The interpreter's exact wording — operator interception must
            // report what the scalar op would have.
            OdsError::DivisionByZero => write!(f, "Division by zero"),
            OdsError::IntegerOverflow(op) => write!(f, "Integer overflow in {}", op),
            OdsError::IndexOutOfBounds { index, length } => {
                write!(f, "Index out of bounds: {} for length {}", index, length)
            }
            OdsError::TypeMismatch(msg) => write!(f, "{}", msg),
            OdsError::InvalidArgument(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for OdsError {}

type Result<T> = std::result::Result<T, OdsError>;

/// A string column: cells are `(start, len)` spans into one shared
/// immutable buffer. Gather, filter, sort, and join copy spans and
/// share the buffer; the CSV reader points spans straight into the
/// file body. Offsets are u32 — a single text column is capped at
/// 4 GB of backing text, enforced by the builders.
#[derive(Debug, Clone)]
pub struct StrCol {
    buf: Arc<str>,
    spans: Arc<Vec<(u32, u32)>>,
}

impl StrCol {
    /// When a gather keeps less than this fraction of the backing
    /// buffer's bytes, the result is rebuilt into a tight buffer so a
    /// small filtered frame never pins a huge file body in memory.
    const COMPACT_DENOM: usize = 8;

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    #[inline]
    pub fn get(&self, i: usize) -> &str {
        let (start, len) = self.spans[i];
        &self.buf[start as usize..(start + len) as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.spans
            .iter()
            .map(|&(start, len)| &self.buf[start as usize..(start + len) as usize])
    }

    /// Pack owned/borrowed strings into one contiguous buffer.
    /// Panics if the packed text exceeds the u32 offset space (a 4 GB
    /// text column does not fit this layout).
    pub fn from_strings<S: AsRef<str>>(vals: impl IntoIterator<Item = S>) -> Self {
        let vals: Vec<S> = vals.into_iter().collect();
        let total: usize = vals.iter().map(|s| s.as_ref().len()).sum();
        assert!(
            u32::try_from(total).is_ok(),
            "text column exceeds 4 GB of backing text"
        );
        let mut buf = String::with_capacity(total);
        let mut spans = Vec::with_capacity(vals.len());
        for s in &vals {
            let s = s.as_ref();
            spans.push((buf.len() as u32, s.len() as u32));
            buf.push_str(s);
        }
        StrCol {
            buf: Arc::from(buf),
            spans: Arc::new(spans),
        }
    }

    /// Cells as spans into a caller-provided buffer — the CSV reader's
    /// zero-copy path. Spans must lie inside `buf` on char boundaries;
    /// debug builds check every one.
    pub fn from_spans(buf: Arc<str>, spans: Vec<(u32, u32)>) -> Self {
        debug_assert!(
            spans
                .iter()
                .all(|&(a, l)| buf.get(a as usize..(a + l) as usize).is_some()),
            "span outside buffer or off a char boundary"
        );
        StrCol {
            buf,
            spans: Arc::new(spans),
        }
    }

    /// Keep `indices`, sharing the buffer — unless the kept text is a
    /// small fraction of it, in which case rebuild tight.
    pub fn gather(&self, indices: &[usize]) -> Self {
        let spans: Vec<(u32, u32)> = indices.iter().map(|&i| self.spans[i]).collect();
        self.rewrap(spans)
    }

    /// Gather with absent slots (outer-join shape): absent cells become
    /// empty spans — the caller's validity bitmap is what marks them
    /// null.
    pub fn gather_opt(&self, indices: &[Option<usize>]) -> Self {
        let spans: Vec<(u32, u32)> = indices
            .iter()
            .map(|o| o.map(|i| self.spans[i]).unwrap_or((0, 0)))
            .collect();
        self.rewrap(spans)
    }

    /// The raw spans — for kernels that reorder cells while keeping
    /// the shared buffer (sort, join gathers).
    pub fn spans(&self) -> &[(u32, u32)] {
        &self.spans
    }

    /// The shared backing buffer.
    pub fn buf_arc(&self) -> Arc<str> {
        self.buf.clone()
    }

    fn rewrap(&self, spans: Vec<(u32, u32)>) -> Self {
        let kept: usize = spans.iter().map(|&(_, l)| l as usize).sum();
        if kept < self.buf.len() / Self::COMPACT_DENOM {
            let mut buf = String::with_capacity(kept);
            let compacted = spans
                .into_iter()
                .map(|(a, l)| {
                    let start = buf.len() as u32;
                    buf.push_str(&self.buf[a as usize..(a + l) as usize]);
                    (start, l)
                })
                .collect();
            StrCol {
                buf: Arc::from(buf),
                spans: Arc::new(compacted),
            }
        } else {
            StrCol {
                buf: self.buf.clone(),
                spans: Arc::new(spans),
            }
        }
    }
}

/// A 1-D typed, null-aware, Arc-shared array.
#[derive(Clone, Debug)]
pub enum Series {
    F64 {
        values: Arc<Vec<f64>>,
        validity: Option<Bitmap>,
    },
    I64 {
        values: Arc<Vec<i64>>,
        validity: Option<Bitmap>,
    },
    Bool {
        values: Arc<Vec<bool>>,
        validity: Option<Bitmap>,
    },
    Str {
        /// String cells are (start, len) spans into one shared buffer
        /// (`StrCol`) — the Arrow/Polars string design. Bulk movement
        /// copies 8-byte spans and shares the buffer; the CSV reader
        /// records spans into the file body itself, so loading a text
        /// column allocates nothing per cell.
        col: StrCol,
        validity: Option<Bitmap>,
    },
}

// ---------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------

fn split_options<T: Copy + Default>(opts: Vec<Option<T>>) -> (Vec<T>, Option<Bitmap>) {
    if opts.iter().all(|o| o.is_some()) {
        return (opts.into_iter().map(|o| o.unwrap()).collect(), None);
    }
    let bits: Vec<bool> = opts.iter().map(|o| o.is_some()).collect();
    let values = opts.into_iter().map(|o| o.unwrap_or_default()).collect();
    (values, Some(Bitmap::from_bools(&bits)))
}

impl Series {
    pub fn from_f64(values: Vec<f64>) -> Self {
        Series::F64 {
            values: Arc::new(values),
            validity: None,
        }
    }

    pub fn from_i64(values: Vec<i64>) -> Self {
        Series::I64 {
            values: Arc::new(values),
            validity: None,
        }
    }

    pub fn from_bool(values: Vec<bool>) -> Self {
        Series::Bool {
            values: Arc::new(values),
            validity: None,
        }
    }

    pub fn from_f64_options(opts: Vec<Option<f64>>) -> Self {
        let (values, validity) = split_options(opts);
        Series::F64 {
            values: Arc::new(values),
            validity,
        }
    }

    pub fn from_i64_options(opts: Vec<Option<i64>>) -> Self {
        let (values, validity) = split_options(opts);
        Series::I64 {
            values: Arc::new(values),
            validity,
        }
    }

    pub fn from_bool_options(opts: Vec<Option<bool>>) -> Self {
        let (values, validity) = split_options(opts);
        Series::Bool {
            values: Arc::new(values),
            validity,
        }
    }

    pub fn from_str_values(values: Vec<String>) -> Self {
        Series::Str {
            col: StrCol::from_strings(values),
            validity: None,
        }
    }

    pub fn from_str_options(opts: Vec<Option<String>>) -> Self {
        if opts.iter().all(|o| o.is_some()) {
            return Self::from_str_values(opts.into_iter().flatten().collect());
        }
        let bits: Vec<bool> = opts.iter().map(|o| o.is_some()).collect();
        let col = StrCol::from_strings(opts.iter().map(|o| o.as_deref().unwrap_or("")));
        Series::Str {
            col,
            validity: Some(Bitmap::from_bools(&bits)),
        }
    }

    /// A string series over spans into a shared buffer — the CSV
    /// reader's zero-copy path. `nulls` marks the invalid cells (their
    /// spans should be empty).
    pub fn from_str_spans(buf: Arc<str>, spans: Vec<(u32, u32)>, nulls: Option<Vec<bool>>) -> Self {
        Series::Str {
            col: StrCol::from_spans(buf, spans),
            validity: nulls.map(|valid_bits| Bitmap::from_bools(&valid_bits)),
        }
    }

    pub fn zeros(len: usize) -> Self {
        Series::from_f64(vec![0.0; len])
    }

    pub fn linspace(start: f64, stop: f64, num: usize) -> Result<Self> {
        if num == 0 {
            return Ok(Series::from_f64(Vec::new()));
        }
        if num == 1 {
            return Ok(Series::from_f64(vec![start]));
        }
        let step = (stop - start) / (num - 1) as f64;
        Ok(Series::from_f64(
            (0..num).map(|i| start + step * i as f64).collect(),
        ))
    }

    /// olang range semantics: `start..end` exclusive, `start..=end` inclusive.
    pub fn from_range(start: i64, end: i64, inclusive: bool) -> Self {
        let upper = if inclusive { end + 1 } else { end };
        if upper <= start {
            return Series::from_i64(Vec::new());
        }
        Series::from_i64((start..upper).collect())
    }

    // -----------------------------------------------------------------
    // Introspection
    // -----------------------------------------------------------------

    pub fn dtype(&self) -> DType {
        match self {
            Series::F64 { .. } => DType::F64,
            Series::I64 { .. } => DType::I64,
            Series::Bool { .. } => DType::Bool,
            Series::Str { .. } => DType::Str,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Series::F64 { values, .. } => values.len(),
            Series::I64 { values, .. } => values.len(),
            Series::Bool { values, .. } => values.len(),
            Series::Str { col, .. } => col.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The validity bitmap, or `None` when the column has no nulls at
    /// all — which callers can use as a fast "nothing to check here".
    pub fn validity(&self) -> Option<&Bitmap> {
        match self {
            Series::F64 { validity, .. }
            | Series::I64 { validity, .. }
            | Series::Bool { validity, .. }
            | Series::Str { validity, .. } => validity.as_ref(),
        }
    }

    pub fn null_count(&self) -> usize {
        match self.validity() {
            None => 0,
            Some(v) => v.len() - v.count_valid(),
        }
    }

    #[inline]
    fn is_valid(&self, i: usize) -> bool {
        self.validity().map(|v| v.get(i)).unwrap_or(true)
    }

    /// Element access; caller guarantees `i < len`.
    pub fn scalar_at(&self, i: usize) -> Scalar {
        if !self.is_valid(i) {
            return Scalar::Null;
        }
        match self {
            Series::F64 { values, .. } => Scalar::F64(values[i]),
            Series::I64 { values, .. } => Scalar::I64(values[i]),
            Series::Bool { values, .. } => Scalar::Bool(values[i]),
            Series::Str { col, .. } => Scalar::Str(col.get(i).to_string()),
        }
    }

    pub fn get(&self, index: i64) -> Result<Scalar> {
        let len = self.len();
        let idx = if index < 0 { index + len as i64 } else { index };
        if idx < 0 || idx as usize >= len {
            return Err(OdsError::IndexOutOfBounds { index, length: len });
        }
        Ok(self.scalar_at(idx as usize))
    }

    /// Structural equality: same dtype, same length, same null pattern,
    /// equal values at valid positions. Values under null slots are
    /// ignored. Floats compare with `==` (a NaN position makes a series
    /// unequal to itself — standard float semantics).
    pub fn series_eq(&self, other: &Series) -> bool {
        if self.dtype() != other.dtype() || self.len() != other.len() {
            return false;
        }
        (0..self.len()).all(|i| self.scalar_at(i) == other.scalar_at(i))
    }

    // -----------------------------------------------------------------
    // Elementwise arithmetic
    // -----------------------------------------------------------------

    /// `self ⊕ other`, elementwise. Nulls propagate; only positions where
    /// both operands are valid compute (so error checks apply only there).
    pub fn arith(&self, op: ArithOp, other: &Series, par: bool) -> Result<Series> {
        if self.len() != other.len() {
            return Err(OdsError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let validity = merge_validity(self.validity(), other.validity());
        match (self, other) {
            (Series::F64 { values: a, .. }, Series::F64 { values: b, .. }) => {
                f64_arith(a, b, op, validity, par)
            }
            (Series::I64 { values: a, .. }, Series::I64 { values: b, .. }) => {
                i64_arith(a, b, op, validity)
            }
            (Series::I64 { values: a, .. }, Series::F64 { values: b, .. }) => {
                let a: Vec<f64> = a.iter().map(|&x| x as f64).collect();
                f64_arith(&a, b, op, validity, par)
            }
            (Series::F64 { values: a, .. }, Series::I64 { values: b, .. }) => {
                let b: Vec<f64> = b.iter().map(|&x| x as f64).collect();
                f64_arith(a, &b, op, validity, par)
            }
            _ => Err(OdsError::TypeMismatch(format!(
                "cannot apply {} to Series[{}] and Series[{}]",
                op.word(),
                self.dtype(),
                other.dtype()
            ))),
        }
    }

    /// `self ⊕ scalar` (or `scalar ⊕ self` when `swapped`), elementwise.
    pub fn arith_scalar(
        &self,
        op: ArithOp,
        scalar: Scalar,
        swapped: bool,
        par: bool,
    ) -> Result<Series> {
        let validity = self.validity().cloned();
        match (self, &scalar) {
            (_, Scalar::Null) => Err(OdsError::InvalidArgument(
                "cannot apply arithmetic with a null scalar".to_string(),
            )),
            (Series::F64 { values, .. }, Scalar::F64(s)) => {
                f64_arith_scalar(values, *s, op, swapped, validity, par)
            }
            (Series::F64 { values, .. }, Scalar::I64(s)) => {
                f64_arith_scalar(values, *s as f64, op, swapped, validity, par)
            }
            (Series::I64 { values, .. }, Scalar::F64(s)) => {
                let a: Vec<f64> = values.iter().map(|&x| x as f64).collect();
                f64_arith_scalar(&a, *s, op, swapped, validity, par)
            }
            (Series::I64 { values, .. }, Scalar::I64(s)) => {
                i64_arith_scalar(values, *s, op, swapped, validity)
            }
            _ => Err(OdsError::TypeMismatch(format!(
                "cannot apply {} to Series[{}] and {:?}",
                op.word(),
                self.dtype(),
                scalar
            ))),
        }
    }

    /// Apply a unary `f64 → f64` math function elementwise, producing an
    /// F64 series. Nulls propagate untouched; only valid positions are
    /// evaluated. Domain-restricted functions (`sqrt`, `ln`, `log2`,
    /// `log10`, `asin`, `acos`) error on any *valid* out-of-domain
    /// element, exactly matching the scalar `math.*` functions — so
    /// `ods.map(xs, "sin")` is the vectorized form of
    /// `map(xs, (v) => math.sin(v))`, computed entirely in the kernel
    /// with zero per-element boundary crossings. Every function
    /// evaluates through the same std `f64` methods `math.*` calls, so
    /// results are bit-identical by construction.
    pub fn map_unary(&self, name: &str) -> Result<Series> {
        let src: Vec<f64> = match self {
            Series::F64 { values, .. } => (**values).clone(),
            Series::I64 { values, .. } => values.iter().map(|&x| x as f64).collect(),
            _ => {
                return Err(OdsError::TypeMismatch(format!(
                    "ods.map: a {} series is not numeric",
                    self.dtype()
                )));
            }
        };
        let f: fn(f64) -> f64 = match name {
            "sin" => f64::sin,
            "cos" => f64::cos,
            "tan" => f64::tan,
            "asin" => f64::asin,
            "acos" => f64::acos,
            "atan" => f64::atan,
            "sinh" => f64::sinh,
            "cosh" => f64::cosh,
            "tanh" => f64::tanh,
            "exp" => f64::exp,
            "exp2" => f64::exp2,
            "ln" => f64::ln,
            "log2" => f64::log2,
            "log10" => f64::log10,
            "sqrt" => f64::sqrt,
            "cbrt" => f64::cbrt,
            "floor" => f64::floor,
            "ceil" => f64::ceil,
            "round" => f64::round,
            "trunc" => f64::trunc,
            "fract" => f64::fract,
            "abs" => f64::abs,
            "degrees" => f64::to_degrees,
            "radians" => f64::to_radians,
            _ => {
                return Err(OdsError::InvalidArgument(format!(
                    "ods.map: unknown function \"{}\" — one of sin, cos, tan, asin, \
                     acos, atan, sinh, cosh, tanh, exp, exp2, ln, log2, log10, sqrt, \
                     cbrt, floor, ceil, round, trunc, fract, abs, degrees, radians",
                    name
                )));
            }
        };
        // The domain guard reproduces the exact error the scalar
        // `math.<name>` raises on an out-of-range argument.
        let domain_err = |x: f64| -> Option<String> {
            match name {
                "sqrt" if x < 0.0 => {
                    Some("sqrt: cannot take square root of negative number".to_string())
                }
                "ln" if x <= 0.0 => Some("ln: input must be positive".to_string()),
                "log2" if x <= 0.0 => Some("log2: input must be positive".to_string()),
                "log10" if x <= 0.0 => Some("log10: input must be positive".to_string()),
                "asin" if !(-1.0..=1.0).contains(&x) => {
                    Some("asin: input must be in range [-1, 1]".to_string())
                }
                "acos" if !(-1.0..=1.0).contains(&x) => {
                    Some("acos: input must be in range [-1, 1]".to_string())
                }
                _ => None,
            }
        };
        let validity = self.validity().cloned();
        let mut out = Vec::with_capacity(src.len());
        for (i, &x) in src.iter().enumerate() {
            if validity.as_ref().map(|v| v.get(i)).unwrap_or(true) {
                if let Some(msg) = domain_err(x) {
                    return Err(OdsError::InvalidArgument(msg));
                }
                out.push(f(x));
            } else {
                out.push(0.0);
            }
        }
        Ok(Series::F64 {
            values: Arc::new(out),
            validity,
        })
    }

    // -----------------------------------------------------------------
    // Comparisons
    // -----------------------------------------------------------------

    pub fn compare(&self, op: CmpOp, other: &Series) -> Result<Series> {
        if self.len() != other.len() {
            return Err(OdsError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let validity = merge_validity(self.validity(), other.validity());
        let bits: Vec<bool> = match (self, other) {
            (Series::F64 { values: a, .. }, Series::F64 { values: b, .. }) => cmp_slices(a, b, op),
            (Series::I64 { values: a, .. }, Series::I64 { values: b, .. }) => cmp_slices(a, b, op),
            (Series::I64 { values: a, .. }, Series::F64 { values: b, .. }) => {
                let a: Vec<f64> = a.iter().map(|&x| x as f64).collect();
                cmp_slices(&a, b, op)
            }
            (Series::F64 { values: a, .. }, Series::I64 { values: b, .. }) => {
                let b: Vec<f64> = b.iter().map(|&x| x as f64).collect();
                cmp_slices(a, &b, op)
            }
            (Series::Bool { values: a, .. }, Series::Bool { values: b, .. }) => match op {
                CmpOp::Eq => a.iter().zip(b.iter()).map(|(x, y)| x == y).collect(),
                CmpOp::Ne => a.iter().zip(b.iter()).map(|(x, y)| x != y).collect(),
                _ => {
                    return Err(OdsError::TypeMismatch(
                        "Bool series support only == and != comparisons".to_string(),
                    ));
                }
            },
            // Strings order lexicographically, like the language's own
            // string comparisons.
            (Series::Str { col: a, .. }, Series::Str { col: b, .. }) => a
                .iter()
                .zip(b.iter())
                .map(|(x, y)| cmp_one(x, y, op))
                .collect(),
            _ => {
                return Err(OdsError::TypeMismatch(format!(
                    "cannot compare Series[{}] with Series[{}]",
                    self.dtype(),
                    other.dtype()
                )));
            }
        };
        Ok(Series::Bool {
            values: Arc::new(bits),
            validity,
        })
    }

    pub fn compare_scalar(&self, op: CmpOp, scalar: Scalar, swapped: bool) -> Result<Series> {
        // a < s  ≡  swapped(s < a) with the operator mirrored.
        let op = if swapped { mirror_cmp(op) } else { op };
        let validity = self.validity().cloned();
        let bits: Vec<bool> = match (self, &scalar) {
            (Series::F64 { values, .. }, Scalar::F64(s)) => {
                values.iter().map(|&x| cmp_one(x, *s, op)).collect()
            }
            (Series::F64 { values, .. }, Scalar::I64(s)) => {
                values.iter().map(|&x| cmp_one(x, *s as f64, op)).collect()
            }
            (Series::I64 { values, .. }, Scalar::I64(s)) => {
                values.iter().map(|&x| cmp_one(x, *s, op)).collect()
            }
            (Series::I64 { values, .. }, Scalar::F64(s)) => {
                values.iter().map(|&x| cmp_one(x as f64, *s, op)).collect()
            }
            (Series::Str { col, .. }, Scalar::Str(s)) => {
                col.iter().map(|x| cmp_one(x, s.as_str(), op)).collect()
            }
            (Series::Bool { values, .. }, Scalar::Bool(s)) => match op {
                CmpOp::Eq => values.iter().map(|&x| x == *s).collect(),
                CmpOp::Ne => values.iter().map(|&x| x != *s).collect(),
                _ => {
                    return Err(OdsError::TypeMismatch(
                        "Bool series support only == and != comparisons".to_string(),
                    ));
                }
            },
            _ => {
                return Err(OdsError::TypeMismatch(format!(
                    "cannot compare Series[{}] with {:?}",
                    self.dtype(),
                    scalar
                )));
            }
        };
        Ok(Series::Bool {
            values: Arc::new(bits),
            validity,
        })
    }

    // -----------------------------------------------------------------
    // Reductions (skip nulls)
    // -----------------------------------------------------------------

    pub fn sum(&self, par: bool) -> Result<Scalar> {
        match self {
            Series::F64 { values, validity } => {
                Ok(Scalar::F64(f64_sum(values, validity.as_ref(), par)))
            }
            Series::I64 { values, validity } => {
                let mut acc: i64 = 0;
                for i in 0..values.len() {
                    if validity.as_ref().map(|v| v.get(i)).unwrap_or(true) {
                        acc = acc
                            .checked_add(values[i])
                            .ok_or(OdsError::IntegerOverflow("addition"))?;
                    }
                }
                Ok(Scalar::I64(acc))
            }
            Series::Bool { .. } => Err(OdsError::TypeMismatch(
                "sum is not defined for Bool series".to_string(),
            )),
            Series::Str { .. } => Err(OdsError::TypeMismatch(
                "sum is not defined for String series".to_string(),
            )),
        }
    }

    fn valid_count(&self) -> usize {
        self.len() - self.null_count()
    }

    pub fn mean(&self, par: bool) -> Result<Option<f64>> {
        let n = self.valid_count();
        if n == 0 {
            return Ok(None);
        }
        let total = match self.sum(par)? {
            Scalar::F64(x) => x,
            Scalar::I64(x) => x as f64,
            _ => unreachable!("sum returns numeric scalars"),
        };
        Ok(Some(total / n as f64))
    }

    /// Sample variance (ddof = 1). `None` when fewer than two valid values.
    pub fn var(&self, par: bool) -> Result<Option<f64>> {
        let n = self.valid_count();
        if n < 2 {
            return Ok(None);
        }
        let mean = self.mean(par)?.expect("n >= 2");
        let ss = match self {
            Series::F64 { values, validity } => {
                f64_sum_sq_dev(values, validity.as_ref(), mean, par)
            }
            Series::I64 { values, validity } => {
                let cast: Vec<f64> = values.iter().map(|&x| x as f64).collect();
                f64_sum_sq_dev(&cast, validity.as_ref(), mean, par)
            }
            Series::Bool { .. } | Series::Str { .. } => {
                return Err(OdsError::TypeMismatch(format!(
                    "var is not defined for {} series",
                    self.dtype()
                )));
            }
        };
        Ok(Some(ss / (n - 1) as f64))
    }

    pub fn std(&self, par: bool) -> Result<Option<f64>> {
        Ok(self.var(par)?.map(f64::sqrt))
    }

    pub fn min(&self) -> Result<Scalar> {
        self.extremum(true)
    }

    pub fn max(&self) -> Result<Scalar> {
        self.extremum(false)
    }

    fn extremum(&self, want_min: bool) -> Result<Scalar> {
        match self {
            Series::F64 { values, validity } => {
                let mut best: Option<f64> = None;
                for i in 0..values.len() {
                    if validity.as_ref().map(|v| v.get(i)).unwrap_or(true) {
                        let x = values[i];
                        best = Some(match best {
                            None => x,
                            Some(b) => {
                                if want_min == (x.total_cmp(&b) == std::cmp::Ordering::Less) {
                                    x
                                } else {
                                    b
                                }
                            }
                        });
                    }
                }
                Ok(best.map(Scalar::F64).unwrap_or(Scalar::Null))
            }
            Series::I64 { values, validity } => {
                let mut best: Option<i64> = None;
                for i in 0..values.len() {
                    if validity.as_ref().map(|v| v.get(i)).unwrap_or(true) {
                        let x = values[i];
                        best = Some(match best {
                            None => x,
                            Some(b) => {
                                if want_min == (x < b) {
                                    x
                                } else {
                                    b
                                }
                            }
                        });
                    }
                }
                Ok(best.map(Scalar::I64).unwrap_or(Scalar::Null))
            }
            Series::Bool { .. } => Err(OdsError::TypeMismatch(
                "min/max are not defined for Bool series".to_string(),
            )),
            Series::Str { col, validity } => {
                let mut best: Option<&str> = None;
                for (i, v) in col.iter().enumerate() {
                    if validity.as_ref().map(|b| b.get(i)).unwrap_or(true) {
                        best = Some(match best {
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
                Ok(best
                    .map(|s| Scalar::Str(s.to_string()))
                    .unwrap_or(Scalar::Null))
            }
        }
    }

    /// Quantile with linear interpolation over the sorted valid values
    /// (NumPy's default). `q` must lie in `[0, 1]`. `None` when no valid
    /// values exist.
    pub fn quantile(&self, q: f64) -> Result<Option<f64>> {
        if !(0.0..=1.0).contains(&q) {
            return Err(OdsError::InvalidArgument(format!(
                "quantile expects q in [0, 1], got {}",
                q
            )));
        }
        let mut vals = self.valid_f64_values()?;
        if vals.is_empty() {
            return Ok(None);
        }
        vals.sort_unstable_by(f64::total_cmp);
        let h = (vals.len() - 1) as f64 * q;
        let lo = h.floor() as usize;
        let hi = h.ceil() as usize;
        Ok(Some(vals[lo] + (vals[hi] - vals[lo]) * (h - lo as f64)))
    }

    fn valid_f64_values(&self) -> Result<Vec<f64>> {
        match self {
            Series::F64 { values, validity } => Ok((0..values.len())
                .filter(|&i| validity.as_ref().map(|v| v.get(i)).unwrap_or(true))
                .map(|i| values[i])
                .collect()),
            Series::I64 { values, validity } => Ok((0..values.len())
                .filter(|&i| validity.as_ref().map(|v| v.get(i)).unwrap_or(true))
                .map(|i| values[i] as f64)
                .collect()),
            Series::Bool { .. } | Series::Str { .. } => Err(OdsError::TypeMismatch(format!(
                "numeric reduction is not defined for {} series",
                self.dtype()
            ))),
        }
    }

    /// Dot product, skipping positions where either side is null.
    pub fn dot(&self, other: &Series, par: bool) -> Result<f64> {
        if self.len() != other.len() {
            return Err(OdsError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let merged = merge_validity(self.validity(), other.validity());
        let a = self.valid_f64_all()?;
        let b = other.valid_f64_all()?;
        if merged.is_none() {
            #[cfg(feature = "parallel")]
            if par {
                return Ok(a.par_iter().zip(b.par_iter()).map(|(x, y)| x * y).sum());
            }
            let _ = par;
            return Ok(a.iter().zip(b.iter()).map(|(x, y)| x * y).sum());
        }
        let bm = merged.unwrap();
        Ok((0..a.len())
            .filter(|&i| bm.get(i))
            .map(|i| a[i] * b[i])
            .sum())
    }

    /// All values cast to f64, including those under null slots (callers
    /// mask separately).
    fn valid_f64_all(&self) -> Result<Vec<f64>> {
        match self {
            Series::F64 { values, .. } => Ok(values.as_ref().clone()),
            Series::I64 { values, .. } => Ok(values.iter().map(|&x| x as f64).collect()),
            Series::Bool { .. } | Series::Str { .. } => Err(OdsError::TypeMismatch(format!(
                "numeric reduction is not defined for {} series",
                self.dtype()
            ))),
        }
    }

    // -----------------------------------------------------------------
    // Rearrangement and selection
    // -----------------------------------------------------------------

    /// Ascending sort; nulls go last. Floats use IEEE total order (NaN
    /// sorts after +inf).
    pub fn sort(&self, par: bool) -> Result<Series> {
        match self {
            Series::F64 { .. } | Series::I64 { .. } | Series::Str { .. } => {}
            Series::Bool { .. } => {
                return Err(OdsError::TypeMismatch(
                    "sort is not defined for Bool series".to_string(),
                ));
            }
        }
        let n = self.len();
        let nulls = self.null_count();
        match self {
            Series::F64 { values, validity } => {
                let mut vals: Vec<f64> = match validity {
                    None => values.as_ref().clone(),
                    Some(v) => (0..n).filter(|&i| v.get(i)).map(|i| values[i]).collect(),
                };
                #[cfg(feature = "parallel")]
                if par {
                    vals.par_sort_unstable_by(f64::total_cmp);
                } else {
                    vals.sort_unstable_by(f64::total_cmp);
                }
                #[cfg(not(feature = "parallel"))]
                {
                    let _ = par;
                    vals.sort_unstable_by(f64::total_cmp);
                }
                Ok(rebuild_with_trailing_nulls(
                    vals,
                    n,
                    nulls,
                    Series::from_f64,
                ))
            }
            Series::I64 { values, validity } => {
                let mut vals: Vec<i64> = match validity {
                    None => values.as_ref().clone(),
                    Some(v) => (0..n).filter(|&i| v.get(i)).map(|i| values[i]).collect(),
                };
                #[cfg(feature = "parallel")]
                if par {
                    vals.par_sort_unstable();
                } else {
                    vals.sort_unstable();
                }
                #[cfg(not(feature = "parallel"))]
                {
                    let _ = par;
                    vals.sort_unstable();
                }
                Ok(rebuild_with_trailing_nulls(
                    vals,
                    n,
                    nulls,
                    Series::from_i64,
                ))
            }
            Series::Str { col, validity } => {
                let mut vals: Vec<(u32, u32)> = match validity {
                    None => col.spans().to_vec(),
                    Some(v) => (0..n)
                        .filter(|&i| v.get(i))
                        .map(|i| col.spans()[i])
                        .collect(),
                };
                let buf = col.buf_arc();
                let key = |&(a, l): &(u32, u32)| -> &str { &buf[a as usize..(a + l) as usize] };
                #[cfg(feature = "parallel")]
                if par {
                    vals.par_sort_unstable_by(|a, b| key(a).cmp(key(b)));
                } else {
                    vals.sort_unstable_by(|a, b| key(a).cmp(key(b)));
                }
                #[cfg(not(feature = "parallel"))]
                {
                    let _ = par;
                    vals.sort_unstable_by(|a, b| key(a).cmp(key(b)));
                }
                let buf = col.buf_arc();
                Ok(rebuild_with_trailing_nulls(vals, n, nulls, move |spans| {
                    Series::Str {
                        col: StrCol::from_spans(buf.clone(), spans),
                        validity: None,
                    }
                }))
            }
            Series::Bool { .. } => unreachable!("rejected above"),
        }
    }

    /// Indices that would sort the series ascending (stable); null
    /// positions come last in their original order.
    pub fn argsort(&self) -> Result<Series> {
        let n = self.len();
        let (mut valid_idx, null_idx): (Vec<usize>, Vec<usize>) =
            (0..n).partition(|&i| self.is_valid(i));
        // Large series sort their index across all cores. Rayon's
        // par_sort_by is stable with the same comparator, so the result
        // is bit-identical to the sequential path — ties keep original
        // order either way.
        #[cfg(feature = "parallel")]
        let par = n >= PAR_SORT_ROWS && crate::parallel_enabled();
        match self {
            Series::F64 { values, .. } => {
                #[cfg(feature = "parallel")]
                if par {
                    valid_idx.par_sort_by(|&a, &b| values[a].total_cmp(&values[b]));
                } else {
                    valid_idx.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
                }
                #[cfg(not(feature = "parallel"))]
                valid_idx.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
            }
            Series::I64 { values, .. } => {
                #[cfg(feature = "parallel")]
                if par {
                    valid_idx.par_sort_by_key(|&i| values[i]);
                } else {
                    valid_idx.sort_by_key(|&i| values[i]);
                }
                #[cfg(not(feature = "parallel"))]
                valid_idx.sort_by_key(|&i| values[i]);
            }
            Series::Str { col, .. } => {
                #[cfg(feature = "parallel")]
                if par {
                    valid_idx.par_sort_by(|&a, &b| col.get(a).cmp(col.get(b)));
                } else {
                    valid_idx.sort_by(|&a, &b| col.get(a).cmp(col.get(b)));
                }
                #[cfg(not(feature = "parallel"))]
                valid_idx.sort_by(|&a, &b| col.get(a).cmp(col.get(b)));
            }
            Series::Bool { .. } => {
                return Err(OdsError::TypeMismatch(
                    "argsort is not defined for Bool series".to_string(),
                ));
            }
        }
        valid_idx.extend(null_idx);
        Ok(Series::from_i64(
            valid_idx.into_iter().map(|i| i as i64).collect(),
        ))
    }

    /// Gather by index; indices must be a non-null Int series, negatives
    /// count from the end (olang's indexing rule).
    pub fn take(&self, indices: &Series) -> Result<Series> {
        let idx = match indices {
            Series::I64 {
                values,
                validity: None,
            } => values,
            Series::I64 { .. } => {
                return Err(OdsError::InvalidArgument(
                    "take indices must not contain nulls".to_string(),
                ));
            }
            _ => {
                return Err(OdsError::TypeMismatch(format!(
                    "take indices must be an Int series, got Series[{}]",
                    indices.dtype()
                )));
            }
        };
        let resolved = resolve_take_indices(idx, self.len())?;
        Ok(self.gather(&resolved))
    }

    /// Keep positions where the mask is true; a null mask position drops
    /// the element.
    pub fn filter(&self, mask: &Series) -> Result<Series> {
        if self.len() != mask.len() {
            return Err(OdsError::LengthMismatch {
                left: self.len(),
                right: mask.len(),
            });
        }
        let keep = mask_keep_indices(mask)?;
        Ok(self.gather(&keep))
    }

    pub(crate) fn gather(&self, indices: &[usize]) -> Series {
        // A column without a validity bitmap has no nulls: skip the
        // full is_valid scan over the indices (it answered `true` per
        // element after a bitmap check that could not exist).
        let opts_needed = self.validity().is_some() && indices.iter().any(|&i| !self.is_valid(i));
        match self {
            Series::F64 { values, .. } => {
                if opts_needed {
                    Series::from_f64_options(
                        indices
                            .iter()
                            .map(|&i| self.is_valid(i).then(|| values[i]))
                            .collect(),
                    )
                } else {
                    Series::from_f64(indices.iter().map(|&i| values[i]).collect())
                }
            }
            Series::I64 { values, .. } => {
                if opts_needed {
                    Series::from_i64_options(
                        indices
                            .iter()
                            .map(|&i| self.is_valid(i).then(|| values[i]))
                            .collect(),
                    )
                } else {
                    Series::from_i64(indices.iter().map(|&i| values[i]).collect())
                }
            }
            Series::Bool { values, .. } => {
                if opts_needed {
                    Series::from_bool_options(
                        indices
                            .iter()
                            .map(|&i| self.is_valid(i).then(|| values[i]))
                            .collect(),
                    )
                } else {
                    Series::from_bool(indices.iter().map(|&i| values[i]).collect())
                }
            }
            Series::Str { col, .. } => {
                if opts_needed {
                    let valid: Vec<bool> = indices.iter().map(|&i| self.is_valid(i)).collect();
                    Series::Str {
                        col: col.gather(indices),
                        validity: Some(Bitmap::from_bools(&valid)),
                    }
                } else {
                    Series::Str {
                        col: col.gather(indices),
                        validity: None,
                    }
                }
            }
        }
    }

    /// Running sum over valid positions; null positions stay null and the
    /// running total carries across them.
    pub fn cumsum(&self) -> Result<Series> {
        match self {
            Series::F64 { values, validity } => {
                let mut acc = 0.0;
                let out: Vec<Option<f64>> = (0..values.len())
                    .map(|i| {
                        if validity.as_ref().map(|v| v.get(i)).unwrap_or(true) {
                            acc += values[i];
                            Some(acc)
                        } else {
                            None
                        }
                    })
                    .collect();
                Ok(Series::from_f64_options(out))
            }
            Series::I64 { values, validity } => {
                let mut acc: i64 = 0;
                let mut out: Vec<Option<i64>> = Vec::with_capacity(values.len());
                for i in 0..values.len() {
                    if validity.as_ref().map(|v| v.get(i)).unwrap_or(true) {
                        acc = acc
                            .checked_add(values[i])
                            .ok_or(OdsError::IntegerOverflow("addition"))?;
                        out.push(Some(acc));
                    } else {
                        out.push(None);
                    }
                }
                Ok(Series::from_i64_options(out))
            }
            Series::Bool { .. } | Series::Str { .. } => Err(OdsError::TypeMismatch(format!(
                "cumsum is not defined for {} series",
                self.dtype()
            ))),
        }
    }

    // -----------------------------------------------------------------
    // Conversion
    // -----------------------------------------------------------------

    /// Convert every element to `to`, keeping nulls null.
    ///
    /// A conversion that cannot represent a value yields a null rather
    /// than an error or a wrong number: parsing "abc" as an Int, or a
    /// Float too large for an i64, or a NaN. That is the ETL-shaped
    /// choice — one unparseable row in a million should not fail the
    /// load — and it is visible afterwards, since `null_count` counts
    /// exactly what was lost. Float to Int truncates toward zero.
    ///
    /// `fmt_f64` renders a float on the way to String. The engine has no
    /// opinion on how olang spells a float, and holding a second opinion
    /// here would be free to drift from the language's own.
    pub fn cast(&self, to: DType, fmt_f64: &dyn Fn(f64) -> String) -> Result<Series> {
        if self.dtype() == to {
            return Ok(self.clone());
        }
        // Float to Bool is the one pair with no defensible reading: 0.5
        // is neither true nor false, and NaN is neither. The comparison
        // the caller means is better written out.
        if self.dtype() == DType::F64 && to == DType::Bool {
            return Err(OdsError::InvalidArgument(
                "cast: Float to Bool has no meaning for values like 0.5. \
                 Write the comparison you mean, e.g. `s != 0.0`"
                    .to_string(),
            ));
        }
        let n = self.len();
        let scalars = (0..n).map(|i| self.scalar_at(i));
        Ok(match to {
            DType::F64 => Series::from_f64_options(
                scalars
                    .map(|v| match v {
                        Scalar::Null => None,
                        Scalar::F64(x) => Some(x),
                        Scalar::I64(x) => Some(x as f64),
                        Scalar::Bool(b) => Some(if b { 1.0 } else { 0.0 }),
                        Scalar::Str(s) => s.trim().parse::<f64>().ok(),
                    })
                    .collect(),
            ),
            DType::I64 => Series::from_i64_options(
                scalars
                    .map(|v| match v {
                        Scalar::Null => None,
                        Scalar::I64(x) => Some(x),
                        // Truncate toward zero, but only where the result
                        // is representable: `as` would silently saturate
                        // 1e30 to i64::MAX and turn NaN into 0.
                        Scalar::F64(x) => {
                            let t = x.trunc();
                            (t.is_finite() && t >= i64::MIN as f64 && t <= i64::MAX as f64)
                                .then_some(t as i64)
                        }
                        Scalar::Bool(b) => Some(i64::from(b)),
                        Scalar::Str(s) => s.trim().parse::<i64>().ok(),
                    })
                    .collect(),
            ),
            DType::Bool => Series::from_bool_options(
                scalars
                    .map(|v| match v {
                        Scalar::Null => None,
                        Scalar::Bool(b) => Some(b),
                        Scalar::I64(x) => Some(x != 0),
                        Scalar::Str(s) => match s.trim().to_ascii_lowercase().as_str() {
                            "true" => Some(true),
                            "false" => Some(false),
                            _ => None,
                        },
                        // Refused above.
                        Scalar::F64(_) => None,
                    })
                    .collect(),
            ),
            DType::Str => Series::from_str_options(
                scalars
                    .map(|v| match v {
                        Scalar::Null => None,
                        Scalar::Str(s) => Some(s),
                        Scalar::F64(x) => Some(fmt_f64(x)),
                        Scalar::I64(x) => Some(x.to_string()),
                        Scalar::Bool(b) => Some(b.to_string()),
                    })
                    .collect(),
            ),
        })
    }

    // -----------------------------------------------------------------
    // Nulls
    // -----------------------------------------------------------------

    pub fn is_null(&self) -> Series {
        let n = self.len();
        Series::from_bool((0..n).map(|i| !self.is_valid(i)).collect())
    }

    /// Replace nulls with `fill`. Copy-on-write: mutates the buffer in
    /// place when uniquely owned, so `xs |> fill_null(0.0)` on an
    /// unshared series allocates nothing.
    pub fn fill_null(mut self, fill: Scalar) -> Result<Series> {
        if self.validity().is_none() {
            return Ok(self);
        }
        match (&mut self, &fill) {
            (Series::F64 { values, validity }, Scalar::F64(_) | Scalar::I64(_)) => {
                let f = match fill {
                    Scalar::F64(x) => x,
                    Scalar::I64(x) => x as f64,
                    _ => unreachable!(),
                };
                let bm = validity.take().expect("checked above");
                let vals = Arc::make_mut(values);
                for (i, v) in vals.iter_mut().enumerate() {
                    if !bm.get(i) {
                        *v = f;
                    }
                }
            }
            (Series::I64 { values, validity }, Scalar::I64(f)) => {
                let f = *f;
                let bm = validity.take().expect("checked above");
                let vals = Arc::make_mut(values);
                for (i, v) in vals.iter_mut().enumerate() {
                    if !bm.get(i) {
                        *v = f;
                    }
                }
            }
            (Series::Bool { values, validity }, Scalar::Bool(f)) => {
                let f = *f;
                let bm = validity.take().expect("checked above");
                let vals = Arc::make_mut(values);
                for (i, v) in vals.iter_mut().enumerate() {
                    if !bm.get(i) {
                        *v = f;
                    }
                }
            }
            (Series::Str { col, validity }, Scalar::Str(f)) => {
                let bm = validity.take().expect("checked above");
                *col = StrCol::from_strings(
                    (0..col.len()).map(|i| if bm.get(i) { col.get(i) } else { f.as_str() }),
                );
            }
            _ => {
                return Err(OdsError::TypeMismatch(format!(
                    "cannot fill Series[{}] nulls with {:?}",
                    self.dtype(),
                    fill
                )));
            }
        }
        Ok(self)
    }
}

// ---------------------------------------------------------------------
// f64 kernels — dense fast paths use eight accumulators (strict FP
// forbids the compiler from breaking the add dependency chain itself, so
// a naive `.sum()` runs at add latency, ~5x slower than memory
// bandwidth; the summation order differs from left-to-right by design,
// exactly like NumPy's pairwise sum). Null paths go word-wise over the
// validity bitmap: an all-ones word runs the dense kernel on its
// 64-element block, anything else iterates set bits.
// ---------------------------------------------------------------------

fn f64_sum_dense(values: &[f64]) -> f64 {
    let mut acc = [0.0f64; 8];
    let (chunks, rem) = values.as_chunks::<8>();
    for c in chunks {
        for k in 0..8 {
            acc[k] += c[k];
        }
    }
    acc.iter().sum::<f64>() + rem.iter().sum::<f64>()
}

fn f64_sum_sq_dev_dense(values: &[f64], mean: f64) -> f64 {
    let mut acc = [0.0f64; 8];
    let (chunks, rem) = values.as_chunks::<8>();
    for c in chunks {
        for k in 0..8 {
            let d = c[k] - mean;
            acc[k] += d * d;
        }
    }
    acc.iter().sum::<f64>() + rem.iter().map(|&x| (x - mean) * (x - mean)).sum::<f64>()
}

/// Fold the valid positions of a null-carrying buffer: dense 64-blocks
/// take the fast kernel, sparse blocks iterate set bits (tail bits of
/// the last word are zero by the bitmap invariant).
fn f64_fold_valid(
    values: &[f64],
    bm: &Bitmap,
    dense: impl Fn(&[f64]) -> f64,
    one: impl Fn(f64) -> f64,
) -> f64 {
    let words = bm.words();
    let mut acc = 0.0;
    // Four rotating lanes in the sparse path break the FP add chain the
    // same way the dense kernel's eight accumulators do.
    let mut lanes = [0.0f64; 4];
    let mut k = 0usize;
    for (block, chunk) in values.chunks(64).enumerate() {
        let mut w = words[block];
        if w == u64::MAX && chunk.len() == 64 {
            acc += dense(chunk);
        } else {
            while w != 0 {
                let tz = w.trailing_zeros() as usize;
                debug_assert!(tz < chunk.len(), "tail bits must be zero");
                lanes[k & 3] += one(chunk[tz]);
                k += 1;
                w &= w - 1;
            }
        }
    }
    acc + lanes.iter().sum::<f64>()
}

fn f64_sum(values: &[f64], validity: Option<&Bitmap>, par: bool) -> f64 {
    match validity {
        None => {
            #[cfg(feature = "parallel")]
            if par {
                return values.par_chunks(65_536).map(f64_sum_dense).sum();
            }
            let _ = par;
            f64_sum_dense(values)
        }
        Some(bm) => f64_fold_valid(values, bm, f64_sum_dense, |x| x),
    }
}

fn f64_sum_sq_dev(values: &[f64], validity: Option<&Bitmap>, mean: f64, par: bool) -> f64 {
    match validity {
        None => {
            #[cfg(feature = "parallel")]
            if par {
                return values
                    .par_chunks(65_536)
                    .map(|c| f64_sum_sq_dev_dense(c, mean))
                    .sum();
            }
            let _ = par;
            f64_sum_sq_dev_dense(values, mean)
        }
        Some(bm) => f64_fold_valid(
            values,
            bm,
            |c| f64_sum_sq_dev_dense(c, mean),
            |x| (x - mean) * (x - mean),
        ),
    }
}

fn f64_arith(
    a: &[f64],
    b: &[f64],
    op: ArithOp,
    validity: Option<Bitmap>,
    par: bool,
) -> Result<Series> {
    if op == ArithOp::Div {
        // The language errors on float division by zero; only positions
        // that actually compute (both operands valid) are checked.
        let zero_at = |i: usize| b[i] == 0.0 && validity.as_ref().map(|v| v.get(i)).unwrap_or(true);
        if (0..b.len()).any(zero_at) {
            return Err(OdsError::DivisionByZero);
        }
    }
    let f = arith_fn(op);
    let values: Vec<f64> = {
        #[cfg(feature = "parallel")]
        if par {
            let v = a
                .par_iter()
                .zip(b.par_iter())
                .map(|(x, y)| f(*x, *y))
                .collect();
            return Ok(Series::F64 {
                values: Arc::new(v),
                validity,
            });
        }
        let _ = par;
        a.iter().zip(b.iter()).map(|(x, y)| f(*x, *y)).collect()
    };
    Ok(Series::F64 {
        values: Arc::new(values),
        validity,
    })
}

fn f64_arith_scalar(
    a: &[f64],
    s: f64,
    op: ArithOp,
    swapped: bool,
    validity: Option<Bitmap>,
    par: bool,
) -> Result<Series> {
    if op == ArithOp::Div {
        if !swapped && s == 0.0 {
            return Err(OdsError::DivisionByZero);
        }
        if swapped {
            let zero_at =
                |i: usize| a[i] == 0.0 && validity.as_ref().map(|v| v.get(i)).unwrap_or(true);
            if (0..a.len()).any(zero_at) {
                return Err(OdsError::DivisionByZero);
            }
        }
    }
    let f = arith_fn(op);
    let apply = move |x: f64| if swapped { f(s, x) } else { f(x, s) };
    let values: Vec<f64> = {
        #[cfg(feature = "parallel")]
        if par {
            let v = a.par_iter().map(|&x| apply(x)).collect();
            return Ok(Series::F64 {
                values: Arc::new(v),
                validity,
            });
        }
        let _ = par;
        a.iter().map(|&x| apply(x)).collect()
    };
    Ok(Series::F64 {
        values: Arc::new(values),
        validity,
    })
}

fn arith_fn(op: ArithOp) -> fn(f64, f64) -> f64 {
    match op {
        ArithOp::Add => |x, y| x + y,
        ArithOp::Sub => |x, y| x - y,
        ArithOp::Mul => |x, y| x * y,
        ArithOp::Div => |x, y| x / y,
    }
}

// ---------------------------------------------------------------------
// i64 kernels — checked, sequential (checked semantics don't autovectorize
// anyway, and errors must be deterministic).
// ---------------------------------------------------------------------

fn i64_checked(op: ArithOp, x: i64, y: i64) -> Result<i64> {
    match op {
        ArithOp::Add => x
            .checked_add(y)
            .ok_or(OdsError::IntegerOverflow("addition")),
        ArithOp::Sub => x
            .checked_sub(y)
            .ok_or(OdsError::IntegerOverflow("subtraction")),
        ArithOp::Mul => x
            .checked_mul(y)
            .ok_or(OdsError::IntegerOverflow("multiplication")),
        ArithOp::Div => {
            if y == 0 {
                Err(OdsError::DivisionByZero)
            } else {
                x.checked_div(y)
                    .ok_or(OdsError::IntegerOverflow("division"))
            }
        }
    }
}

fn i64_arith(a: &[i64], b: &[i64], op: ArithOp, validity: Option<Bitmap>) -> Result<Series> {
    let mut out = Vec::with_capacity(a.len());
    for i in 0..a.len() {
        if validity.as_ref().map(|v| v.get(i)).unwrap_or(true) {
            out.push(i64_checked(op, a[i], b[i])?);
        } else {
            out.push(0);
        }
    }
    Ok(Series::I64 {
        values: Arc::new(out),
        validity,
    })
}

fn i64_arith_scalar(
    a: &[i64],
    s: i64,
    op: ArithOp,
    swapped: bool,
    validity: Option<Bitmap>,
) -> Result<Series> {
    let mut out = Vec::with_capacity(a.len());
    for (i, &ai) in a.iter().enumerate() {
        if validity.as_ref().map(|v| v.get(i)).unwrap_or(true) {
            let (x, y) = if swapped { (s, ai) } else { (ai, s) };
            out.push(i64_checked(op, x, y)?);
        } else {
            out.push(0);
        }
    }
    Ok(Series::I64 {
        values: Arc::new(out),
        validity,
    })
}

// ---------------------------------------------------------------------
// Comparison helpers
// ---------------------------------------------------------------------

fn cmp_one<T: PartialOrd>(x: T, y: T, op: CmpOp) -> bool {
    match op {
        CmpOp::Eq => x == y,
        CmpOp::Ne => x != y,
        CmpOp::Lt => x < y,
        CmpOp::Le => x <= y,
        CmpOp::Gt => x > y,
        CmpOp::Ge => x >= y,
    }
}

fn cmp_slices<T: PartialOrd + Copy>(a: &[T], b: &[T], op: CmpOp) -> Vec<bool> {
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| cmp_one(x, y, op))
        .collect()
}

fn mirror_cmp(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Lt => CmpOp::Gt,
        CmpOp::Le => CmpOp::Ge,
        CmpOp::Gt => CmpOp::Lt,
        CmpOp::Ge => CmpOp::Le,
        CmpOp::Eq | CmpOp::Ne => op,
    }
}

fn rebuild_with_trailing_nulls<T: Default + Clone>(
    mut vals: Vec<T>,
    total: usize,
    nulls: usize,
    dense: impl Fn(Vec<T>) -> Series,
) -> Series {
    if nulls == 0 {
        return dense(vals);
    }
    vals.extend(std::iter::repeat_n(T::default(), nulls));
    let mut bits = vec![true; total - nulls];
    bits.extend(std::iter::repeat_n(false, nulls));
    match dense(vals) {
        Series::F64 { values, .. } => Series::F64 {
            values,
            validity: Some(Bitmap::from_bools(&bits)),
        },
        Series::I64 { values, .. } => Series::I64 {
            values,
            validity: Some(Bitmap::from_bools(&bits)),
        },
        Series::Bool { values, .. } => Series::Bool {
            values,
            validity: Some(Bitmap::from_bools(&bits)),
        },
        Series::Str { col, .. } => Series::Str {
            col,
            validity: Some(Bitmap::from_bools(&bits)),
        },
    }
}

// ---------------------------------------------------------------------
// Fused elementwise evaluation
// ---------------------------------------------------------------------

/// A pending elementwise chain over F64 series, evaluated in one pass.
///
/// The host language builds these lazily from `a * b + 1.0`-style
/// operator chains instead of materializing a full-length intermediate
/// per operator; forcing evaluates the whole tree chunk by chunk, so
/// intermediates live in cache instead of main memory. Deliberately
/// narrow: Add/Sub/Mul only (infallible IEEE ops — `Div` pre-checks its
/// divisors and must error at its own expression, so it is never
/// deferred), null-free F64 leaves of equal length, and Float scalars.
/// Within that shape the fused result is bit-identical to the eager
/// kernels: each output element performs the same float operations in
/// the same order.
pub mod fuse {
    use super::*;

    /// Elements evaluated per tree walk — small enough that one buffer
    /// per tree level stays in L1.
    pub const CHUNK: usize = 1024;

    #[derive(Clone, Debug)]
    pub enum Expr {
        /// A materialized, null-free F64 column.
        Leaf(Arc<Vec<f64>>),
        /// A broadcast scalar.
        Const(f64),
        /// An infallible elementwise operator (never `Div`).
        Bin(ArithOp, Arc<Expr>, Arc<Expr>),
    }

    impl Expr {
        fn eval_chunk(&self, start: usize, out: &mut [f64]) {
            match self {
                Expr::Leaf(v) => out.copy_from_slice(&v[start..start + out.len()]),
                Expr::Const(c) => out.fill(*c),
                Expr::Bin(op, a, b) => {
                    a.eval_chunk(start, out);
                    let mut buf = [0.0f64; CHUNK];
                    let rhs = &mut buf[..out.len()];
                    b.eval_chunk(start, rhs);
                    match op {
                        ArithOp::Add => {
                            for (o, r) in out.iter_mut().zip(rhs.iter()) {
                                *o += r;
                            }
                        }
                        ArithOp::Sub => {
                            for (o, r) in out.iter_mut().zip(rhs.iter()) {
                                *o -= r;
                            }
                        }
                        ArithOp::Mul => {
                            for (o, r) in out.iter_mut().zip(rhs.iter()) {
                                *o *= r;
                            }
                        }
                        ArithOp::Div => unreachable!("Div is never fused"),
                    }
                }
            }
        }

        /// Materialize the chain: one output allocation, one pass.
        /// Chunks are independent, so the parallel build fans them out.
        pub fn eval(&self, len: usize, par: bool) -> Series {
            let mut values = vec![0.0f64; len];
            #[cfg(feature = "parallel")]
            if par {
                values
                    .par_chunks_mut(CHUNK)
                    .enumerate()
                    .for_each(|(ci, chunk)| self.eval_chunk(ci * CHUNK, chunk));
                return Series::F64 {
                    values: Arc::new(values),
                    validity: None,
                };
            }
            let _ = par;
            for (ci, chunk) in values.chunks_mut(CHUNK).enumerate() {
                self.eval_chunk(ci * CHUNK, chunk);
            }
            Series::F64 {
                values: Arc::new(values),
                validity: None,
            }
        }
    }
}

/// Resolve take indices (negatives from the end, bounds checked) once —
/// `Frame::take` shares one resolution across every column instead of
/// re-resolving per column.
pub(crate) fn resolve_take_indices(idx: &[i64], len: usize) -> Result<Vec<usize>> {
    let mut resolved = Vec::with_capacity(idx.len());
    for &raw in idx {
        let i = if raw < 0 { raw + len as i64 } else { raw };
        if i < 0 || i as usize >= len {
            return Err(OdsError::IndexOutOfBounds {
                index: raw,
                length: len,
            });
        }
        resolved.push(i as usize);
    }
    Ok(resolved)
}

/// The row positions a Bool mask keeps (null mask positions drop) —
/// `Frame::filter` computes this once for every column.
pub(crate) fn mask_keep_indices(mask: &Series) -> Result<Vec<usize>> {
    match mask {
        Series::Bool { values, validity } => Ok((0..values.len())
            .filter(|&i| values[i] && validity.as_ref().map(|v| v.get(i)).unwrap_or(true))
            .collect()),
        _ => Err(OdsError::TypeMismatch(format!(
            "filter mask must be a Bool series, got Series[{}]",
            mask.dtype()
        ))),
    }
}
