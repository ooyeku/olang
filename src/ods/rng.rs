//! `random.rng(seed)`: a generator of its own, and the columns it draws.
//!
//! The value is a PCG64 stream behind a lock (`olang_ods::random`). Its
//! methods are the functions of `random.Rng` (`g.normal(1000, 0.0, 1.0)`
//! is `random.Rng.normal(g, 1000, 0.0, 1.0)`), so they never shadow the
//! global stream's own `random.uniform` or `random.choice`.
//!
//! Every draw with a count is a Series, drawn in one native pass (in
//! parallel blocks past the elementwise threshold, the values the same on
//! any number of threads); without a count it is one value. The kernels
//! that take `seed:` take `rng:` too (`with_source`): a generator, or an
//! Int seed.

use super::series::{make_series_value, series_from_list, series_of};
use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use olang_ods::random::{self as kernels, Pcg64};
use olang_ods::{ParClass, Series};
use std::any::Any;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct OdsRng {
    state: Mutex<Pcg64>,
    /// The seed it was made from (a spawned child shows its parent's).
    seed: Option<u64>,
}

impl OdsRng {
    pub fn new(rng: Pcg64, seed: Option<u64>) -> OdsRng {
        OdsRng {
            state: Mutex::new(rng),
            seed,
        }
    }

    pub fn into_value(self) -> Value {
        Value::Native(NativeHandle::new(self))
    }

    /// Run `f` on the generator's state.
    pub fn with<R>(&self, f: impl FnOnce(&mut Pcg64) -> R) -> R {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut g)
    }
}

impl NativeObject for OdsRng {
    fn module(&self) -> &'static str {
        "random.Rng"
    }

    fn type_name(&self) -> &'static str {
        "Rng"
    }

    fn display(&self) -> String {
        match self.seed {
            Some(s) => format!("<random.Rng seed {}>", s),
            None => "<random.Rng>".to_string(),
        }
    }

    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        std::ptr::eq(
            self as *const OdsRng as *const u8,
            other.as_any() as *const dyn Any as *const u8,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn rng_of(v: &Value) -> Option<&OdsRng> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<OdsRng>(),
        _ => None,
    }
}

/// A seed from the `random` module's global stream: what a kernel with
/// neither `seed:` nor `rng:` uses, so `random.seed(k)` still governs it.
pub fn global_seed() -> u64 {
    let u = crate::stdlib::random::draw_uniforms(1)[0];
    (u * 9_007_199_254_740_992.0) as u64
}

/// Run `f` on the generator a kernel's `rng:`/`seed:` names: a generator
/// (its own state, which moves), an Int seed (a fresh one), or nothing
/// (a fresh one seeded from the global stream).
pub fn with_source<R>(
    func: &str,
    rng: Option<&Value>,
    seed: Option<&Value>,
    f: impl FnOnce(&mut Pcg64) -> R,
) -> Result<R, String> {
    fn given(v: Option<&Value>) -> Option<&Value> {
        v.filter(|v| !matches!(v, Value::Unit))
    }
    match (given(rng), given(seed)) {
        (Some(_), Some(_)) => Err(format!("{}: give seed or rng, not both", func)),
        (Some(v), None) | (None, Some(v)) => match v {
            Value::Integer(s) => Ok(f(&mut Pcg64::new(*s as u64))),
            other => match rng_of(other) {
                Some(g) => Ok(g.with(f)),
                None => Err(format!(
                    "{}: rng must be a generator (random.rng(seed)) or an Int seed, got {}",
                    func,
                    other.type_name()
                )),
            },
        },
        (None, None) => Ok(f(&mut Pcg64::new(global_seed()))),
    }
}

/// A seed for a kernel that seeds itself, drawn from `rng:` (a generator,
/// which moves by one step, or an Int seed as it is).
pub fn seed_from(func: &str, rng: &Value) -> Result<u64, String> {
    with_source(func, Some(rng), None, |g| g.draw_seed())
}

/// A draw fans out past the sort threshold (100,000 values): a normal
/// costs about five nanoseconds, and the blocks give the same values on
/// any number of threads.
fn par(n: usize) -> bool {
    super::series::par(ParClass::Sort, n)
}

// ---------------------------------------------------------------------
// random.rng and random.Rng.*
// ---------------------------------------------------------------------

/// The methods, with the fewest arguments each takes (the generator
/// counted).
pub const METHODS: &[(&str, usize)] = &[
    ("normal", 1),
    ("uniform", 1),
    ("integers", 3),
    ("poisson", 1),
    ("lognormal", 1),
    ("binomial", 3),
    ("choice", 2),
    ("shuffle", 2),
    ("permutation", 2),
    ("spawn", 2),
];

fn num(func: &str, what: &str, v: Option<&Value>, default: f64) -> Result<f64, String> {
    match v {
        None | Some(Value::Unit) => Ok(default),
        Some(Value::Float(x)) => Ok(*x),
        Some(Value::Integer(x)) => Ok(*x as f64),
        Some(other) => Err(format!(
            "random.Rng.{}: {} must be a number, got {}",
            func,
            what,
            other.type_name()
        )),
    }
}

