//! `plot.gg` and `plot.gg_scene`: a grammar-of-graphics spec (the map the
//! embedded `gg` module builds) read into olang-ods's `gg::Spec` and
//! rendered — to SVG text, or to the scene a host draws natively.
//!
//! A spec is a map with `"$gg": 1`: `data` (a Frame), `mapping`
//! (aesthetic → column name, a Series, or `"after_stat(name)"`),
//! `layers` (each `geom`, `stat`, `position`, `mapping`, `data`,
//! `inherit`, `params`, `legend`), `scales` (each `aes`, `kind`,
//! `params`), `facet`, `coord`, `labels` and `theme`. The kernel draws a
//! value that is one (repl_serve's encode) through `scene_of`.

use super::frame::frame_of;
use super::series::series_of;
use crate::ast::Value;
use olang_ods::gg::{
    Aes, Appearance, Column, CoordSpec, Data, FacetSpec, Layer, Param, Params, RenderOptions,
    Scene, ScaleSpec, Source, Spec,
};
use olang_ods::{Scalar, Series};

/// The key that marks a map as a gg plot.
pub const MARK: &str = "$gg";

/// Whether a value is a gg plot spec.
pub fn is_spec(v: &Value) -> bool {
    matches!(v, Value::Map(m) if m.contains_key(MARK))
}

fn s(v: &str) -> Value {
    Value::String(std::sync::Arc::new(v.to_string()))
}

fn get<'a>(m: &'a crate::ast::ValueMap, k: &str) -> Option<&'a Value> {
    m.get(k).filter(|v| !matches!(v, Value::Unit))
}

fn text(v: &Value, what: &str) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.as_ref().clone()),
        other => Err(format!("gg: {} must be a String, got {}", what, other.type_name())),
    }
}

fn texts(v: &Value, what: &str) -> Result<Vec<String>, String> {
    match v {
        Value::List(items) => items.iter().map(|i| text(i, what)).collect(),
        Value::String(s) => Ok(vec![s.as_ref().clone()]),
        other => Err(format!("gg: {} must be a list of Strings, got {}", what, other.type_name())),
    }
}

fn column_of(series: &Series) -> Column {
    match series {
        // A date is time on an axis, in seconds.
        Series::Date { values, .. } => Column::Time(
            (0..values.len())
                .map(|i| {
                    if series.validity().is_none_or(|v| v.get(i)) {
                        values[i] as f64 * 86400.0
                    } else {
                        f64::NAN
                    }
                })
                .collect(),
            false,
        ),
        Series::Datetime { values, .. } => Column::Time(
            (0..values.len())
                .map(|i| {
                    if series.validity().is_none_or(|v| v.get(i)) {
                        values[i] as f64 / 1e6
                    } else {
                        f64::NAN
                    }
                })
                .collect(),
            true,
        ),
        // A Category keeps its levels: their order is the scale's.
        Series::Cat { levels, .. } => {
            let levels: Vec<String> = levels.iter().map(str::to_string).collect();
            let texts = match column_of(&series.key_view()) {
                Column::Str(v) => v,
                _ => Vec::new(),
            };
            Column::Cat(texts, std::sync::Arc::new(levels))
        }
        Series::F64 { .. } | Series::I64 { .. } => Column::Num(
            (0..series.len())
                .map(|i| match series.scalar_at(i) {
                    Scalar::F64(v) => v,
                    Scalar::I64(v) => v as f64,
                    _ => f64::NAN,
                })
                .collect(),
        ),
        Series::Bool { .. } | Series::Str { .. } => Column::Str(
            (0..series.len())
                .map(|i| match series.scalar_at(i) {
                    Scalar::Str(s) => Some(s),
                    Scalar::Bool(b) => Some(b.to_string()),
                    _ => None,
                })
                .collect(),
        ),
    }
}

