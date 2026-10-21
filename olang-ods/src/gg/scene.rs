//! The scene: a plot resolved to a display list in plot pixels, and its
//! two serializations — SVG (an export, a page) and JSON (what a host
//! such as Studio draws natively, decimated for the screen, with each
//! mark's values for hover and each panel's scales for zoom and brush).

use super::theme::{Appearance, quiet};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Rgba {
    pub fn new(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 1.0 }
    }

    /// `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub fn hex(s: &str) -> Option<Rgba> {
        let h = s.strip_prefix('#')?;
        let p = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
        match h.len() {
            3 => Some(Rgba::new(p(0, 1)? * 17, p(1, 1)? * 17, p(2, 1)? * 17)),
            6 => Some(Rgba::new(p(0, 2)?, p(2, 2)?, p(4, 2)?)),
            8 => Some(Rgba {
                a: p(6, 2)? as f32 / 255.0,
                ..Rgba::new(p(0, 2)?, p(2, 2)?, p(4, 2)?)
            }),
            _ => None,
        }
    }

    pub fn with_alpha(self, a: f64) -> Rgba {
        Rgba {
            a: (self.a as f64 * a).clamp(0.0, 1.0) as f32,
            ..self
        }
    }

    pub fn rgb_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// `#rrggbb`, or `#rrggbbaa` when translucent.
    pub fn hex8(self) -> String {
        if self.a >= 0.999 {
            self.rgb_hex()
        } else {
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                self.r,
                self.g,
                self.b,
                (self.a * 255.0).round() as u8
            )
        }
    }

    /// Mixed toward `o` by `t` in sRGB.
    pub fn mix(self, o: Rgba, t: f64) -> Rgba {
        let l = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round() as u8;
        Rgba {
            r: l(self.r, o.r),
            g: l(self.g, o.g),
            b: l(self.b, o.b),
            a: self.a + (o.a - self.a) * t as f32,
        }
    }

    /// Relative luminance (for a label's ink on a filled tile).
    pub fn luminance(self) -> f64 {
        let c = |v: u8| {
            let s = v as f64 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * c(self.r) + 0.7152 * c(self.g) + 0.0722 * c(self.b)
    }
}

/// A frame colour's role: the appearance (or the host's tokens) says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Surface,
    Ink,
    Ink2,
    Grid,
    Axis,
    Strip,
    Panel,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Role::Surface => "@surface",
            Role::Ink => "@ink",
            Role::Ink2 => "@ink2",
            Role::Grid => "@grid",
            Role::Axis => "@axis",
            Role::Strip => "@strip",
            Role::Panel => "@panel",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Paint {
    Role(Role),
    Rgba(Rgba),
}

impl Paint {
    fn resolve(self, app: Appearance) -> Rgba {
        match self {
            Paint::Role(r) => app.role(r),
            Paint::Rgba(c) => c,
        }
    }
    fn json(self) -> String {
        match self {
            Paint::Role(r) => r.name().to_string(),
            Paint::Rgba(c) => c.hex8(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    fn svg(self) -> &'static str {
        match self {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
            Anchor::End => "end",
        }
    }
    fn json(self) -> &'static str {
        match self {
            Anchor::Start => "start",
            Anchor::Middle => "center",
            Anchor::End => "end",
        }
    }
}

/// A point's glyph (ggplot's shapes 0–25, by kind).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Circle,
    Square,
    Triangle,
    Diamond,
    TriangleDown,
    OpenCircle,
    OpenSquare,
    OpenTriangle,
    OpenDiamond,
    Plus,
    Cross,
}

impl Shape {
    pub const ORDER: [Shape; 11] = [
        Shape::Circle,
        Shape::Triangle,
        Shape::Square,
        Shape::Diamond,
        Shape::TriangleDown,
        Shape::OpenCircle,
        Shape::OpenTriangle,
        Shape::OpenSquare,
        Shape::Plus,
        Shape::Cross,
        Shape::OpenDiamond,
    ];

