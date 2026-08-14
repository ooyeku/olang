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
use olang_ods::plot::{
    BarSeries, HeatScale, PlotOptions, Theme, XyKind, XySeries, ramp_color, render_bar_groups,
    render_bars, render_box, render_heatmap, render_hist, render_xy,
};
use olang_ods::{Scalar, Series};

/// (name, arity) of the plot functions.
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("line", 3),
    ("scatter", 3),
    ("lines", 3),
    ("xy", 2),
    ("area", 3),
    ("bar", 3),
    ("bars", 3),
    ("stacked", 3),
    ("hist", 3),
    ("heatmap", 4),
    ("box", 2),
    ("ramp", 2),
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
            ("theme", Value::String(s)) => {
                opts.theme = match s.as_str() {
                    "light" => Theme::Light,
                    "dark" => Theme::Dark,
                    other => {
                        return Err(format!(
                            "plot: theme must be \"light\" or \"dark\", got \"{}\"",
                            other
                        ));
                    }
                };
            }
            ("theme", other) => {
                return Err(format!(
                    "plot: theme must be a String, got {}",
                    other.type_name()
                ));
            }
            ("responsive", Value::Boolean(b)) => opts.responsive = *b,
            ("interactive", Value::Boolean(b)) => opts.interactive = *b,
            ("vary", Value::Boolean(b)) => opts.vary = *b,
            ("responsive" | "interactive" | "vary", other) => {
                return Err(format!(
                    "plot: {} must be a Bool, got {}",
                    key,
                    other.type_name()
                ));
            }
            ("colors", Value::List(items)) => {
                opts.colors = items
                    .iter()
                    .map(|v| match v {
                        Value::String(c) => Ok(c.as_ref().clone()),
                        other => Err(format!(
                            "plot: colors must be Strings, got {}",
                            other.type_name()
                        )),
                    })
                    .collect::<Result<_, _>>()?;
                if opts.colors.is_empty() {
                    return Err(
                        "plot: colors must not be empty (omit it for the theme palette)"
                            .to_string(),
                    );
                }
            }
            ("colors", other) => {
                return Err(format!(
                    "plot: colors must be a list of CSS colors, got {}",
                    other.type_name()
                ));
            }
            ("scale", Value::String(name)) => {
                opts.scale = HeatScale::parse(name).ok_or_else(|| {
                    format!(
                        "plot: scale must be \"auto\", \"ocean\", \"ember\", \"thermal\", or \"diverging\", got \"{}\"",
                        name
                    )
                })?;
            }
            ("scale", other) => {
                return Err(format!(
                    "plot: scale must be a String, got {}",
                    other.type_name()
                ));
            }
            _ => {
                return Err(format!(
                    "plot: unknown option '{}' (title, x_label, y_label, width, height, theme, responsive, interactive, colors, vary, scale)",
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

/// Category labels from a Series or a plain list; numbers stringify,
/// nulls refuse (a category with no name is a data bug).
fn want_labels(func: &str, arg: &Value) -> Result<Vec<String>, String> {
    match (arg, series_of(arg)) {
        (_, Some(s)) => (0..s.len())
            .map(|i| match s.scalar_at(i) {
                Scalar::Str(v) => Ok(v),
                Scalar::Null => Err(format!("plot.{}: category labels must not be null", func)),
                Scalar::F64(v) => Ok(Value::Float(v).to_string()),
                Scalar::I64(v) => Ok(v.to_string()),
                Scalar::Bool(b) => Ok(b.to_string()),
            })
            .collect(),
        (Value::List(items), None) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.as_ref().clone()),
                Value::Integer(n) => Ok(n.to_string()),
                other => Err(format!(
                    "plot.{}: labels must be Strings, got {}",
                    func,
                    other.type_name()
                )),
            })
            .collect(),
        (other, None) => Err(format!(
            "plot.{}: labels must be a Series or list, got {}",
            func,
            other.type_name()
        )),
    }
}