/// A Frame's columns (only those a mapping or a facet names, read once).
fn data_of(v: &Value, wanted: &[String]) -> Result<Data, String> {
    let f = frame_of(v).ok_or_else(|| {
        format!(
            "gg: data must be a Frame (ods.read_csv, ods.frame, ods.frame_from_records), got {}",
            v.type_name()
        )
    })?;
    let names: Vec<String> = f.names().to_vec();
    let mut out_names = Vec::new();
    let mut cols = Vec::new();
    for (i, n) in names.iter().enumerate() {
        if wanted.iter().any(|w| w == n) {
            out_names.push(n.clone());
            cols.push(column_of(&f.columns()[i]));
        }
    }
    let mut d = Data::new(out_names, cols);
    d.n = f.columns().first().map(|c| c.len()).unwrap_or(0);
    // every column's name, for the error that names what a frame has
    for n in names {
        if !d.names.contains(&n) {
            d.names.push(n);
            d.cols.push(Column::Num(vec![]));
        }
    }
    Ok(d)
}

fn param_of(v: &Value) -> Option<Param> {
    Some(match v {
        Value::Integer(i) => Param::Num(*i as f64),
        Value::Float(f) => Param::Num(*f),
        Value::Boolean(b) => Param::Bool(*b),
        Value::String(s) => Param::Str(s.as_ref().clone()),
        Value::List(items) | Value::Tuple(items) => {
            if items.iter().all(|i| matches!(i, Value::Integer(_) | Value::Float(_) | Value::Unit)) {
                Param::Nums(
                    items
                        .iter()
                        .map(|i| match i {
                            Value::Integer(n) => *n as f64,
                            Value::Float(f) => *f,
                            _ => f64::NAN,
                        })
                        .collect(),
                )
            } else {
                Param::Strs(
                    items
                        .iter()
                        .map(|i| match i {
                            Value::String(s) => s.as_ref().clone(),
                            other => other.to_string(),
                        })
                        .collect(),
                )
            }
        }
        Value::Map(m) => Param::Pairs(
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        match v {
                            Value::String(s) => s.as_ref().clone(),
                            other => other.to_string(),
                        },
                    )
                })
                .collect(),
        ),
        _ => return None,
    })
}

fn params_of(v: Option<&Value>) -> Result<Params, String> {
    let mut p = Params::new();
    match v {
        None => {}
        Some(Value::Map(m)) => {
            for (k, v) in m.iter() {
                if let Some(x) = param_of(v) {
                    p.insert(k.clone(), x);
                }
            }
        }
        Some(other) => return Err(format!("gg: params must be a map, got {}", other.type_name())),
    }
    Ok(p)
}

/// `after_stat(name)` or `..name..`: a stat's variable.
fn stat_name(s: &str) -> Option<String> {
    let t = s.trim();
    if let Some(r) = t.strip_prefix("after_stat(").and_then(|r| r.strip_suffix(')')) {
        return Some(r.trim().to_string());
    }
    if t.len() > 4 && t.starts_with("..") && t.ends_with("..") {
        return Some(t[2..t.len() - 2].to_string());
    }
    None
}

/// `constant(text)`: a String constant mapping (gg.constant).
fn constant_text(s: &str) -> Option<String> {
    s.trim().strip_prefix("constant(").and_then(|r| r.strip_suffix(')')).map(str::to_string)
}

