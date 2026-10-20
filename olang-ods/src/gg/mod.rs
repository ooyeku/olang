//! gg — a grammar of graphics renderer.
//!
//! olang's `gg` module (an embedded olang package) builds a plot as pure
//! data: a frame, aesthetic mappings, layers (a geom, a stat, a
//! position), scales, a facet, a coord, labels and a theme. This module
//! does all the computation that spec implies:
//!
//! 1. each layer's aesthetics are read from its frame (columns, a Series,
//!    or a stat's computed variable), ISO date strings parsed;
//! 2. rows are split into facet panels and groups;
//! 3. position scales transform (log10, sqrt, reverse);
//! 4. stats run per panel and group (bin, density, boxplot, smooth, …);
//! 5. positions adjust (stack, fill, dodge, jitter);
//! 6. scales train (per panel when free) and map colour, size, shape,
//!    alpha and linetype;
//! 7. layout measures text and places panels, strips, axes and legends.
//!
//! The result is a [`Scene`]: a resolved display list in plot pixels
//! (marks, axes, gridlines, legends, strips), with per-mark data for
//! hover and per-panel scales for zoom. SVG is serialized from that same
//! scene, so an exported file and a host drawing the scene natively agree
//! on the layout exactly.

mod build;
mod data;
mod geom;
mod layout;
pub mod math;
mod position;
mod scale;
mod scene;
mod stat;
mod text;
mod theme;

pub use data::{Column, Data};
pub use scene::{Item, Paint, Scene, SceneJsonOptions};
pub use theme::Appearance;

use crate::OdsError;
use std::collections::BTreeMap;

pub type Result<T> = std::result::Result<T, OdsError>;

pub(crate) fn bad<T>(msg: impl Into<String>) -> Result<T> {
    Err(OdsError::InvalidArgument(format!("gg: {}", msg.into())))
}

/// An aesthetic: a visual property a column can be mapped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Aes {
    X,
    Y,
    Xmin,
    Xmax,
    Ymin,
    Ymax,
    Xend,
    Yend,
    Color,
    Fill,
    Size,
    Shape,
    Alpha,
    Linetype,
    Linewidth,
    Group,
    Label,
    Weight,
    Sample,
    Lower,
    Middle,
    Upper,
    Xintercept,
    Yintercept,
    Slope,
    Intercept,
}