fn int(func: &str, what: &str, v: Option<&Value>) -> Result<i64, String> {
    match v {
        Some(Value::Integer(x)) => Ok(*x),
        Some(other) => Err(format!(
            "random.Rng.{}: {} must be an Int, got {}",
            func,
            what,
            other.type_name()
        )),
        None => Err(format!("random.Rng.{}: missing {}", func, what)),
    }
}

/// The count: `None` for one value.
fn count(func: &str, v: Option<&Value>) -> Result<Option<usize>, String> {
    match v {
        None | Some(Value::Unit) => Ok(None),
        Some(Value::Integer(n)) if *n >= 0 => Ok(Some(*n as usize)),
        Some(other) => Err(format!(
            "random.Rng.{}: n must be a non-negative Int (or () for one value), got {}",
            func, other
        )),
    }
}

fn floats(n: Option<usize>, v: Vec<f64>) -> Value {
    match n {
        Some(_) => make_series_value(Series::from_f64(v)),
        None => Value::Float(v[0]),
    }
}

fn ints(n: Option<usize>, v: Vec<i64>) -> Value {
    match n {
        Some(_) => make_series_value(Series::from_i64(v)),
        None => Value::Integer(v[0]),
    }
}

/// `random.rng(seed?)`.
pub fn make(args: &[Value]) -> Result<Value, String> {
    match args.first() {
        None | Some(Value::Unit) => {
            let s = global_seed();
            Ok(OdsRng::new(Pcg64::new(s), Some(s)).into_value())
        }
        Some(Value::Integer(s)) => {
            Ok(OdsRng::new(Pcg64::new(*s as u64), Some(*s as u64)).into_value())
        }
        Some(other) => Err(format!(
            "random.rng: the seed must be an Int, got {}",
            other.type_name()
        )),
    }
}

/// The values a choice or shuffle picks from, and how picks go back.
enum Pool<'a> {
    Series(&'a Series),
    List(&'a [Value]),
}

impl Pool<'_> {
    fn len(&self) -> usize {
        match self {
            Pool::Series(s) => s.len(),
            Pool::List(l) => l.len(),
        }
    }

    /// The picks, as a Series when the pool is one (or a list a Series
    /// can hold), else a List.
    fn take(&self, idx: &[usize]) -> Result<Value, String> {
        match self {
            Pool::Series(s) => s
                .take(&Series::from_i64(idx.iter().map(|&i| i as i64).collect()))
                .map(make_series_value)
                .map_err(|e| e.to_string()),
            Pool::List(l) => {
                let picked: Vec<Value> = idx.iter().map(|&i| l[i].clone()).collect();
                Ok(match series_from_list(&picked) {
                    Ok(s) => make_series_value(s),
                    Err(_) => Value::List(Arc::new(picked)),
                })
            }
        }
    }

    fn one(&self, i: usize) -> Value {
        match self {
            Pool::Series(s) => super::series::scalar_to_value(s.scalar_at(i)),
            Pool::List(l) => l[i].clone(),
        }
    }
}

fn pool<'a>(func: &str, v: Option<&'a Value>) -> Result<Pool<'a>, String> {
    match v {
        Some(Value::List(items)) => Ok(Pool::List(&items[..])),
        Some(v) if series_of(v).is_some() => Ok(Pool::Series(series_of(v).expect("checked"))),
        other => Err(format!(
            "random.Rng.{}: the values must be a List or a Series, got {}",
            func,
            other.map(|v| v.type_name()).unwrap_or_default()
        )),
    }
}

fn weights(func: &str, v: Option<&Value>) -> Result<Option<Vec<f64>>, String> {
    match v {
        None | Some(Value::Unit) => Ok(None),
        Some(Value::List(items)) => items
            .iter()
            .map(|w| match w {
                Value::Float(x) => Ok(*x),
                Value::Integer(x) => Ok(*x as f64),
                other => Err(format!(
                    "random.Rng.{}: weights must be numbers, got {}",
                    func,
                    other.type_name()
                )),
            })
            .collect::<Result<Vec<f64>, String>>()
            .map(Some),
        Some(v) => match series_of(v) {
            Some(s) => {
                if s.null_count() > 0 {
                    return Err(format!("random.Rng.{}: the weights hold a null", func));
                }
                s.cast(olang_ods::DType::F64, &crate::ast::format_float)
                    .map_err(|e| e.to_string())
                    .map(|c| match c {
                        Series::F64 { values, .. } => Some(values.as_ref().clone()),
                        _ => None,
                    })
            }
            None => Err(format!(
                "random.Rng.{}: weights must be a List or a Series of numbers, got {}",
                func,
                v.type_name()
            )),
        },
    }
}

