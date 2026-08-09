//! SVG plot emitter: charts as text, no rendering dependency.
//!
//! Output is a complete standalone SVG document string — it composes
//! with `fs.write_file`, an HTTP response, or the wasm playground
//! equally well, which is the whole design (docs/design/ods.md,
//! Phase 4: "plot output is SVG text").
//!
//! The visual defaults follow a validated categorical palette and the
//! usual charting discipline: series hues assigned in fixed order
//! (never cycled), 2px lines, 8px markers, thin bars with a 2px gap
//! and rounded data-ends, recessive grid and axes, text in ink colors
//! (never the series color), a legend only when there are two or more
//! series, and one y-axis — always.

use crate::OdsError;
use std::fmt::Write as _;

type Result<T> = std::result::Result<T, OdsError>;

// Validated categorical palette (light surface), fixed assignment order.
const SERIES_COLORS: [&str; 8] = [
    "#2a78d6", // blue
    "#eb6834", // orange
    "#1baf7a", // aqua
    "#eda100", // yellow
    "#e87ba4", // magenta
    "#008300", // green
    "#4a3aa7", // violet
    "#e34948", // red
];
const SURFACE: &str = "#fcfcfb";
const INK_PRIMARY: &str = "#0b0b0b";
const INK_SECONDARY: &str = "#52514e";
const GRID: &str = "#e4e3df";
const AXIS: &str = "#d0cfca";
const FONT: &str = "system-ui, -apple-system, 'Segoe UI', sans-serif";

#[derive(Clone, Debug)]
pub struct PlotOptions {
    pub width: u32,
    pub height: u32,
    pub title: String,
    pub x_label: String,
    pub y_label: String,
}