/// `[[label, series], ...]` pairs — the multi-series argument shape
/// shared by lines, bars, stacked, and box.
fn want_pairs<'a>(func: &str, arg: &'a Value) -> Result<Vec<(String, &'a Series)>, String> {
    let items = match arg {
        Value::List(items) => items,
        other => {
            return Err(format!(
                "plot.{} expects a list of [label, series] pairs, got {}",
                func,
                other.type_name()
            ));
        }
    };
    items
        .iter()
        .map(|pair| match pair {
            Value::List(p) if p.len() == 2 => match (&p[0], series_of(&p[1])) {
                (Value::String(s), Some(y)) => Ok((s.as_ref().clone(), y)),
                _ => Err(format!(
                    "plot.{}: each entry is [label (String), values (Series)]",
                    func
                )),
            },
            other => Err(format!(
                "plot.{}: each entry is [label, series], got {}",
                func,
                other.type_name()
            )),
        })
        .collect()
}

/// Every value of a numeric series, nulls refused — bar-family charts
/// draw absolute quantities, and a bar of unknown height draws a lie.
fn dense_values(func: &str, s: &Series) -> Result<Vec<f64>, String> {
    if s.null_count() > 0 {
        return Err(format!(
            "plot.{}: values must not contain nulls (fill_null or filter first)",
            func
        ));
    }
    (0..s.len())
        .map(|i| match s.scalar_at(i) {
            Scalar::F64(v) => Ok(v),
            Scalar::I64(v) => Ok(v as f64),
            _ => Err(format!(
                "plot.{}: values must be numeric, got a {} series",
                func,
                s.dtype()
            )),
        })
        .collect()
}