    pub fn parse(s: &str) -> Option<Shape> {
        Some(match s {
            "circle" | "16" | "19" | "20" | "21" => Shape::Circle,
            "square" | "15" | "22" => Shape::Square,
            "triangle" | "17" | "24" => Shape::Triangle,
            "diamond" | "18" | "23" => Shape::Diamond,
            "triangle down" | "triangle_down" | "25" => Shape::TriangleDown,
            "circle open" | "open circle" | "open_circle" | "1" => Shape::OpenCircle,
            "square open" | "open square" | "open_square" | "0" => Shape::OpenSquare,
            "triangle open" | "open triangle" | "open_triangle" | "2" => Shape::OpenTriangle,
            "diamond open" | "open diamond" | "open_diamond" | "5" => Shape::OpenDiamond,
            "plus" | "3" => Shape::Plus,
            "cross" | "4" => Shape::Cross,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Shape::Circle => "circle",
            Shape::Square => "square",
            Shape::Triangle => "triangle",
            Shape::Diamond => "diamond",
            Shape::TriangleDown => "triangle_down",
            Shape::OpenCircle => "open_circle",
            Shape::OpenSquare => "open_square",
            Shape::OpenTriangle => "open_triangle",
            Shape::OpenDiamond => "open_diamond",
            Shape::Plus => "plus",
            Shape::Cross => "cross",
        }
    }

    pub fn open(self) -> bool {
        matches!(
            self,
            Shape::OpenCircle
                | Shape::OpenSquare
                | Shape::OpenTriangle
                | Shape::OpenDiamond
                | Shape::Plus
                | Shape::Cross
        )
    }
}

/// The values a tooltip says, by column: `cols[f][k]` is field `f` of
/// mark `k`.
#[derive(Clone, Debug, Default)]
pub struct Tips {
    pub layer: u32,
    pub cols: Vec<Vec<String>>,
}

impl Tips {
    fn take(&self, idx: &[usize]) -> Tips {
        Tips {
            layer: self.layer,
            cols: self
                .cols
                .iter()
                .map(|c| idx.iter().map(|&i| c[i].clone()).collect())
                .collect(),
        }
    }
}

/// Many points of one glyph: the batch a host draws in one go.
#[derive(Clone, Debug)]
pub struct Points {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
    /// One radius, or one per point.
    pub r: Vec<f32>,
    /// One colour index, or one per point, into `pal`.
    pub fill: Vec<u32>,
    pub pal: Vec<Rgba>,
    pub shape: Shape,
    /// An outline's width (open glyphs, plus, cross).
    pub stroke_w: f32,
    pub panel: u32,
    pub tips: Option<Tips>,
}

impl Points {
    fn r_at(&self, k: usize) -> f32 {
        if self.r.len() == 1 { self.r[0] } else { self.r[k] }
    }
    fn fill_at(&self, k: usize) -> u32 {
        if self.fill.len() == 1 {
            self.fill[0]
        } else {
            self.fill[k]
        }
    }
}

#[derive(Clone, Debug)]
pub enum Item {
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        fill: Option<Paint>,
        stroke: Option<Paint>,
        sw: f64,
        r: f64,
        /// layer and values when the rect is a mark
        tip: Option<(u32, Vec<String>)>,
    },
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        stroke: Paint,
        sw: f64,
        dash: Option<Vec<f64>>,
    },
    Path {
        pts: Vec<(f64, f64)>,
        stroke: Option<Paint>,
        sw: f64,
        dash: Option<Vec<f64>>,
        fill: Option<Paint>,
        close: bool,
        /// one row per vertex when the path is a mark a pointer reads
        tips: Option<Tips>,
    },
    Points(Points),
    Text {
        x: f64,
        /// the line's vertical centre
        y: f64,
        text: String,
        size: f64,
        paint: Paint,
        anchor: Anchor,
        weight: u16,
    },
    Clip {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
    Unclip,
}

/// One axis of a panel, as a host maps the pointer through it.
#[derive(Clone, Debug)]
pub struct AxisInfo {
    /// "continuous", "discrete" or "time".
    pub kind: &'static str,
    /// "identity", "log10", "sqrt", "reverse".
    pub trans: &'static str,
    /// The range shown, in the transformed space (the pixel box's ends).
    pub lo: f64,
    pub hi: f64,
    pub levels: Vec<String>,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct PanelInfo {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// The horizontal axis (the y scale under coord_flip).
    pub h_axis: AxisInfo,
    pub v_axis: AxisInfo,
    pub flip: bool,
    pub row: usize,
    pub col: usize,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct LayerInfo {
    pub geom: String,
    pub fields: Arc<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub width: f64,
    pub height: f64,
    pub app: Appearance,
    pub font: String,
    pub items: Vec<Item>,
    pub panels: Vec<PanelInfo>,
    pub layers: Vec<LayerInfo>,
    pub title: String,
    /// What the plot is, said to a screen reader.
    pub alt: String,
    /// Whether the horizontal axis zooms (continuous or time).
    pub zoomable: bool,
    pub zoom: Option<(f64, f64)>,
    /// The theme's paper, ink and id prefix (`Theme`): the SVG's alone, a
    /// host draws the scene in its own tokens.
    pub paper: Option<String>,
    pub ink: Option<String>,
    pub id: Option<String>,
}

fn n1(v: f64) -> String {
    // one decimal, trimmed
    let r = (v * 10.0).round() / 10.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        format!("{:.1}", r)
    }
}

fn n2(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        let s = format!("{:.2}", r);
        s.trim_end_matches('0').to_string()
    }
}