impl Default for PlotOptions {
    fn default() -> Self {
        Self {
            width: 720,
            height: 440,
            title: String::new(),
            x_label: String::new(),
            y_label: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XyKind {
    Line,
    Scatter,
}

/// One xy series; points are pre-cleaned by the caller (no NaN/null).
#[derive(Clone, Debug)]
pub struct XySeries {
    pub label: String,
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
}

// ---------------------------------------------------------------------
// Public renderers
// ---------------------------------------------------------------------

pub fn render_xy(kind: XyKind, series: &[XySeries], opts: &PlotOptions) -> Result<String> {
    if series.is_empty() || series.iter().all(|s| s.xs.is_empty()) {
        return Err(OdsError::InvalidArgument(
            "plot: no data points to draw".to_string(),
        ));
    }
    if series.len() > SERIES_COLORS.len() {
        return Err(OdsError::InvalidArgument(format!(
            "plot: at most {} series per chart (fold the rest or facet)",
            SERIES_COLORS.len()
        )));
    }
    for s in series {
        if s.xs.len() != s.ys.len() {
            return Err(OdsError::LengthMismatch {
                left: s.xs.len(),
                right: s.ys.len(),
            });
        }
    }

    let all_x: Vec<f64> = series.iter().flat_map(|s| s.xs.iter().copied()).collect();
    let all_y: Vec<f64> = series.iter().flat_map(|s| s.ys.iter().copied()).collect();
    let (x_ticks, x_lo, x_hi) = nice_ticks(&all_x);
    let (y_ticks, y_lo, y_hi) = nice_ticks(&all_y);

    let geo = Geometry::new(opts, !series[0].label.is_empty() && series.len() >= 2);
    let mut svg = Svg::open(opts, &geo);
    svg.grid_and_axes(&geo, &y_ticks, y_lo, y_hi);
    svg.x_numeric_ticks(&geo, &x_ticks, x_lo, x_hi);

    for (i, s) in series.iter().enumerate() {
        let color = SERIES_COLORS[i];
        let pts: Vec<(f64, f64)> =
            s.xs.iter()
                .zip(&s.ys)
                .map(|(&x, &y)| (geo.px(x, x_lo, x_hi), geo.py(y, y_lo, y_hi)))
                .collect();
        match kind {
            XyKind::Line => {
                let mut d = String::new();
                for (j, (x, y)) in pts.iter().enumerate() {
                    let _ = write!(d, "{}{:.2},{:.2}", if j == 0 { "M" } else { " L" }, x, y);
                }
                let _ = write!(
                    svg.body,
                    "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\" \
                     stroke-linejoin=\"round\" stroke-linecap=\"round\"/>",
                    d, color
                );
            }
            XyKind::Scatter => {
                for (x, y) in &pts {
                    let _ = write!(
                        svg.body,
                        "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"4\" fill=\"{}\"/>",
                        x, y, color
                    );
                }
            }
        }
    }
    if geo.legend {
        svg.legend(&geo, series.iter().map(|s| s.label.as_str()));
    }
    Ok(svg.close(opts, &geo))
}

pub fn render_bars(labels: &[String], values: &[f64], opts: &PlotOptions) -> Result<String> {
    if labels.len() != values.len() {
        return Err(OdsError::LengthMismatch {
            left: labels.len(),
            right: values.len(),
        });
    }
    if labels.is_empty() {
        return Err(OdsError::InvalidArgument(
            "plot: no data points to draw".to_string(),
        ));
    }
    // Bars are anchored at zero: include it in the scale.
    let mut with_zero: Vec<f64> = values.to_vec();
    with_zero.push(0.0);
    let (y_ticks, y_lo, y_hi) = nice_ticks(&with_zero);

    let geo = Geometry::new(opts, false);
    let mut svg = Svg::open(opts, &geo);
    svg.grid_and_axes(&geo, &y_ticks, y_lo, y_hi);

    let n = values.len() as f64;
    let slot = geo.plot_w / n;
    let bar_w = (slot * 0.72).max(1.0);
    let y0 = geo.py(0.0, y_lo, y_hi);
    for (i, &v) in values.iter().enumerate() {
        let x = geo.left + slot * i as f64 + (slot - bar_w) / 2.0;
        let yv = geo.py(v, y_lo, y_hi);
        let (top, h) = if yv <= y0 {
            (yv, y0 - yv)
        } else {
            (y0, yv - y0)
        };
        let _ = write!(
            svg.body,
            "{}",
            rounded_bar(x, top, bar_w, h, v >= 0.0, SERIES_COLORS[0])
        );
    }
    // Category labels: skip-step when crowded, truncate when long.
    let step = (labels.len() / 12).max(1);
    for (i, label) in labels.iter().enumerate() {
        if i % step != 0 {
            continue;
        }
        let x = geo.left + slot * i as f64 + slot / 2.0;
        let text = truncate(label, 12);
        let _ = write!(
            svg.body,
            "<text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"middle\" font-size=\"11\" fill=\"{}\">{}</text>",
            x,
            geo.top + geo.plot_h + 16.0,
            INK_SECONDARY,
            escape(&text)
        );
    }
    Ok(svg.close(opts, &geo))
}

pub fn render_hist(values: &[f64], bins: usize, opts: &PlotOptions) -> Result<String> {
    if values.is_empty() {
        return Err(OdsError::InvalidArgument(
            "plot: no data points to draw".to_string(),
        ));
    }
    if bins == 0 || bins > 200 {
        return Err(OdsError::InvalidArgument(
            "plot: bins must be between 1 and 200".to_string(),
        ));
    }
    let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let width = if hi > lo { hi - lo } else { 1.0 };
    let mut counts = vec![0f64; bins];
    for &v in values {
        let b = (((v - lo) / width) * bins as f64) as usize;
        counts[b.min(bins - 1)] += 1.0;
    }
    let labels: Vec<String> = (0..bins)
        .map(|b| format_sig(lo + width * (b as f64 + 0.5) / bins as f64, 3))
        .collect();
    render_bars(&labels, &counts, opts)
}

// ---------------------------------------------------------------------
// Layout and drawing helpers
// ---------------------------------------------------------------------

struct Geometry {
    left: f64,
    top: f64,
    plot_w: f64,
    plot_h: f64,
    legend: bool,
}

impl Geometry {
    fn new(opts: &PlotOptions, legend: bool) -> Self {
        let top = if opts.title.is_empty() { 20.0 } else { 44.0 } + if legend { 22.0 } else { 0.0 };
        let bottom = if opts.x_label.is_empty() { 40.0 } else { 58.0 };
        let left = if opts.y_label.is_empty() { 56.0 } else { 74.0 };
        Self {
            left,
            top,
            plot_w: opts.width as f64 - left - 20.0,
            plot_h: opts.height as f64 - top - bottom,
            legend,
        }
    }

    fn px(&self, v: f64, lo: f64, hi: f64) -> f64 {
        self.left + (v - lo) / (hi - lo) * self.plot_w
    }

    fn py(&self, v: f64, lo: f64, hi: f64) -> f64 {
        self.top + self.plot_h - (v - lo) / (hi - lo) * self.plot_h
    }
}

struct Svg {
    body: String,
}

impl Svg {
    fn open(opts: &PlotOptions, _geo: &Geometry) -> Self {
        let mut body = String::with_capacity(4096);
        let _ = write!(
            body,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
             viewBox=\"0 0 {w} {h}\" font-family=\"{font}\">\
             <rect width=\"{w}\" height=\"{h}\" fill=\"{surface}\"/>",
            w = opts.width,
            h = opts.height,
            font = FONT,
            surface = SURFACE
        );
        Self { body }
    }

    fn grid_and_axes(&mut self, geo: &Geometry, y_ticks: &[f64], y_lo: f64, y_hi: f64) {
        for &t in y_ticks {
            let y = geo.py(t, y_lo, y_hi);
            let _ = write!(
                self.body,
                "<line x1=\"{:.2}\" y1=\"{y:.2}\" x2=\"{:.2}\" y2=\"{y:.2}\" stroke=\"{}\" stroke-width=\"1\"/>\
                 <text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"end\" font-size=\"11\" fill=\"{}\">{}</text>",
                geo.left,
                geo.left + geo.plot_w,
                GRID,
                geo.left - 8.0,
                y + 4.0,
                INK_SECONDARY,
                escape(&format_num(t)),
                y = y,
            );
        }
        // Recessive axis lines: left and bottom only.
        let _ = write!(
            self.body,
            "<line x1=\"{l:.2}\" y1=\"{t:.2}\" x2=\"{l:.2}\" y2=\"{b:.2}\" stroke=\"{axis}\"/>\
             <line x1=\"{l:.2}\" y1=\"{b:.2}\" x2=\"{r:.2}\" y2=\"{b:.2}\" stroke=\"{axis}\"/>",
            l = geo.left,
            t = geo.top,
            b = geo.top + geo.plot_h,
            r = geo.left + geo.plot_w,
            axis = AXIS
        );
    }

    fn x_numeric_ticks(&mut self, geo: &Geometry, ticks: &[f64], lo: f64, hi: f64) {
        for &t in ticks {
            let x = geo.px(t, lo, hi);
            let _ = write!(
                self.body,
                "<text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"middle\" font-size=\"11\" fill=\"{}\">{}</text>",
                x,
                geo.top + geo.plot_h + 16.0,
                INK_SECONDARY,
                escape(&format_num(t))
            );
        }
    }

    fn legend<'a>(&mut self, geo: &Geometry, labels: impl Iterator<Item = &'a str>) {
        let mut x = geo.left;
        let y = geo.top - 12.0;
        for (i, label) in labels.enumerate() {
            let _ = write!(
                self.body,
                "<rect x=\"{x:.2}\" y=\"{:.2}\" width=\"10\" height=\"10\" rx=\"2\" fill=\"{}\"/>\
                 <text x=\"{:.2}\" y=\"{:.2}\" font-size=\"11\" fill=\"{}\">{}</text>",
                y - 9.0,
                SERIES_COLORS[i],
                x + 14.0,
                y,
                INK_SECONDARY,
                escape(label),
                x = x,
            );
            x += 22.0 + 7.0 * label.chars().count() as f64;
        }
    }

    fn close(mut self, opts: &PlotOptions, geo: &Geometry) -> String {
        if !opts.title.is_empty() {
            let _ = write!(
                self.body,
                "<text x=\"{:.2}\" y=\"24\" font-size=\"15\" font-weight=\"600\" fill=\"{}\">{}</text>",
                geo.left,
                INK_PRIMARY,
                escape(&opts.title)
            );
        }
        if !opts.x_label.is_empty() {
            let _ = write!(
                self.body,
                "<text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"middle\" font-size=\"12\" fill=\"{}\">{}</text>",
                geo.left + geo.plot_w / 2.0,
                opts.height as f64 - 12.0,
                INK_SECONDARY,
                escape(&opts.x_label)
            );
        }
        if !opts.y_label.is_empty() {
            let _ = write!(
                self.body,
                "<text x=\"16\" y=\"{:.2}\" text-anchor=\"middle\" font-size=\"12\" fill=\"{}\" \
                 transform=\"rotate(-90 16 {:.2})\">{}</text>",
                geo.top + geo.plot_h / 2.0,
                geo.top + geo.plot_h / 2.0,
                INK_SECONDARY,
                escape(&opts.y_label)
            );
        }
        self.body.push_str("</svg>");
        self.body
    }
}

/// A bar with 4px-rounded *data ends* only: the baseline edge stays
/// square (anchored), the far edge rounds.
fn rounded_bar(x: f64, y: f64, w: f64, h: f64, positive: bool, color: &str) -> String {
    let r = 4.0_f64.min(w / 2.0).min(h);
    if h <= 0.0 {
        return String::new();
    }
    let d = if positive {
        // Rounded top, square bottom.
        format!(
            "M{:.2},{b:.2} L{:.2},{ty:.2} Q{:.2},{y:.2} {:.2},{y:.2} L{:.2},{y:.2} Q{:.2},{y:.2} {:.2},{ty:.2} L{:.2},{b:.2} Z",
            x,
            x,
            x,
            x + r,
            x + w - r,
            x + w,
            x + w,
            x + w,
            b = y + h,
            ty = y + r,
            y = y,
        )
    } else {
        // Rounded bottom, square top.
        format!(
            "M{:.2},{y:.2} L{:.2},{by:.2} Q{:.2},{b:.2} {:.2},{b:.2} L{:.2},{b:.2} Q{:.2},{b:.2} {:.2},{by:.2} L{:.2},{y:.2} Z",
            x,
            x,
            x,
            x + r,
            x + w - r,
            x + w,
            x + w,
            x + w,
            y = y,
            b = y + h,
            by = y + h - r,
        )
    };
    format!("<path d=\"{}\" fill=\"{}\"/>", d, color)
}

/// 1-2-5 nice ticks over the data range; returns (ticks, lo, hi) with
/// the range expanded to tick boundaries. Degenerate ranges widen.
fn nice_ticks(values: &[f64]) -> (Vec<f64>, f64, f64) {
    let mut lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let mut hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !lo.is_finite() || !hi.is_finite() {
        lo = 0.0;
        hi = 1.0;
    }
    if lo == hi {
        lo -= 0.5;
        hi += 0.5;
    }
    let span = hi - lo;
    let raw_step = span / 5.0;
    let mag = 10f64.powf(raw_step.log10().floor());
    let norm = raw_step / mag;
    let step = mag
        * if norm <= 1.0 {
            1.0
        } else if norm <= 2.0 {
            2.0
        } else if norm <= 5.0 {
            5.0
        } else {
            10.0
        };
    let lo = (lo / step).floor() * step;
    let hi = (hi / step).ceil() * step;
    let mut ticks = Vec::new();
    let mut t = lo;
    while t <= hi + step * 1e-9 {
        // Snap tiny float error so labels read clean.
        ticks.push((t / step).round() * step);
        t += step;
    }
    (ticks, lo, hi)
}

/// Round to `sig` significant figures — bin labels read as "104", not
/// "104.2966".
fn format_sig(v: f64, sig: i32) -> String {
    if v == 0.0 || !v.is_finite() {
        return format_num(v);
    }
    let mag = v.abs().log10().floor() as i32;
    let factor = 10f64.powi(sig - 1 - mag);
    format_num((v * factor).round() / factor)
}

fn format_num(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    if v.abs() >= 1e6 || v.abs() < 1e-4 {
        return format!("{:e}", v);
    }
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}…", cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> PlotOptions {
        PlotOptions {
            title: "Test <chart> & more".to_string(),
            x_label: "x".to_string(),
            y_label: "y".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn line_svg_structure() {
        let s = render_xy(
            XyKind::Line,
            &[XySeries {
                label: String::new(),
                xs: vec![0.0, 1.0, 2.0],
                ys: vec![1.0, 3.0, 2.0],
            }],
            &opts(),
        )
        .unwrap();
        assert!(s.starts_with("<svg xmlns"));
        assert!(s.ends_with("</svg>"));
        assert!(s.contains("stroke-width=\"2\""));
        assert!(s.matches("<path").count() >= 1);
        // Title is escaped, never raw.
        assert!(s.contains("Test &lt;chart&gt; &amp; more"));
        assert!(!s.contains("<chart>"));
    }

    #[test]
    fn scatter_marker_count_and_legend_rules() {
        let one = render_xy(
            XyKind::Scatter,
            &[XySeries {
                label: "a".to_string(),
                xs: vec![1.0, 2.0],
                ys: vec![1.0, 2.0],
            }],
            &PlotOptions::default(),
        )
        .unwrap();
        assert_eq!(one.matches("<circle").count(), 2);
        // Single series: no legend swatch.
        assert_eq!(one.matches("rx=\"2\"").count(), 0);

        let two = render_xy(
            XyKind::Line,
            &[
                XySeries {
                    label: "alpha".to_string(),
                    xs: vec![0.0, 1.0],
                    ys: vec![0.0, 1.0],
                },
                XySeries {
                    label: "beta".to_string(),
                    xs: vec![0.0, 1.0],
                    ys: vec![1.0, 0.0],
                },
            ],
            &PlotOptions::default(),
        )
        .unwrap();
        // Two series: legend appears, colors in fixed order.
        assert!(two.contains(SERIES_COLORS[0]));
        assert!(two.contains(SERIES_COLORS[1]));
        assert!(two.contains(">alpha<"));
        assert!(two.contains(">beta<"));
    }

    #[test]
    fn bars_and_hist() {
        let s = render_bars(
            &["a".to_string(), "b".to_string()],
            &[3.0, -1.5],
            &PlotOptions::default(),
        )
        .unwrap();
        // Two bars plus axes; negative bar renders below the zero line.
        assert!(s.matches("<path").count() >= 2);
        assert!(s.contains(">a<") && s.contains(">b<"));

        let h = render_hist(&[1.0, 1.1, 1.2, 5.0, 5.1], 4, &PlotOptions::default()).unwrap();
        assert!(h.starts_with("<svg"));
        assert!(render_hist(&[], 4, &PlotOptions::default()).is_err());
        assert!(render_hist(&[1.0], 0, &PlotOptions::default()).is_err());
    }

    #[test]
    fn errors_and_limits() {
        assert!(render_xy(XyKind::Line, &[], &PlotOptions::default()).is_err());
        let too_many: Vec<XySeries> = (0..9)
            .map(|i| XySeries {
                label: format!("s{}", i),
                xs: vec![0.0],
                ys: vec![0.0],
            })
            .collect();
        assert!(render_xy(XyKind::Line, &too_many, &PlotOptions::default()).is_err());
        let bad = XySeries {
            label: String::new(),
            xs: vec![0.0, 1.0],
            ys: vec![0.0],
        };
        assert!(render_xy(XyKind::Line, &[bad], &PlotOptions::default()).is_err());
    }

    #[test]
    fn nice_ticks_are_clean() {
        let (ticks, lo, hi) = nice_ticks(&[0.3, 9.7]);
        assert_eq!(lo, 0.0);
        assert_eq!(hi, 10.0);
        assert_eq!(ticks, vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        let (t2, _, _) = nice_ticks(&[5.0, 5.0]);
        assert!(t2.len() >= 2);
    }
}
