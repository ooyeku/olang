//! The `plot` namespace: charts as SVG text from Series data.
//!
//! Every function returns a complete SVG document as a String — write
//! it with `fs.write_file("chart.svg", svg)`, serve it over `http`, or
//! hand it to the playground. Options ride in one map:
//! `#{ title: "...", x_label: "...", y_label: "...", width: 720,
//! height: 440 }` — pass `#{}` for the defaults. Unknown option keys
//! are errors (they are always typos).
//!
//! Null handling: xy plots drop a point when either coordinate is
//! null; bars refuse null values (a bar of unknown height draws lies).

use super::series::series_of;
use crate::ast::Value;
use olang_ods::plot::{PlotOptions, XyKind, XySeries, render_bars, render_hist, render_xy};
use olang_ods::{Scalar, Series};

/// (name, arity) of the plot functions.
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("line", 3),
    ("scatter", 3),
    ("lines", 3),
    ("bar", 3),
    ("hist", 3),
];

pub fn handles(func: &str) -> bool {
    FUNCTIONS.iter().any(|(n, _)| *n == func)
}

fn e(err: olang_ods::OdsError) -> String {
    err.to_string()
}

fn parse_options(value: &Value) -> Result<PlotOptions, String> {
    let map = match value {
        Value::Map(m) => m,
        other => {
            return Err(format!(
                "plot: options must be a map (pass #{{}} for defaults), got {}",
                other.type_name()
            ));
        }
    };
    let mut opts = PlotOptions::default();
    for (key, v) in map.iter() {
        match (key.as_str(), v) {
            ("title", Value::String(s)) => opts.title = s.as_ref().clone(),
            ("x_label", Value::String(s)) => opts.x_label = s.as_ref().clone(),
            ("y_label", Value::String(s)) => opts.y_label = s.as_ref().clone(),
            ("width", Value::Integer(n)) if (100..=4000).contains(n) => opts.width = *n as u32,
            ("height", Value::Integer(n)) if (100..=4000).contains(n) => opts.height = *n as u32,
            ("width" | "height", other) => {
                return Err(format!(
                    "plot: {} must be an Int between 100 and 4000, got {}",
                    key, other
                ));
            }
            ("title" | "x_label" | "y_label", other) => {
                return Err(format!(
                    "plot: {} must be a String, got {}",
                    key,
                    other.type_name()
                ));
            }
            _ => {
                return Err(format!(
                    "plot: unknown option '{}' (title, x_label, y_label, width, height)",
                    key
                ));
            }
        }
    }
    Ok(opts)
}

fn want_series<'a>(func: &str, args: &'a [Value], idx: usize) -> Result<&'a Series, String> {
    args.get(idx).and_then(series_of).ok_or_else(|| {
        format!(
            "plot.{}: argument {} must be a Series, got {}",
            func,
            idx + 1,
            args.get(idx).map(|v| v.type_name()).unwrap_or_default()
        )
    })
}

/// Paired xy points with nulls dropped pairwise.
fn xy_points(func: &str, x: &Series, y: &Series) -> Result<(Vec<f64>, Vec<f64>), String> {
    if x.len() != y.len() {
        return Err(format!(
            "plot.{}: x and y lengths differ ({} vs {})",
            func,
            x.len(),
            y.len()
        ));
    }
    let mut xs = Vec::with_capacity(x.len());
    let mut ys = Vec::with_capacity(y.len());
    for i in 0..x.len() {
        let (xv, yv) = (x.scalar_at(i), y.scalar_at(i));
        let xf = match xv {
            Scalar::F64(v) => v,
            Scalar::I64(v) => v as f64,
            Scalar::Null => continue,
            _ => {
                return Err(format!(
                    "plot.{}: coordinates must be numeric, got a {} series",
                    func,
                    x.dtype()
                ));
            }
        };
        let yf = match yv {
            Scalar::F64(v) => v,
            Scalar::I64(v) => v as f64,
            Scalar::Null => continue,
            _ => {
                return Err(format!(
                    "plot.{}: coordinates must be numeric, got a {} series",
                    func,
                    y.dtype()
                ));
            }
        };
        xs.push(xf);
        ys.push(yf);
    }
    Ok((xs, ys))
}