/// A row of heatmap cells: a Series or a plain list of numbers.
fn want_row(func: &str, row: &Value) -> Result<Vec<f64>, String> {
    if let Some(s) = series_of(row) {
        return dense_values(func, s);
    }
    match row {
        Value::List(items) => items
            .iter()
            .map(|v| match v {
                Value::Integer(n) => Ok(*n as f64),
                Value::Float(f) => Ok(*f),
                other => Err(format!(
                    "plot.{}: cell values must be numeric, got {}",
                    func,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "plot.{}: each row must be a Series or list of numbers, got {}",
            func,
            other.type_name()
        )),
    }
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
        "line" | "scatter" | "area" => {
            let x = want_series(func, &args, 0)?;
            let y = want_series(func, &args, 1)?;
            let opts = parse_options(&args[2])?;
            let (xs, ys) = xy_points(func, x, y)?;
            let kind = match func {
                "line" => XyKind::Line,
                "area" => XyKind::Area,
                _ => XyKind::Scatter,
            };
            render_xy(
                &[XySeries {
                    label: String::new(),
                    xs,
                    ys,
                    kind,
                    point_colors: Vec::new(),
                }],
                &opts,
            )
            .map_err(e)?
        }
        "lines" => {
            // plot.lines(x, [[label, y], ...], opts) — multi-series.
            let x = want_series(func, &args, 0)?;
            let opts = parse_options(&args[2])?;
            let mut series = Vec::new();
            for (label, y) in want_pairs(func, &args[1])? {
                let (xs, ys) = xy_points(func, x, y)?;
                series.push(XySeries {
                    label,
                    xs,
                    ys,
                    kind: XyKind::Line,
                    point_colors: Vec::new(),
                });
            }
            render_xy(&series, &opts).map_err(e)?
        }
        "xy" => {
            // plot.xy([[label, mark, x, y], ...], opts) — layered marks
            // over shared scales; each entry carries its own x.
            let opts = parse_options(&args[1])?;
            let entries = match &args[0] {
                Value::List(items) => items,
                other => {
                    return Err(format!(
                        "plot.xy expects a list of [label, mark, x, y] entries, got {}",
                        other.type_name()
                    ));
                }
            };
            let mut series = Vec::new();
            for entry in entries.iter() {
                let (label, mark, x, y, point_colors) = match entry {
                    Value::List(p) if p.len() == 4 || p.len() == 5 => {
                        let colors = match p.get(4) {
                            None => Vec::new(),
                            Some(Value::List(cs)) => cs
                                .iter()
                                .map(|c| match c {
                                    Value::String(c) => Ok(c.as_ref().clone()),
                                    other => Err(format!(
                                        "plot.xy: point colors must be Strings, got {}",
                                        other.type_name()
                                    )),
                                })
                                .collect::<Result<_, _>>()?,
                            Some(other) => {
                                return Err(format!(
                                    "plot.xy: the 5th entry element is a list of colors, got {}",
                                    other.type_name()
                                ));
                            }
                        };
                        match (&p[0], &p[1], series_of(&p[2]), series_of(&p[3])) {
                            (Value::String(l), Value::String(m), Some(x), Some(y)) => {
                                (l.as_ref().clone(), m.as_ref().clone(), x, y, colors)
                            }
                            _ => {
                                return Err(
                                    "plot.xy: each entry is [label (String), mark (String), \
                                     x (Series), y (Series), colors?]"
                                        .to_string(),
                                );
                            }
                        }
                    }
                    other => {
                        return Err(format!(
                            "plot.xy: each entry is [label, mark, x, y], got {}",
                            other.type_name()
                        ));
                    }
                };
                let kind = match mark.as_str() {
                    "line" => XyKind::Line,
                    "area" => XyKind::Area,
                    "scatter" | "point" => XyKind::Scatter,
                    other => {
                        return Err(format!(
                            "plot.xy: mark must be \"line\", \"area\", or \"scatter\", got \"{}\"",
                            other
                        ));
                    }
                };
                let (xs, ys) = xy_points(func, x, y)?;
                series.push(XySeries {
                    label,
                    xs,
                    ys,
                    kind,
                    point_colors,
                });
            }
            render_xy(&series, &opts).map_err(e)?
        }
        "bar" => {
            let labels = want_labels(func, &args[0])?;
            let vals = dense_values(func, want_series(func, &args, 1)?)?;
            let opts = parse_options(&args[2])?;
            render_bars(&labels, &vals, &opts).map_err(e)?
        }
        "bars" | "stacked" => {
            // plot.bars/stacked(labels, [[label, values], ...], opts).
            let labels = want_labels(func, &args[0])?;
            let opts = parse_options(&args[2])?;
            let mut series = Vec::new();
            for (label, s) in want_pairs(func, &args[1])? {
                series.push(BarSeries {
                    label,
                    values: dense_values(func, s)?,
                });
            }
            render_bar_groups(&labels, &series, func == "stacked", &opts).map_err(e)?
        }
        "heatmap" => {
            // plot.heatmap(x_labels, y_labels, rows, opts) — rows[r][c].
            let x_labels = want_labels(func, &args[0])?;
            let y_labels = want_labels(func, &args[1])?;
            let rows = match &args[2] {
                Value::List(items) => items
                    .iter()
                    .map(|row| want_row(func, row))
                    .collect::<Result<Vec<_>, _>>()?,
                other => {
                    return Err(format!(
                        "plot.heatmap: rows must be a list of rows, got {}",
                        other.type_name()
                    ));
                }
            };
            let opts = parse_options(&args[3])?;
            render_heatmap(&x_labels, &y_labels, &rows, &opts).map_err(e)?
        }
        "ramp" => {
            // plot.ramp(scale, t) — one color from a named ramp; how
            // olang code (viz's color_by, custom pieces) speaks the
            // same scales the heatmap uses. Dark-theme tuned.
            let name = match &args[0] {
                Value::String(s) => s.as_ref().clone(),
                other => {
                    return Err(format!(
                        "plot.ramp: scale must be a String, got {}",
                        other.type_name()
                    ));
                }
            };
            let scale = HeatScale::parse(&name).ok_or_else(|| {
                format!(
                    "plot.ramp: scale must be \"auto\", \"ocean\", \"ember\", \"thermal\", or \"diverging\", got \"{}\"",
                    name
                )
            })?;
            let t = match &args[1] {
                Value::Float(f) => *f,
                Value::Integer(n) => *n as f64,
                other => {
                    return Err(format!(
                        "plot.ramp: t must be a number in [0, 1], got {}",
                        other.type_name()
                    ));
                }
            };
            ramp_color(scale, Theme::Dark, t)
        }
        "box" => {
            // plot.box([[label, values], ...], opts) — nulls dropped
            // (a distribution summary tolerates missing observations).
            let opts = parse_options(&args[1])?;
            let mut series = Vec::new();
            for (label, s) in want_pairs(func, &args[0])? {
                let values: Vec<f64> = (0..s.len())
                    .filter_map(|i| match s.scalar_at(i) {
                        Scalar::F64(v) => Some(Ok(v)),
                        Scalar::I64(v) => Some(Ok(v as f64)),
                        Scalar::Null => None,
                        _ => Some(Err(format!(
                            "plot.box: values must be numeric, got a {} series",
                            s.dtype()
                        ))),
                    })
                    .collect::<Result<_, _>>()?;
                series.push(BarSeries { label, values });
            }
            render_box(&series, &opts).map_err(e)?
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