fn mapping_of(v: Option<&Value>, n_hint: usize) -> Result<Vec<(Aes, Source)>, String> {
    let mut out = Vec::new();
    let Some(v) = v else { return Ok(out) };
    let Value::Map(m) = v else {
        return Err(format!("gg: a mapping must be gg.aes(…) (a map), got {}", v.type_name()));
    };
    for (k, v) in m.iter() {
        if matches!(v, Value::Unit) {
            continue;
        }
        let a = Aes::parse(k).ok_or_else(|| {
            format!(
                "gg: unknown aesthetic \"{}\" (x, y, color, fill, size, shape, alpha, linetype, group, label, xmin, xmax, ymin, ymax, xend, yend, weight, sample)",
                k
            )
        })?;
        let src = match v {
            Value::String(s) => match (stat_name(s), constant_text(s)) {
                (Some(n), _) => Source::Stat(n),
                (None, Some(c)) => Source::Const(k.clone(), Column::Str(vec![Some(c)])),
                (None, None) => Source::Col(s.as_ref().clone()),
            },
            // a number or a Bool: one value for every row (ggplot's aes(group = 1))
            Value::Integer(i) => Source::Const(k.clone(), Column::Num(vec![*i as f64])),
            Value::Float(f) => Source::Const(k.clone(), Column::Num(vec![*f])),
            Value::Boolean(b) => Source::Const(k.clone(), Column::Str(vec![Some(b.to_string())])),
            other => match series_of(other) {
                Some(series) => Source::Values(k.clone(), column_of(series)),
                None => match other {
                    Value::List(items) => {
                        let series = super::series::series_from_list(items).map_err(|e| format!("gg: {} = a list: {}", k, e))?;
                        Source::Values(k.clone(), column_of(&series))
                    }
                    _ => {
                        return Err(format!(
                            "gg: aesthetic {} maps a column name (a String), a Series, after_stat(\"…\") or a constant (a number, a Bool, gg.constant(…)), got {} (a constant look goes to the geom: gg.point(color: \"red\"))",
                            k,
                            other.type_name()
                        ));
                    }
                },
            },
        };
        let _ = n_hint;
        out.push((a, src));
    }
    Ok(out)
}

fn columns_named(m: &[(Aes, Source)], into: &mut Vec<String>) {
    for (_, s) in m {
        if let Source::Col(n) = s {
            into.push(n.clone());
        }
    }
}