fn esc(s: &str) -> String {
    s.replace(ID_MARK, "")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn jstr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn jnum(v: f64) -> String {
    if v.is_finite() {
        format!("{:?}", v)
    } else {
        "null".to_string()
    }
}

/// A glyph's outline as an SVG path (centre, radius).
fn glyph_d(shape: Shape, x: f64, y: f64, r: f64) -> String {
    match shape {
        Shape::Square | Shape::OpenSquare => {
            let s = r * 0.9;
            format!(
                "M{} {}h{}v{}h{}z",
                n2(x - s),
                n2(y - s),
                n2(2.0 * s),
                n2(2.0 * s),
                n2(-2.0 * s)
            )
        }
        Shape::Triangle | Shape::OpenTriangle => {
            let s = r * 1.2;
            format!(
                "M{} {}L{} {}L{} {}z",
                n2(x),
                n2(y - s),
                n2(x + s * 0.866),
                n2(y + s * 0.5),
                n2(x - s * 0.866),
                n2(y + s * 0.5)
            )
        }
        Shape::TriangleDown => {
            let s = r * 1.2;
            format!(
                "M{} {}L{} {}L{} {}z",
                n2(x),
                n2(y + s),
                n2(x + s * 0.866),
                n2(y - s * 0.5),
                n2(x - s * 0.866),
                n2(y - s * 0.5)
            )
        }
        Shape::Diamond | Shape::OpenDiamond => {
            let s = r * 1.25;
            format!(
                "M{} {}L{} {}L{} {}L{} {}z",
                n2(x),
                n2(y - s),
                n2(x + s),
                n2(y),
                n2(x),
                n2(y + s),
                n2(x - s),
                n2(y)
            )
        }
        Shape::Plus => format!(
            "M{} {}H{}M{} {}V{}",
            n2(x - r),
            n2(y),
            n2(x + r),
            n2(x),
            n2(y - r),
            n2(y + r)
        ),
        Shape::Cross => {
            let s = r * 0.8;
            format!(
                "M{} {}L{} {}M{} {}L{} {}",
                n2(x - s),
                n2(y - s),
                n2(x + s),
                n2(y + s),
                n2(x - s),
                n2(y + s),
                n2(x + s),
                n2(y - s)
            )
        }
        Shape::Circle | Shape::OpenCircle => String::new(),
    }
}

fn dash_attr(d: &Option<Vec<f64>>) -> String {
    match d {
        Some(d) if !d.is_empty() => format!(
            " stroke-dasharray=\"{}\"",
            d.iter().map(|v| n1(*v)).collect::<Vec<_>>().join(" ")
        ),
        _ => String::new(),
    }
}

fn paint_attr(name: &str, p: Paint, app: Appearance) -> String {
    let c = p.resolve(app);
    if c.a >= 0.999 {
        format!(" {}=\"{}\"", name, c.rgb_hex())
    } else {
        format!(
            " {}=\"{}\" {}-opacity=\"{}\"",
            name,
            c.rgb_hex(),
            name,
            n2(c.a as f64)
        )
    }
}

/// The marker standing for a plot's id prefix while its SVG is written:
/// a character no SVG text holds (`esc` drops it), replaced once the
/// document is whole.
const ID_MARK: char = '\u{1}';

/// A 64-bit FNV-1a hash: a plot's ids derive from its SVG's content, so a
/// build is reproducible and two different plots on one page never share
/// an id.
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A caller's id prefix kept to what an id may hold (letters, digits,
/// `-`, `_`), starting with a letter.
fn id_prefix_of(given: &str) -> Option<String> {
    let kept: String = given
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if kept.is_empty() {
        return None;
    }
    Some(if kept.starts_with(|c: char| c.is_ascii_alphabetic()) {
        kept
    } else {
        format!("g{}", kept)
    })
}

/// How the JSON a host draws is cut down for the screen.
#[derive(Clone, Debug)]
pub struct SceneJsonOptions {
    /// Points the scene carries at most (more are thinned: one a pixel
    /// and colour, then at an even stride).
    pub max_points: usize,
    /// Vertices a path carries at most (more: each pixel column's least
    /// and most).
    pub max_vertices: usize,
}

impl Default for SceneJsonOptions {
    fn default() -> Self {
        SceneJsonOptions {
            max_points: 12_000,
            max_vertices: 3_000,
        }
    }
}

impl Scene {
    /// The description an SVG carries (`plot.describe` reads it): what the
    /// plot is, its size and panels, without its marks.
    pub fn description(&self) -> String {
        let mut o = format!(
            "{{\"v\":1,\"type\":\"gg\",\"title\":{},\"width\":{},\"height\":{},\"theme\":{},\"alt\":{},\"panels\":",
            jstr(&self.title),
            jnum(self.width),
            jnum(self.height),
            jstr(self.app.name()),
            jstr(&self.alt)
        );
        self.panels_json(&mut o);
        o.push('}');
        o
    }

    fn panels_json(&self, o: &mut String) {
        o.push('[');
        for (i, p) in self.panels.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            let axis = |a: &AxisInfo| {
                format!(
                    "{{\"kind\":{},\"trans\":{},\"lo\":{},\"hi\":{},\"levels\":[{}],\"title\":{}}}",
                    jstr(a.kind),
                    jstr(a.trans),
                    jnum(a.lo),
                    jnum(a.hi),
                    a.levels.iter().map(|l| jstr(l)).collect::<Vec<_>>().join(","),
                    jstr(&a.title)
                )
            };
            let _ = write!(
                o,
                "{{\"x\":{},\"y\":{},\"w\":{},\"h\":{},\"row\":{},\"col\":{},\"label\":{},\"flip\":{},\"h_axis\":{},\"v_axis\":{}}}",
                n2(p.x),
                n2(p.y),
                n2(p.w),
                n2(p.h),
                p.row,
                p.col,
                jstr(&p.label),
                p.flip,
                axis(&p.h_axis),
                axis(&p.v_axis)
            );
        }
        o.push(']');
    }

    /// The scene as an SVG document.
    pub fn to_svg(&self) -> String {
        let npts: usize = self
            .items
            .iter()
            .map(|i| match i {
                Item::Points(p) => p.xs.len(),
                _ => 0,
            })
            .sum();
        let mut s = String::with_capacity(8192 + npts * 40);
        let _ = write!(
            s,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" font-family=\"{f}\" role=\"img\" aria-label=\"{a}\">",
            w = n1(self.width),
            h = n1(self.height),
            f = esc(&self.font),
            a = esc(&self.alt)
        );
        let desc = self.description();
        let _ = write!(
            s,
            "<metadata class=\"olang-chart\">{}</metadata>",
            desc.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        );
        let _ = write!(
            s,
            "<rect width=\"{}\" height=\"{}\"{}/>",
            n1(self.width),
            n1(self.height),
            self.paint_attr("fill", Paint::Role(Role::Surface))
        );
        let mut clip_id = 0;
        let mut open_groups = 0;
        for item in &self.items {
            match item {
                Item::Clip { x, y, w, h } => {
                    clip_id += 1;
                    let _ = write!(
                        s,
                        "<clipPath id=\"{ID_MARK}clip{clip_id}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath><g clip-path=\"url(#{ID_MARK}clip{clip_id})\">",
                        n2(*x),
                        n2(*y),
                        n2(*w),
                        n2(*h)
                    );
                    open_groups += 1;
                }
                Item::Unclip => {
                    if open_groups > 0 {
                        s.push_str("</g>");
                        open_groups -= 1;
                    }
                }
                Item::Rect {
                    x,
                    y,
                    w,
                    h,
                    fill,
                    stroke,
                    sw,
                    r,
                    ..
                } => {
                    if *w <= 0.0 || *h <= 0.0 {
                        continue;
                    }
                    let _ = write!(
                        s,
                        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                        n2(*x),
                        n2(*y),
                        n2(w.max(0.0)),
                        n2(h.max(0.0))
                    );
                    if *r > 0.0 {
                        let _ = write!(s, " rx=\"{}\"", n1(*r));
                    }
                    match fill {
                        Some(f) => s.push_str(&self.paint_attr("fill", *f)),
                        None => s.push_str(" fill=\"none\""),
                    }
                    if let Some(st) = stroke {
                        s.push_str(&self.paint_attr("stroke", *st));
                        let _ = write!(s, " stroke-width=\"{}\"", n2(*sw));
                    }
                    s.push_str("/>");
                }
                Item::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    stroke,
                    sw,
                    dash,
                } => {
                    let _ = write!(
                        s,
                        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"{} stroke-width=\"{}\"{}/>",
                        n2(*x1),
                        n2(*y1),
                        n2(*x2),
                        n2(*y2),
                        self.paint_attr("stroke", *stroke),
                        n2(*sw),
                        dash_attr(dash)
                    );
                }
                Item::Path {
                    pts,
                    stroke,
                    sw,
                    dash,
                    fill,
                    close,
                    ..
                } => {
                    if pts.len() < 2 {
                        continue;
                    }
                    let mut d = String::with_capacity(pts.len() * 14);
                    for (k, (x, y)) in pts.iter().enumerate() {
                        let _ = write!(d, "{}{} {}", if k == 0 { "M" } else { "L" }, n1(*x), n1(*y));
                    }
                    if *close {
                        d.push('Z');
                    }
                    let _ = write!(s, "<path d=\"{}\"", d);
                    match fill {
                        Some(f) => s.push_str(&self.paint_attr("fill", *f)),
                        None => s.push_str(" fill=\"none\""),
                    }
                    if let Some(st) = stroke {
                        s.push_str(&self.paint_attr("stroke", *st));
                        let _ = write!(
                            s,
                            " stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{}",
                            n2(*sw),
                            dash_attr(dash)
                        );
                    }
                    s.push_str("/>");
                }
                Item::Points(p) => self.points_svg(&mut s, p),
                Item::Text {
                    x,
                    y,
                    text,
                    size,
                    paint,
                    anchor,
                    weight,
                } => {
                    let _ = write!(
                        s,
                        "<text x=\"{}\" y=\"{}\" font-size=\"{}\"{}",
                        n2(*x),
                        n2(*y + size * 0.35),
                        n1(*size),
                        self.paint_attr("fill", *paint)
                    );
                    if *anchor != Anchor::Start {
                        let _ = write!(s, " text-anchor=\"{}\"", anchor.svg());
                    }
                    if *weight != 400 {
                        let _ = write!(s, " font-weight=\"{}\"", weight);
                    }
                    let _ = write!(s, ">{}</text>", esc(text));
                }
            }
        }
        for _ in 0..open_groups {
            s.push_str("</g>");
        }
        s.push_str("</svg>");
        // every id the document names takes one prefix: the theme's `id`,
        // else a hash of the document, so two plots on one page (or a
        // plot's light and dark twins) never share one and a build of the
        // same plot writes the same ids
        if s.contains(ID_MARK) {
            let prefix = match self.id.as_deref().and_then(id_prefix_of) {
                Some(p) => format!("{}-", p),
                None => format!("gg{:08x}-", fnv1a(&s) as u32),
            };
            s = s.replace(ID_MARK, &prefix);
        }
        s
    }

    /// A paint as an SVG attribute: a role in the theme's paper or ink
    /// when it names one (the lines at a reduced opacity), else the
    /// appearance's colour.
    fn paint_attr(&self, name: &str, p: Paint) -> String {
        if let Paint::Role(r) = p {
            if r == Role::Surface
                && let Some(paper) = &self.paper
            {
                return format!(" {}=\"{}\"", name, esc(paper));
            }
            if r != Role::Surface
                && let Some(ink) = &self.ink
            {
                let opacity = match r {
                    Role::Grid => Some(0.12),
                    Role::Axis => Some(0.3),
                    Role::Strip | Role::Panel => Some(0.05),
                    _ => None,
                };
                return match opacity {
                    Some(o) => format!(" {}=\"{}\" {}-opacity=\"{}\"", name, esc(ink), name, o),
                    None => format!(" {}=\"{}\"", name, esc(ink)),
                };
            }
        }
        paint_attr(name, p, self.app)
    }

    fn points_svg(&self, s: &mut String, p: &Points) {
        let n = p.xs.len();
        // points in runs of one colour share a group; colours interleaved
        // point by point (a scatter coloured by a column, in data order)
        // are grouped by colour instead, which keeps the file small
        let runs = (1..n).filter(|&k| p.fill_at(k) != p.fill_at(k - 1)).count() + 1;
        let order: Vec<usize> = if runs * 4 > n && p.pal.len() > 1 {
            let mut o: Vec<usize> = (0..n).collect();
            o.sort_by_key(|&k| p.fill_at(k));
            o
        } else {
            (0..n).collect()
        };
        let mut k = 0;
        while k < n {
            let c = p.fill_at(order[k]);
            let mut e = k + 1;
            while e < n && p.fill_at(order[e]) == c {
                e += 1;
            }
            let col = p.pal[c as usize];
            if col.a <= 0.0 {
                k = e;
                continue;
            }
            if p.shape.open() {
                let _ = write!(
                    s,
                    "<g fill=\"none\"{} stroke-width=\"{}\">",
                    paint_attr("stroke", Paint::Rgba(col), self.app),
                    n2(p.stroke_w as f64)
                );
            } else {
                let _ = write!(s, "<g{}>", paint_attr("fill", Paint::Rgba(col), self.app));
            }
            for &j in &order[k..e] {
                let (x, y, r) = (p.xs[j] as f64, p.ys[j] as f64, p.r_at(j) as f64);
                if !(x.is_finite() && y.is_finite()) || r <= 0.0 {
                    continue;
                }
                match p.shape {
                    Shape::Circle | Shape::OpenCircle => {
                        let _ = write!(s, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\"/>", n1(x), n1(y), n2(r));
                    }
                    other => {
                        let _ = write!(s, "<path d=\"{}\"/>", glyph_d(other, x, y, r));
                    }
                }
            }
            s.push_str("</g>");
            k = e;
        }
    }

    /// The scene as JSON for a host: ops in plot pixels (frame colours as
    /// roles `@ink`…, data colours `#rrggbb[aa]`), points batched and
    /// thinned for the screen, each mark's tooltip values, the panels'
    /// scales.
    pub fn to_json(&self, o: &SceneJsonOptions) -> String {
        let total: usize = self
            .items
            .iter()
            .map(|i| match i {
                Item::Points(p) => p.xs.len(),
                _ => 0,
            })
            .sum();
        let mut cut = false;
        let mut out = String::with_capacity(16384 + total.min(o.max_points) * 24);
        let _ = write!(
            out,
            "{{\"v\":2,\"type\":\"scene\",\"title\":{},\"width\":{},\"height\":{},\"theme\":{},\"alt\":{},\"zoomable\":{},\"n\":{}",
            jstr(&self.title),
            jnum(self.width),
            jnum(self.height),
            jstr(self.app.name()),
            jstr(&self.alt),
            self.zoomable,
            total
        );
        if let Some((a, b)) = self.zoom {
            let _ = write!(out, ",\"zoom\":{{\"x0\":{},\"x1\":{}}}", jnum(a), jnum(b));
        }
        // the categorical palette in every appearance: a host drawing in
        // another appearance swaps one for the other
        let _ = write!(out, ",\"palettes\":{{");
        for (i, app) in [Appearance::Light, Appearance::Dark, Appearance::Contrast]
            .iter()
            .enumerate()
        {
            if i > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{}:[{}]",
                jstr(app.name()),
                quiet(*app)
                    .iter()
                    .map(|c| jstr(c))
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        out.push('}');
        out.push_str(",\"layers\":[");
        for (i, l) in self.layers.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"geom\":{},\"fields\":[{}]}}",
                jstr(&l.geom),
                l.fields.iter().map(|f| jstr(f)).collect::<Vec<_>>().join(",")
            );
        }
        out.push_str("],\"panels\":");
        self.panels_json(&mut out);
        out.push_str(",\"ops\":[");
        // each points batch's share of the budget
        let budget = |n: usize| -> usize {
            if total <= o.max_points {
                n
            } else {
                ((n as f64 / total as f64) * o.max_points as f64).ceil() as usize
            }
        };
        let mut first = true;
        let mut sep = |out: &mut String| {
            if !first {
                out.push(',');
            }
            first = false;
        };
        for item in &self.items {
            match item {
                Item::Clip { x, y, w, h } => {
                    sep(&mut out);
                    let _ = write!(
                        out,
                        "{{\"op\":\"clip\",\"x\":{},\"y\":{},\"w\":{},\"h\":{}}}",
                        n2(*x),
                        n2(*y),
                        n2(*w),
                        n2(*h)
                    );
                }
                Item::Unclip => {
                    sep(&mut out);
                    out.push_str("{\"op\":\"unclip\"}");
                }
                Item::Rect {
                    x,
                    y,
                    w,
                    h,
                    fill,
                    stroke,
                    sw,
                    r,
                    tip,
                } => {
                    sep(&mut out);
                    let _ = write!(
                        out,
                        "{{\"op\":\"rect\",\"x\":{},\"y\":{},\"w\":{},\"h\":{}",
                        n2(*x),
                        n2(*y),
                        n2(*w),
                        n2(*h)
                    );
                    if let Some(f) = fill {
                        let _ = write!(out, ",\"fill\":{}", jstr(&f.json()));
                    }
                    if let Some(st) = stroke {
                        let _ = write!(out, ",\"stroke\":{},\"sw\":{}", jstr(&st.json()), n2(*sw));
                    }
                    if *r > 0.0 {
                        let _ = write!(out, ",\"r\":{}", n1(*r));
                    }
                    if let Some((l, vals)) = tip {
                        let _ = write!(
                            out,
                            ",\"L\":{},\"tip\":[{}]",
                            l,
                            vals.iter().map(|v| jstr(v)).collect::<Vec<_>>().join(",")
                        );
                    }
                    out.push('}');
                }
                Item::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    stroke,
                    sw,
                    dash,
                } => {
                    sep(&mut out);
                    let _ = write!(
                        out,
                        "{{\"op\":\"line\",\"x1\":{},\"y1\":{},\"x2\":{},\"y2\":{},\"stroke\":{},\"sw\":{}",
                        n2(*x1),
                        n2(*y1),
                        n2(*x2),
                        n2(*y2),
                        jstr(&stroke.json()),
                        n2(*sw)
                    );
                    if let Some(d) = dash {
                        let _ = write!(
                            out,
                            ",\"dash\":[{}]",
                            d.iter().map(|v| n1(*v)).collect::<Vec<_>>().join(",")
                        );
                    }
                    out.push('}');
                }
                Item::Path {
                    pts,
                    stroke,
                    sw,
                    dash,
                    fill,
                    close,
                    tips,
                } => {
                    if pts.len() < 2 {
                        continue;
                    }
                    sep(&mut out);
                    let mut keep = thin_path(pts, o.max_vertices);
                    if keep.len() < pts.len() {
                        cut = true;
                    }
                    // a path no pointer reads drops the vertices that land
                    // where the last did (at a tenth of a pixel): drawn the same
                    if tips.is_none() {
                        let mut last: Option<(i64, i64)> = None;
                        keep.retain(|&k| {
                            let at = tenths(pts[k]);
                            let new = last != Some(at);
                            last = Some(at);
                            new
                        });
                    }
                    // the vertices in tenths of a pixel, each after the first
                    // as its step from the one before (`d`)
                    out.push_str("{\"op\":\"path\",\"d\":[");
                    let mut prev = (0i64, 0i64);
                    for (j, &k) in keep.iter().enumerate() {
                        if j > 0 {
                            out.push(',');
                        }
                        let at = tenths(pts[k]);
                        let _ = write!(out, "{},{}", at.0 - prev.0, at.1 - prev.1);
                        prev = at;
                    }
                    out.push(']');
                    if let Some(f) = fill {
                        let _ = write!(out, ",\"fill\":{}", jstr(&f.json()));
                    }
                    if let Some(st) = stroke {
                        let _ = write!(out, ",\"stroke\":{},\"sw\":{}", jstr(&st.json()), n2(*sw));
                    }
                    if let Some(d) = dash {
                        let _ = write!(
                            out,
                            ",\"dash\":[{}]",
                            d.iter().map(|v| n1(*v)).collect::<Vec<_>>().join(",")
                        );
                    }
                    if *close {
                        out.push_str(",\"close\":true");
                    }
                    if let Some(t) = tips {
                        tips_json(&mut out, &t.take(&keep));
                    }
                    out.push('}');
                }
                Item::Points(p) => {
                    sep(&mut out);
                    let keep = thin_points(p, budget(p.xs.len()));
                    if keep.len() < p.xs.len() {
                        cut = true;
                    }
                    out.push_str("{\"op\":\"points\",\"x\":[");
                    for (j, &k) in keep.iter().enumerate() {
                        if j > 0 {
                            out.push(',');
                        }
                        out.push_str(&n1(p.xs[k] as f64));
                    }
                    out.push_str("],\"y\":[");
                    for (j, &k) in keep.iter().enumerate() {
                        if j > 0 {
                            out.push(',');
                        }
                        out.push_str(&n1(p.ys[k] as f64));
                    }
                    out.push(']');
                    if p.r.len() == 1 {
                        let _ = write!(out, ",\"r\":{}", n2(p.r[0] as f64));
                    } else {
                        let _ = write!(
                            out,
                            ",\"r\":[{}]",
                            keep.iter()
                                .map(|&k| n2(p.r[k] as f64))
                                .collect::<Vec<_>>()
                                .join(",")
                        );
                    }
                    let _ = write!(
                        out,
                        ",\"pal\":[{}]",
                        p.pal.iter().map(|c| jstr(&c.hex8())).collect::<Vec<_>>().join(",")
                    );
                    if p.fill.len() == 1 {
                        let _ = write!(out, ",\"fill\":{}", p.fill[0]);
                    } else {
                        let _ = write!(
                            out,
                            ",\"fill\":[{}]",
                            keep.iter()
                                .map(|&k| p.fill[k].to_string())
                                .collect::<Vec<_>>()
                                .join(",")
                        );
                    }
                    let _ = write!(
                        out,
                        ",\"shape\":{},\"sw\":{},\"panel\":{}",
                        jstr(p.shape.name()),
                        n2(p.stroke_w as f64),
                        p.panel
                    );
                    if keep.len() < p.xs.len() {
                        let _ = write!(out, ",\"n\":{}", p.xs.len());
                    }
                    if let Some(t) = &p.tips {
                        tips_json(&mut out, &t.take(&keep));
                    }
                    out.push('}');
                }
                Item::Text {
                    x,
                    y,
                    text,
                    size,
                    paint,
                    anchor,
                    weight,
                } => {
                    sep(&mut out);
                    let _ = write!(
                        out,
                        "{{\"op\":\"text\",\"x\":{},\"y\":{},\"text\":{},\"size\":{},\"color\":{},\"align\":{}",
                        n2(*x),
                        n2(*y),
                        jstr(text),
                        n1(*size),
                        jstr(&paint.json()),
                        jstr(anchor.json())
                    );
                    if *weight != 400 {
                        let _ = write!(out, ",\"weight\":{}", weight);
                    }
                    out.push('}');
                }
            }
        }
        out.push(']');
        let _ = write!(out, ",\"cut\":{}}}", cut);
        out
    }
}