impl Aes {
    pub fn parse(name: &str) -> Option<Aes> {
        Some(match name {
            "x" => Aes::X,
            "y" => Aes::Y,
            "xmin" => Aes::Xmin,
            "xmax" => Aes::Xmax,
            "ymin" => Aes::Ymin,
            "ymax" => Aes::Ymax,
            "xend" => Aes::Xend,
            "yend" => Aes::Yend,
            "color" | "colour" => Aes::Color,
            "fill" => Aes::Fill,
            "size" => Aes::Size,
            "shape" => Aes::Shape,
            "alpha" => Aes::Alpha,
            "linetype" => Aes::Linetype,
            "linewidth" => Aes::Linewidth,
            "group" => Aes::Group,
            "label" => Aes::Label,
            "weight" => Aes::Weight,
            "sample" => Aes::Sample,
            "lower" => Aes::Lower,
            "middle" => Aes::Middle,
            "upper" => Aes::Upper,
            "xintercept" => Aes::Xintercept,
            "yintercept" => Aes::Yintercept,
            "slope" => Aes::Slope,
            "intercept" => Aes::Intercept,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Aes::X => "x",
            Aes::Y => "y",
            Aes::Xmin => "xmin",
            Aes::Xmax => "xmax",
            Aes::Ymin => "ymin",
            Aes::Ymax => "ymax",
            Aes::Xend => "xend",
            Aes::Yend => "yend",
            Aes::Color => "color",
            Aes::Fill => "fill",
            Aes::Size => "size",
            Aes::Shape => "shape",
            Aes::Alpha => "alpha",
            Aes::Linetype => "linetype",
            Aes::Linewidth => "linewidth",
            Aes::Group => "group",
            Aes::Label => "label",
            Aes::Weight => "weight",
            Aes::Sample => "sample",
            Aes::Lower => "lower",
            Aes::Middle => "middle",
            Aes::Upper => "upper",
            Aes::Xintercept => "xintercept",
            Aes::Yintercept => "yintercept",
            Aes::Slope => "slope",
            Aes::Intercept => "intercept",
        }
    }

    /// The position scale an aesthetic trains: 'x', 'y' or neither.
    pub fn axis(self) -> Option<char> {
        match self {
            Aes::X | Aes::Xmin | Aes::Xmax | Aes::Xend | Aes::Xintercept => Some('x'),
            Aes::Y
            | Aes::Ymin
            | Aes::Ymax
            | Aes::Yend
            | Aes::Yintercept
            | Aes::Lower
            | Aes::Middle
            | Aes::Upper => Some('y'),
            _ => None,
        }
    }

    /// The non-position aesthetics a scale maps.
    pub fn scaled(self) -> bool {
        matches!(
            self,
            Aes::Color | Aes::Fill | Aes::Size | Aes::Shape | Aes::Alpha | Aes::Linetype | Aes::Linewidth
        )
    }
}

/// Where an aesthetic's values come from.
#[derive(Clone, Debug)]
pub enum Source {
    /// A column of the layer's frame, by name.
    Col(String),
    /// Values given directly (a Series of the frame's length), with the
    /// name its legend and axis say.
    Values(String, Column),
    /// A variable the layer's stat computes (`after_stat(density)`).
    Stat(String),
    /// One value for every row (`gg.aes(group: 1)`, `color:
    /// gg.constant("all")`): a mapping still, so it groups and keys a
    /// legend as ggplot's `aes(colour = "all")` does. Its title is the
    /// aesthetic's name.
    Const(String, Column),
}

impl Source {
    pub fn title(&self) -> &str {
        match self {
            Source::Col(n) | Source::Values(n, _) | Source::Stat(n) | Source::Const(n, _) => n,
        }
    }
}

/// A layer or plot parameter: a constant aesthetic or a geom/stat option.
#[derive(Clone, Debug, PartialEq)]
pub enum Param {
    Num(f64),
    Str(String),
    Bool(bool),
    Nums(Vec<f64>),
    Strs(Vec<String>),
    /// A map of names to strings (manual colours by level, labels by level).
    Pairs(Vec<(String, String)>),
}

pub type Params = BTreeMap<String, Param>;

pub(crate) fn p_num(p: &Params, k: &str) -> Option<f64> {
    match p.get(k) {
        Some(Param::Num(v)) => Some(*v),
        Some(Param::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

pub(crate) fn p_str<'a>(p: &'a Params, k: &str) -> Option<&'a str> {
    match p.get(k) {
        Some(Param::Str(v)) => Some(v.as_str()),
        _ => None,
    }
}

pub(crate) fn p_bool(p: &Params, k: &str) -> Option<bool> {
    match p.get(k) {
        Some(Param::Bool(b)) => Some(*b),
        Some(Param::Num(v)) => Some(*v != 0.0),
        _ => None,
    }
}

pub(crate) fn p_nums(p: &Params, k: &str) -> Option<Vec<f64>> {
    match p.get(k) {
        Some(Param::Nums(v)) => Some(v.clone()),
        Some(Param::Num(v)) => Some(vec![*v]),
        _ => None,
    }
}

pub(crate) fn p_strs(p: &Params, k: &str) -> Option<Vec<String>> {
    match p.get(k) {
        Some(Param::Strs(v)) => Some(v.clone()),
        Some(Param::Str(v)) => Some(vec![v.clone()]),
        _ => None,
    }
}

/// One layer: a geom drawn from a stat's output after a position.
#[derive(Clone, Debug, Default)]
pub struct Layer {
    pub geom: String,
    /// The stat; the geom's default when None.
    pub stat: Option<String>,
    /// The position adjustment; the geom's default when None.
    pub position: Option<String>,
    pub mapping: Vec<(Aes, Source)>,
    /// The layer's own frame; the plot's when None.
    pub data: Option<Data>,
    /// Whether the plot's mapping applies (default true).
    pub inherit: bool,
    /// Constant aesthetics (`color: "red"`, `alpha: 0.4`) and geom, stat
    /// and position options (`bins`, `method`, `se`, `width`, …).
    pub params: Params,
    /// Whether the layer adds keys to legends (default true).
    pub legend: bool,
}

/// A scale's specification: which aesthetic, which kind, its options.
#[derive(Clone, Debug, Default)]
pub struct ScaleSpec {
    /// "x", "y", "color", "fill", "size", "alpha", "shape", "linetype".
    pub aes: String,
    /// "continuous", "discrete", "log10", "sqrt", "reverse", "date",
    /// "datetime", "manual", "viridis_d", "viridis_c", "gradient",
    /// "gradient2", "gradientn", "identity", "palette", "area", "binned".
    pub kind: String,
    pub params: Params,
}

#[derive(Clone, Debug, Default)]
pub struct FacetSpec {
    /// "wrap" or "grid" ("none" for no facet).
    pub kind: String,
    /// facet_wrap's variables, or facet_grid's row variables.
    pub rows: Vec<String>,
    /// facet_grid's column variables.
    pub cols: Vec<String>,
    pub ncol: Option<usize>,
    pub nrow: Option<usize>,
    /// "fixed", "free", "free_x", "free_y".
    pub scales: String,
}

#[derive(Clone, Debug, Default)]
pub struct CoordSpec {
    /// "cartesian", "flip" or "fixed".
    pub kind: String,
    pub ratio: Option<f64>,
    pub xlim: Option<(f64, f64)>,
    pub ylim: Option<(f64, f64)>,
}

/// A whole plot.
#[derive(Clone, Debug, Default)]
pub struct Spec {
    pub data: Option<Data>,
    pub mapping: Vec<(Aes, Source)>,
    pub layers: Vec<Layer>,
    pub scales: Vec<ScaleSpec>,
    pub facet: Option<FacetSpec>,
    pub coord: CoordSpec,
    /// title, subtitle, caption, x, y, and a legend title per aesthetic.
    pub labels: BTreeMap<String, String>,
    /// The theme's name and options: `name` ("minimal", "classic", "bw",
    /// "void", "dark", "contrast"), `appearance`, `base_size`,
    /// `legend_position`, `width`, `height`, `grid`, `font`.
    pub theme: Params,
}

/// How a render is asked for beyond the spec: a host's zoom into one
/// horizontal range and the appearance a theme takes when it names none.
#[derive(Clone, Debug, Default)]
pub struct RenderOptions {
    /// Zoom: the horizontal axis's range (data units), on every panel
    /// when the panel is None or the scale is shared.
    pub zoom: Option<(f64, f64, Option<usize>)>,
    pub appearance: Option<Appearance>,
}

/// Compute a plot's scene.
pub fn render(spec: &Spec, opts: &RenderOptions) -> Result<Scene> {
    build::render(spec, opts)
}

/// A plot as an SVG document (the scene serialized).
pub fn render_svg(spec: &Spec, opts: &RenderOptions) -> Result<String> {
    Ok(render(spec, opts)?.to_svg())
}

#[cfg(test)]
mod tests;