/// A spec map read into the renderer's spec.
pub fn spec_of(v: &Value) -> Result<Spec, String> {
    let Value::Map(m) = v else {
        return Err(format!("gg: expects a plot (gg.plot(…)), got {}", v.type_name()));
    };
    if !m.contains_key(MARK) {
        return Err("gg: expects a plot made by gg.plot(…)".to_string());
    }
    let mut spec = Spec {
        mapping: mapping_of(get(m, "mapping"), 0)?,
        ..Default::default()
    };
    // which columns to read: every name a mapping or the facet says
    let mut wanted: Vec<String> = Vec::new();
    columns_named(&spec.mapping, &mut wanted);
    let facet = match get(m, "facet") {
        Some(Value::Map(f)) => {
            let kind = get(f, "kind").map(|k| text(k, "facet kind")).transpose()?.unwrap_or_else(|| "wrap".into());
            let rows = get(f, "rows").map(|v| texts(v, "facet rows")).transpose()?.unwrap_or_default();
            let cols = get(f, "cols").map(|v| texts(v, "facet cols")).transpose()?.unwrap_or_default();
            let num = |k: &str| match get(f, k) {
                Some(Value::Integer(i)) if *i > 0 => Some(*i as usize),
                _ => None,
            };
            wanted.extend(rows.iter().cloned());
            wanted.extend(cols.iter().cloned());
            Some(FacetSpec {
                kind,
                rows,
                cols,
                ncol: num("ncol"),
                nrow: num("nrow"),
                scales: get(f, "scales").map(|v| text(v, "facet scales")).transpose()?.unwrap_or_else(|| "fixed".into()),
            })
        }
        _ => None,
    };
    let mut layers_v: Vec<(Layer, Option<&Value>)> = Vec::new();
    if let Some(Value::List(ls)) = get(m, "layers") {
        for l in ls.iter() {
            let Value::Map(lm) = l else {
                return Err(format!("gg: a layer must be a map, got {}", l.type_name()));
            };
            let geom = get(lm, "geom").map(|g| text(g, "geom")).transpose()?.ok_or("gg: a layer needs a geom")?;
            let mapping = mapping_of(get(lm, "mapping"), 0)?;
            columns_named(&mapping, &mut wanted);
            let layer = Layer {
                geom,
                stat: get(lm, "stat").map(|g| text(g, "stat")).transpose()?,
                position: get(lm, "position").map(|g| text(g, "position")).transpose()?,
                mapping,
                data: None,
                inherit: !matches!(get(lm, "inherit"), Some(Value::Boolean(false))),
                params: params_of(get(lm, "params"))?,
                legend: !matches!(get(lm, "legend"), Some(Value::Boolean(false))),
            };
            layers_v.push((layer, get(lm, "data")));
        }
    }
    wanted.sort();
    wanted.dedup();
    if let Some(d) = get(m, "data") {
        spec.data = Some(data_of(d, &wanted)?);
    }
    for (mut layer, d) in layers_v {
        if let Some(d) = d {
            layer.data = Some(data_of(d, &wanted)?);
        }
        spec.layers.push(layer);
    }
    if let Some(Value::List(ss)) = get(m, "scales") {
        for sv in ss.iter() {
            let Value::Map(sm) = sv else { continue };
            spec.scales.push(ScaleSpec {
                aes: get(sm, "aes").map(|v| text(v, "scale aes")).transpose()?.unwrap_or_default(),
                kind: get(sm, "kind").map(|v| text(v, "scale kind")).transpose()?.unwrap_or_else(|| "continuous".into()),
                params: params_of(get(sm, "params"))?,
            });
        }
    }
    spec.facet = facet;
    if let Some(Value::Map(c)) = get(m, "coord") {
        let pair = |k: &str| -> Option<(f64, f64)> {
            match get(c, k) {
                Some(Value::List(l)) if l.len() == 2 => {
                    let f = |v: &Value| match v {
                        Value::Integer(i) => Some(*i as f64),
                        Value::Float(f) => Some(*f),
                        _ => None,
                    };
                    Some((f(&l[0])?, f(&l[1])?))
                }
                _ => None,
            }
        };
        spec.coord = CoordSpec {
            kind: get(c, "kind").map(|v| text(v, "coord")).transpose()?.unwrap_or_else(|| "cartesian".into()),
            ratio: match get(c, "ratio") {
                Some(Value::Integer(i)) => Some(*i as f64),
                Some(Value::Float(f)) => Some(*f),
                _ => None,
            },
            xlim: pair("xlim"),
            ylim: pair("ylim"),
        };
    }
    if let Some(Value::Map(l)) = get(m, "labels") {
        for (k, v) in l.iter() {
            if let Value::String(t) = v {
                spec.labels.insert(if k == "colour" { "color".into() } else { k.clone() }, t.as_ref().clone());
            }
        }
    }
    spec.theme = params_of(get(m, "theme"))?;
    Ok(spec)
}

/// The appearance a host asks for (`OLANG_PLOT_THEME`, as plot reads it).
fn host_appearance() -> Option<Appearance> {
    std::env::var("OLANG_PLOT_THEME").ok().and_then(|t| Appearance::parse(&t))
}

/// A spec value's scene (zoomed to a horizontal range when given).
pub fn scene_of(v: &Value, zoom: Option<(f64, f64, Option<usize>)>) -> Result<Scene, String> {
    let spec = spec_of(v)?;
    olang_ods::gg::render(
        &spec,
        &RenderOptions {
            zoom,
            appearance: host_appearance(),
        },
    )
    .map_err(|e| e.to_string())
}

pub const FUNCTIONS: &[(&str, usize)] = &[("gg", 1), ("gg_scene", 1)];

pub fn dispatch(func: &str, args: Vec<Value>) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!("plot.{} expects 1 argument (a gg plot), got {}", func, args.len()));
    }
    let scene = scene_of(&args[0], None)?;
    Ok(match func {
        "gg" => s(&scene.to_svg()),
        _ => s(&scene.to_json(&olang_ods::gg::SceneJsonOptions::default())),
    })
}