/// A point in tenths of a pixel.
fn tenths(p: (f64, f64)) -> (i64, i64) {
    let t = |v: f64| if v.is_finite() { (v * 10.0).round() as i64 } else { 0 };
    (t(p.0), t(p.1))
}

/// A mark's tooltip values by field: a field the same for every mark is
/// its one String; one with few values is `{"lv": [the values], "ix":
/// [each mark's]}`; else a list of each mark's.
fn tips_json(out: &mut String, t: &Tips) {
    let _ = write!(out, ",\"L\":{},\"tips\":[", t.layer);
    for (i, c) in t.cols.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        if !c.is_empty() && c.iter().all(|v| *v == c[0]) {
            out.push_str(&jstr(&c[0]));
            continue;
        }
        let mut levels: Vec<&String> = Vec::new();
        let mut index: std::collections::HashMap<&String, usize> = std::collections::HashMap::new();
        let mut ix: Vec<usize> = Vec::with_capacity(c.len());
        for v in c {
            if index.len() * 3 > c.len() {
                break;
            }
            let k = *index.entry(v).or_insert_with(|| {
                levels.push(v);
                levels.len() - 1
            });
            ix.push(k);
        }
        if ix.len() == c.len() && levels.len() * 3 <= c.len() {
            out.push_str("{\"lv\":[");
            out.push_str(&levels.iter().map(|v| jstr(v)).collect::<Vec<_>>().join(","));
            out.push_str("],\"ix\":[");
            out.push_str(&ix.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(","));
            out.push_str("]}");
            continue;
        }
        out.push('[');
        for (j, v) in c.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&jstr(v));
        }
        out.push(']');
    }
    out.push(']');
}