/// `random.Rng.<func>(g, …)`.
pub fn method(func: &str, args: &[Value]) -> Result<Value, String> {
    let Some(g) = args.first().and_then(rng_of) else {
        return Err(format!(
            "random.Rng.{}: the first argument must be a generator (random.rng(seed)), got {}",
            func,
            args.first().map(|v| v.type_name()).unwrap_or_default()
        ));
    };
    let a = |i: usize| args.get(i);
    match func {
        "normal" | "uniform" | "lognormal" => {
            let n = count(func, a(1))?;
            let (p1, p2) = match func {
                "normal" => (num(func, "mean", a(2), 0.0)?, num(func, "std", a(3), 1.0)?),
                "uniform" => (num(func, "low", a(2), 0.0)?, num(func, "high", a(3), 1.0)?),
                _ => (
                    num(func, "mean", a(2), 0.0)?,
                    num(func, "sigma", a(3), 1.0)?,
                ),
            };
            if func == "uniform" && (p1.is_nan() || p2.is_nan() || p1 >= p2) {
                return Err(format!(
                    "random.Rng.uniform: low must be below high, got {} and {}",
                    p1, p2
                ));
            }
            if func != "uniform" && (p2.is_nan() || p2 < 0.0) {
                return Err(format!(
                    "random.Rng.{}: the spread must be ≥ 0, got {}",
                    func, p2
                ));
            }
            let k = n.unwrap_or(1);
            let v = g.with(|r| match func {
                "normal" => kernels::normal(r, k, p1, p2, par(k)),
                "uniform" => kernels::uniform(r, k, p1, p2, par(k)),
                _ => kernels::lognormal(r, k, p1, p2, par(k)),
            });
            Ok(floats(n, v))
        }
        "integers" => {
            let n = count(func, a(1))?;
            let lo = int(func, "low", a(2))?;
            let hi = int(func, "high", a(3))?;
            if lo >= hi {
                return Err(format!(
                    "random.Rng.integers: low must be below high (high is excluded), got {} and {}",
                    lo, hi
                ));
            }
            let k = n.unwrap_or(1);
            Ok(ints(n, g.with(|r| kernels::integers(r, k, lo, hi, par(k)))))
        }
        "poisson" => {
            let n = count(func, a(1))?;
            let lam = num(func, "lam", a(2), 1.0)?;
            if !(lam >= 0.0 && lam.is_finite() && lam < 1e15) {
                return Err(format!("random.Rng.poisson: lam must be ≥ 0, got {}", lam));
            }
            let k = n.unwrap_or(1);
            Ok(ints(n, g.with(|r| kernels::poisson(r, k, lam, par(k)))))
        }
        "binomial" => {
            let n = count(func, a(1))?;
            let trials = int(func, "trials", a(2))?;
            let p = num(func, "p", a(3), f64::NAN)?;
            if trials < 0 || !(0.0..=1.0).contains(&p) {
                return Err(format!(
                    "random.Rng.binomial: trials must be ≥ 0 and p in [0, 1], got {} and {}",
                    trials, p
                ));
            }
            let k = n.unwrap_or(1);
            Ok(ints(
                n,
                g.with(|r| kernels::binomial(r, k, trials, p, par(k))),
            ))
        }
        "choice" => {
            let values = pool(func, a(1))?;
            let n = count(func, a(2))?;
            let w = weights(func, a(3))?;
            let replace = match a(4) {
                None | Some(Value::Unit) => true,
                Some(Value::Boolean(b)) => *b,
                Some(other) => {
                    return Err(format!(
                        "random.Rng.choice: replace must be a Bool, got {}",
                        other.type_name()
                    ));
                }
            };
            let k = n.unwrap_or(1);
            let idx = g
                .with(|r| kernels::choice(r, values.len(), k, w.as_deref(), replace, par(k)))
                .map_err(|e| format!("random.Rng.{}", e))?;
            match n {
                Some(_) => values.take(&idx),
                None => Ok(values.one(idx[0])),
            }
        }
        "shuffle" => {
            let values = pool(func, a(1))?;
            let idx = g.with(|r| kernels::permutation(r, values.len()));
            match values {
                Pool::List(l) => Ok(Value::List(Arc::new(
                    idx.iter().map(|&i| l[i].clone()).collect(),
                ))),
                p => p.take(&idx),
            }
        }
        "permutation" => {
            let n = match count(func, a(1))? {
                Some(n) => n,
                None => return Err("random.Rng.permutation: n must be an Int".to_string()),
            };
            let idx = g.with(|r| kernels::permutation(r, n));
            Ok(make_series_value(Series::from_i64(
                idx.into_iter().map(|i| i as i64).collect(),
            )))
        }
        "spawn" => {
            let k = match count(func, a(1))? {
                Some(k) => k,
                None => return Err("random.Rng.spawn: k must be an Int".to_string()),
            };
            let kids = g.with(|r| r.spawn(k));
            Ok(Value::List(Arc::new(
                kids.into_iter()
                    .map(|r| OdsRng::new(r, g.seed).into_value())
                    .collect(),
            )))
        }
        other => Err(format!("unknown random function: random.Rng.{}", other)),
    }
}