pub fn dispatch(func: &str, args: Vec<Value>) -> Result<Value, String> {
    let expected = FUNCTIONS
        .iter()
        .find(|(n, _)| *n == func)
        .map(|(_, a)| *a)
        .expect("caller checked membership");
    if args.len() != expected {
        return Err(format!(
            "plot.{} expects {} arguments, got {}",
            func,
            expected,
            args.len()
        ));
    }
    let svg = match func {
        "line" | "scatter" => {
            let x = want_series(func, &args, 0)?;
            let y = want_series(func, &args, 1)?;
            let opts = parse_options(&args[2])?;
            let (xs, ys) = xy_points(func, x, y)?;
            let kind = if func == "line" {
                XyKind::Line
            } else {
                XyKind::Scatter
            };
            render_xy(
                kind,
                &[XySeries {
                    label: String::new(),
                    xs,
                    ys,
                }],
                &opts,
            )
            .map_err(e)?
        }
        "lines" => {
            // plot.lines(x, [[label, y], ...], opts) — multi-series.
            let x = want_series(func, &args, 0)?;
            let opts = parse_options(&args[2])?;
            let pairs = match &args[1] {
                Value::List(items) => items,
                other => {
                    return Err(format!(
                        "plot.lines expects a list of [label, series] pairs, got {}",
                        other.type_name()
                    ));
                }
            };
            let mut series = Vec::with_capacity(pairs.len());
            for pair in pairs.iter() {
                let (label, y) = match pair {
                    Value::List(p) if p.len() == 2 => match (&p[0], series_of(&p[1])) {
                        (Value::String(s), Some(y)) => (s.as_ref().clone(), y),
                        _ => {
                            return Err("plot.lines: each entry is [label (String), y (Series)]"
                                .to_string());
                        }
                    },
                    other => {
                        return Err(format!(
                            "plot.lines: each entry is [label, series], got {}",
                            other.type_name()
                        ));
                    }
                };
                let (xs, ys) = xy_points(func, x, y)?;
                series.push(XySeries { label, xs, ys });
            }
            render_xy(XyKind::Line, &series, &opts).map_err(e)?
        }
        "bar" => {
            let labels: Vec<String> = match (&args[0], args.first().and_then(series_of)) {
                (_, Some(s)) => (0..s.len())
                    .map(|i| match s.scalar_at(i) {
                        Scalar::Str(v) => Ok(v),
                        Scalar::Null => {
                            Err("plot.bar: category labels must not be null".to_string())
                        }
                        other => Ok(match other {
                            Scalar::F64(v) => Value::Float(v).to_string(),
                            Scalar::I64(v) => v.to_string(),
                            Scalar::Bool(b) => b.to_string(),
                            _ => unreachable!(),
                        }),
                    })
                    .collect::<Result<_, _>>()?,
                (Value::List(items), None) => items
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => Ok(s.as_ref().clone()),
                        Value::Integer(n) => Ok(n.to_string()),
                        other => Err(format!(
                            "plot.bar: labels must be Strings, got {}",
                            other.type_name()
                        )),
                    })
                    .collect::<Result<_, _>>()?,
                (other, None) => {
                    return Err(format!(
                        "plot.bar: labels must be a Series or list, got {}",
                        other.type_name()
                    ));
                }
            };
            let values = want_series(func, &args, 1)?;
            if values.null_count() > 0 {
                return Err(
                    "plot.bar: values must not contain nulls (fill_null or filter first)"
                        .to_string(),
                );
            }
            let opts = parse_options(&args[2])?;
            let vals: Vec<f64> = (0..values.len())
                .map(|i| match values.scalar_at(i) {
                    Scalar::F64(v) => Ok(v),
                    Scalar::I64(v) => Ok(v as f64),
                    _ => Err(format!(
                        "plot.bar: values must be numeric, got a {} series",
                        values.dtype()
                    )),
                })
                .collect::<Result<_, _>>()?;
            render_bars(&labels, &vals, &opts).map_err(e)?
        }
        "hist" => {
            let s = want_series(func, &args, 0)?;
            let bins = match &args[1] {
                Value::Integer(n) if *n > 0 => *n as usize,
                other => {
                    return Err(format!(
                        "plot.hist: bins must be a positive Int, got {}",
                        other
                    ));
                }
            };
            let opts = parse_options(&args[2])?;
            let vals: Vec<f64> = (0..s.len())
                .filter_map(|i| match s.scalar_at(i) {
                    Scalar::F64(v) => Some(Ok(v)),
                    Scalar::I64(v) => Some(Ok(v as f64)),
                    Scalar::Null => None,
                    _ => Some(Err(format!(
                        "plot.hist: values must be numeric, got a {} series",
                        s.dtype()
                    ))),
                })
                .collect::<Result<_, _>>()?;
            render_hist(&vals, bins, &opts).map_err(e)?
        }
        _ => unreachable!("caller checked membership"),
    };
    Ok(Value::String(std::sync::Arc::new(svg)))
}