/// Points kept for the screen: the first at each pixel and colour, then
/// an even stride down to `budget`.
fn thin_points(p: &Points, budget: usize) -> Vec<usize> {
    let n = p.xs.len();
    if n <= budget {
        return (0..n).collect();
    }
    let mut seen: HashSet<(i32, i32, u32)> = HashSet::with_capacity(budget * 2);
    let mut keep = Vec::with_capacity(budget.min(n));
    for k in 0..n {
        let key = (
            (p.xs[k] * 1.5).round() as i32,
            (p.ys[k] * 1.5).round() as i32,
            p.fill_at(k),
        );
        if seen.insert(key) {
            keep.push(k);
        }
    }
    if keep.len() > budget {
        let step = keep.len() as f64 / budget as f64;
        keep = (0..budget)
            .map(|i| keep[((i as f64) * step) as usize])
            .collect();
    }
    keep
}

/// A path's vertices kept for the screen: all when few, else each pixel
/// column's first, least, most and last in order.
fn thin_path(pts: &[(f64, f64)], max: usize) -> Vec<usize> {
    let n = pts.len();
    if n <= max {
        return (0..n).collect();
    }
    let mut keep = Vec::with_capacity(max);
    let mut k = 0;
    while k < n {
        let col = pts[k].0.floor();
        let mut e = k;
        let (mut lo, mut hi) = (k, k);
        while e < n && pts[e].0.floor() == col {
            if pts[e].1 < pts[lo].1 {
                lo = e;
            }
            if pts[e].1 > pts[hi].1 {
                hi = e;
            }
            e += 1;
        }
        let mut c = vec![k, lo, hi, e - 1];
        c.sort();
        c.dedup();
        keep.extend(c);
        k = e;
    }
    if keep.len() > max * 2 {
        // not monotonic in x: an even stride
        let step = n as f64 / max as f64;
        keep = (0..max).map(|i| ((i as f64) * step) as usize).collect();
        if *keep.last().unwrap() != n - 1 {
            keep.push(n - 1);
        }
    }
    keep
}
