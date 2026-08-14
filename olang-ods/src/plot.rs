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
const SERIES_COLORS: [&str; 10] = [
    "#2a78d6", // blue
    "#eb6834", // orange
    "#1baf7a", // aqua
    "#eda100", // yellow
    "#e87ba4", // magenta
    "#008300", // green
    "#4a3aa7", // violet
    "#e34948", // red
    "#00838f", // teal
    "#8d6e63", // umber
];
// The same discipline re-tuned for a dark surface (the example suite's
// panels): ten distinct hues, mint leading, assigned in fixed order.
const SERIES_COLORS_DARK: [&str; 10] = [
    "#3ddc97", // mint — the suite's lead accent
    "#5aa9e6", // blue
    "#f4b84c", // amber
    "#f0854a", // coral
    "#ef8bb0", // magenta
    "#8b7ae0", // violet
    "#4dd0e1", // cyan
    "#a3d977", // lime
    "#ef6b73", // red
    "#2bb8a3", // teal
];
const FONT: &str = "system-ui, -apple-system, 'Segoe UI', sans-serif";

/// Surface theme: every color the renderer touches routes through this,
/// so a chart is dark-ready by option rather than by post-processing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    fn surface(self) -> &'static str {
        match self {
            Theme::Light => "#fcfcfb",
            Theme::Dark => "#101720",
        }
    }
    fn ink(self) -> &'static str {
        match self {
            Theme::Light => "#0b0b0b",
            Theme::Dark => "#d9e6ef",
        }
    }
    fn ink2(self) -> &'static str {
        match self {
            Theme::Light => "#52514e",
            Theme::Dark => "#7f93a3",
        }
    }
    fn grid(self) -> &'static str {
        match self {
            Theme::Light => "#e4e3df",
            Theme::Dark => "#1d2937",
        }
    }
    fn axis(self) -> &'static str {
        match self {
            Theme::Light => "#d0cfca",
            Theme::Dark => "#2b3b4f",
        }
    }
    fn series(self, i: usize) -> &'static str {
        match self {
            Theme::Light => SERIES_COLORS[i],
            Theme::Dark => SERIES_COLORS_DARK[i],
        }
    }
    /// Sequential scale endpoints for heatmaps: near-surface to full hue.
    fn heat(self) -> ((u8, u8, u8), (u8, u8, u8)) {
        match self {
            Theme::Light => ((232, 238, 248), (42, 120, 214)),
            Theme::Dark => ((16, 23, 32), (61, 220, 151)),
        }
    }
}

/// Named color ramps for continuous data. `Auto` follows the theme's
/// sequential scale; the rest are hand-tuned multi-stop gradients —
/// perceptual cousins of the scientific colormaps, chosen to sit well
/// on the suite's dark panels (and acceptably on light).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HeatScale {
    #[default]
    Auto,
    Ocean,
    Ember,
    Thermal,
    Diverging,
}

impl HeatScale {
    pub fn parse(name: &str) -> Option<HeatScale> {
        Some(match name {
            "auto" => HeatScale::Auto,
            "ocean" => HeatScale::Ocean,
            "ember" => HeatScale::Ember,
            "thermal" => HeatScale::Thermal,
            "diverging" => HeatScale::Diverging,
            _ => return None,
        })
    }

    fn stops(self, theme: Theme) -> Vec<(u8, u8, u8)> {
        match self {
            HeatScale::Auto => {
                let (lo, hi) = theme.heat();
                vec![lo, hi]
            }
            HeatScale::Ocean => vec![(11, 32, 58), (29, 111, 184), (77, 208, 225)],
            HeatScale::Ember => vec![(30, 16, 26), (140, 46, 76), (233, 105, 82), (244, 184, 76)],
            HeatScale::Thermal => vec![
                (14, 16, 38),
                (84, 39, 128),
                (190, 66, 143),
                (240, 133, 74),
                (255, 231, 197),
            ],
            // Signed data: cold through the surface to warm.
            HeatScale::Diverging => match theme {
                Theme::Dark => vec![(90, 169, 230), (18, 26, 36), (244, 184, 76)],
                Theme::Light => vec![(42, 120, 214), (247, 247, 244), (235, 104, 52)],
            },
        }
    }
}

/// Piecewise-linear interpolation over a ramp's stops, t in [0, 1].
pub fn ramp_color(scale: HeatScale, theme: Theme, t: f64) -> String {
    let stops = scale.stops(theme);
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f64;
    let i = (t.floor() as usize).min(stops.len() - 2);
    let f = t - i as f64;
    let (r0, g0, b0) = stops[i];
    let (r1, g1, b1) = stops[i + 1];
    let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        lerp(r0, r1),
        lerp(g0, g1),
        lerp(b0, b1)
    )
}

#[derive(Clone, Debug)]
pub struct PlotOptions {
    pub width: u32,
    pub height: u32,
    pub title: String,
    pub x_label: String,
    pub y_label: String,
    pub theme: Theme,
    /// When set, the SVG carries no fixed pixel size — the viewBox plus
    /// `width:100%` lets the *container* size it (the browser case).
    pub responsive: bool,
    /// When set, marks carry their datum as `data-*` attributes —
    /// scatter points, bars, heatmap cells, and boxes become event
    /// targets the browser's delegated handlers can read.
    pub interactive: bool,
    /// Per-chart palette override: series take these colors in order
    /// (cycling), falling back to the theme palette when empty.
    pub colors: Vec<String>,
    /// Single-series bar charts color each category from the palette —
    /// categorical identity, the classic statistical-graphics look.
    pub vary: bool,
    /// The heatmap's color ramp.
    pub scale: HeatScale,
}

impl Default for PlotOptions {
    fn default() -> Self {
        Self {
            width: 720,
            height: 440,
            title: String::new(),
            x_label: String::new(),
            y_label: String::new(),
            theme: Theme::default(),
            responsive: false,
            interactive: false,
            colors: Vec::new(),
            vary: false,
            scale: HeatScale::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XyKind {
    Line,
    Scatter,
    /// Line with the region down to the plot floor filled translucently.
    Area,
}

/// Resolve series color `i`: the chart's own palette first, the theme's
/// otherwise. Every renderer routes through here, so `colors` works
/// uniformly across marks, legends, and gradients.
fn pick(opts: &PlotOptions, i: usize) -> &str {
    if opts.colors.is_empty() {
        opts.theme.series(i % SERIES_COLORS_DARK.len())
    } else {
        &opts.colors[i % opts.colors.len()]
    }
}

/// One xy series; points are pre-cleaned by the caller (no NaN/null).
/// `kind` is per-series so layered charts (an area under a line under
/// markers) render into one document with shared scales.
#[derive(Clone, Debug)]
pub struct XySeries {
    pub label: String,
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    pub kind: XyKind,
    /// Scatter only: one color per point (continuous color encoding).
    /// Empty means the series color. Length must match xs/ys.
    pub point_colors: Vec<String>,
}

// ---------------------------------------------------------------------
// Public renderers
// ---------------------------------------------------------------------

pub fn render_xy(series: &[XySeries], opts: &PlotOptions) -> Result<String> {
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
        if !s.point_colors.is_empty() && s.point_colors.len() != s.xs.len() {
            return Err(OdsError::LengthMismatch {
                left: s.xs.len(),
                right: s.point_colors.len(),
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
        let color = pick(opts, i);
        let pts: Vec<(f64, f64)> =
            s.xs.iter()
                .zip(&s.ys)
                .map(|(&x, &y)| (geo.px(x, x_lo, x_hi), geo.py(y, y_lo, y_hi)))
                .collect();
        match s.kind {
            XyKind::Line | XyKind::Area => {
                let mut d = String::new();
                for (j, (x, y)) in pts.iter().enumerate() {
                    let _ = write!(d, "{}{:.2},{:.2}", if j == 0 { "M" } else { " L" }, x, y);
                }
                if s.kind == XyKind::Area {
                    // Fill to the plot floor with a vertical gradient
                    // fading toward the axis — the fill stays a hint,
                    // the line carries the value. Gradient ids derive
                    // from the color, so identical definitions collide
                    // harmlessly when several charts share a page.
                    let floor = geo.top + geo.plot_h;
                    let (first, last) = (pts[0].0, pts[pts.len() - 1].0);
                    let gid = format!("vzg{}", color.trim_start_matches('#'));
                    let _ = write!(
                        svg.body,
                        "<linearGradient id=\"{gid}\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\
                         <stop offset=\"0\" stop-color=\"{color}\" stop-opacity=\"0.5\"/>\
                         <stop offset=\"1\" stop-color=\"{color}\" stop-opacity=\"0.03\"/>\
                         </linearGradient>\
                         <path d=\"{d} L{last:.2},{floor:.2} L{first:.2},{floor:.2} Z\" \
                         fill=\"url(#{gid})\" stroke=\"none\"/>",
                    );
                }
                let _ = write!(
                    svg.body,
                    "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\" \
                     stroke-linejoin=\"round\" stroke-linecap=\"round\"/>",
                    d, color
                );
            }
            XyKind::Scatter => {
                for (j, (x, y)) in pts.iter().enumerate() {
                    let attrs = data_attrs(
                        opts.interactive,
                        &[
                            ("s", s.label.clone()),
                            ("x", format_num(s.xs[j])),
                            ("y", format_num(s.ys[j])),
                        ],
                    );
                    let fill = if s.point_colors.is_empty() {
                        color
                    } else {
                        &s.point_colors[j]
                    };
                    let _ = write!(
                        svg.body,
                        "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"4\" fill=\"{}\"{}/>",
                        x, y, fill, attrs
                    );
                }
            }
        }
    }
    if geo.legend {
        svg.legend(
            &geo,
            series
                .iter()
                .enumerate()
                .map(|(i, s)| (s.label.as_str(), pick(opts, i))),
        );
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
        let attrs = data_attrs(
            opts.interactive,
            &[("label", labels[i].clone()), ("value", format_num(v))],
        );
        let color = if opts.vary {
            pick(opts, i)
        } else {
            pick(opts, 0)
        };
        let _ = write!(
            svg.body,
            "{}",
            rounded_bar(x, top, bar_w, h, v >= 0.0, color, &attrs)
        );
    }
    svg.x_category_labels(&geo, labels, slot);
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

/// One named value-list per series, aligned to the shared category labels.
#[derive(Clone, Debug)]
pub struct BarSeries {
    pub label: String,
    pub values: Vec<f64>,
}

/// Grouped (side-by-side) or stacked bars for several series over the
/// same categories. Stacking is a part-of-whole statement, so stacked
/// bars refuse negative values — a negative part draws a lie.
pub fn render_bar_groups(
    labels: &[String],
    series: &[BarSeries],
    stacked: bool,
    opts: &PlotOptions,
) -> Result<String> {
    if labels.is_empty() || series.is_empty() {
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
        if s.values.len() != labels.len() {
            return Err(OdsError::LengthMismatch {
                left: labels.len(),
                right: s.values.len(),
            });
        }
        if stacked && s.values.iter().any(|&v| v < 0.0) {
            return Err(OdsError::InvalidArgument(
                "plot.stacked: values must be non-negative (a negative part misleads)".to_string(),
            ));
        }
    }

    // Scale: grouped spans every value; stacked spans category sums.
    let mut extent: Vec<f64> = vec![0.0];
    if stacked {
        for i in 0..labels.len() {
            extent.push(series.iter().map(|s| s.values[i]).sum());
        }
    } else {
        extent.extend(series.iter().flat_map(|s| s.values.iter().copied()));
    }
    let (y_ticks, y_lo, y_hi) = nice_ticks(&extent);

    let geo = Geometry::new(opts, series.len() >= 2);
    let mut svg = Svg::open(opts, &geo);
    svg.grid_and_axes(&geo, &y_ticks, y_lo, y_hi);

    let slot = geo.plot_w / labels.len() as f64;
    let y0 = geo.py(0.0, y_lo, y_hi);
    if stacked {
        let bar_w = (slot * 0.72).max(1.0);
        for (i, _) in labels.iter().enumerate() {
            let x = geo.left + slot * i as f64 + (slot - bar_w) / 2.0;
            let mut running = 0.0;
            for (si, s) in series.iter().enumerate() {
                let v = s.values[i];
                if v <= 0.0 {
                    continue;
                }
                let top = geo.py(running + v, y_lo, y_hi);
                let bottom = geo.py(running, y_lo, y_hi);
                // Only the topmost segment gets the rounded data-end.
                let last = series[si + 1..].iter().all(|r| r.values[i] <= 0.0);
                let attrs = data_attrs(
                    opts.interactive,
                    &[
                        ("s", s.label.clone()),
                        ("label", labels[i].clone()),
                        ("value", format_num(v)),
                    ],
                );
                let seg = if last {
                    rounded_bar(
                        x,
                        top,
                        bar_w,
                        bottom - top,
                        true,
                        opts.theme.series(si),
                        &attrs,
                    )
                } else {
                    format!(
                        "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"{}\"{}/>",
                        x,
                        top,
                        bar_w,
                        bottom - top,
                        pick(opts, si),
                        attrs
                    )
                };
                let _ = write!(svg.body, "{}", seg);
                running += v;
            }
        }
    } else {
        let group_w = (slot * 0.78).max(1.0);
        let bar_w = (group_w / series.len() as f64 - 2.0).max(1.0);
        for (i, _) in labels.iter().enumerate() {
            let start = geo.left + slot * i as f64 + (slot - group_w) / 2.0;
            for (si, s) in series.iter().enumerate() {
                let v = s.values[i];
                let x = start + (bar_w + 2.0) * si as f64;
                let yv = geo.py(v, y_lo, y_hi);
                let (top, h) = if yv <= y0 {
                    (yv, y0 - yv)
                } else {
                    (y0, yv - y0)
                };
                let attrs = data_attrs(
                    opts.interactive,
                    &[
                        ("s", s.label.clone()),
                        ("label", labels[i].clone()),
                        ("value", format_num(v)),
                    ],
                );
                let _ = write!(
                    svg.body,
                    "{}",
                    rounded_bar(x, top, bar_w, h, v >= 0.0, pick(opts, si), &attrs)
                );
            }
        }
    }
    svg.x_category_labels(&geo, labels, slot);
    if geo.legend {
        svg.legend(
            &geo,
            series
                .iter()
                .enumerate()
                .map(|(i, s)| (s.label.as_str(), pick(opts, i))),
        );
    }
    Ok(svg.close(opts, &geo))
}

/// A matrix of values as colored cells: rows[r][c] maps to the cell at
/// (x_labels[c], y_labels[r]), colored on a sequential scale from the
/// theme's surface toward its primary hue. Row 0 renders at the top.
pub fn render_heatmap(
    x_labels: &[String],
    y_labels: &[String],
    rows: &[Vec<f64>],
    opts: &PlotOptions,
) -> Result<String> {
    if rows.is_empty() || x_labels.is_empty() || y_labels.is_empty() {
        return Err(OdsError::InvalidArgument(
            "plot: no data points to draw".to_string(),
        ));
    }
    if rows.len() != y_labels.len() {
        return Err(OdsError::LengthMismatch {
            left: y_labels.len(),
            right: rows.len(),
        });
    }
    for row in rows {
        if row.len() != x_labels.len() {
            return Err(OdsError::LengthMismatch {
                left: x_labels.len(),
                right: row.len(),
            });
        }
    }
    let all: Vec<f64> = rows.iter().flatten().copied().collect();
    if all.iter().any(|v| !v.is_finite()) {
        return Err(OdsError::InvalidArgument(
            "plot.heatmap: values must be finite".to_string(),
        ));
    }
    let lo = all.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = all.iter().copied().fold(f64::NEG_INFINITY, f64::max);

    let geo = Geometry::new(opts, false);
    let mut svg = Svg::open(opts, &geo);
    let cell_w = geo.plot_w / x_labels.len() as f64;
    let cell_h = geo.plot_h / y_labels.len() as f64;
    for (r, row) in rows.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            let t = if hi > lo { (v - lo) / (hi - lo) } else { 0.5 };
            let attrs = data_attrs(
                opts.interactive,
                &[
                    ("xl", x_labels[c].clone()),
                    ("yl", y_labels[r].clone()),
                    ("value", format_num(v)),
                ],
            );
            let _ = write!(
                svg.body,
                "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" rx=\"2\" fill=\"{}\"{}/>",
                geo.left + cell_w * c as f64 + 0.5,
                geo.top + cell_h * r as f64 + 0.5,
                cell_w - 1.0,
                cell_h - 1.0,
                ramp_color(opts.scale, opts.theme, t),
                attrs,
            );
        }
    }
    svg.x_category_labels(&geo, x_labels, cell_w);
    // Row labels down the left, skip-stepped like the x labels.
    let step = (y_labels.len() / 16).max(1);
    for (r, label) in y_labels.iter().enumerate() {
        if r % step != 0 {
            continue;
        }
        let _ = write!(
            svg.body,
            "<text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"end\" font-size=\"12.5\" fill=\"{}\">{}</text>",
            geo.left - 8.0,
            geo.top + cell_h * r as f64 + cell_h / 2.0 + 4.0,
            opts.theme.ink2(),
            escape(&truncate(label, 12)),
        );
    }
    // A minimal scale key: lo and hi swatches in the top-right corner.
    let key_x = geo.left + geo.plot_w - 120.0;
    let key_y = geo.top - 12.0;
    let _ = write!(
        svg.body,
        "<rect x=\"{key_x:.2}\" y=\"{:.2}\" width=\"10\" height=\"10\" rx=\"2\" fill=\"{}\"/>\
         <text x=\"{:.2}\" y=\"{key_y:.2}\" font-size=\"12.5\" fill=\"{ink}\">{}</text>\
         <rect x=\"{:.2}\" y=\"{:.2}\" width=\"10\" height=\"10\" rx=\"2\" fill=\"{}\"/>\
         <text x=\"{:.2}\" y=\"{key_y:.2}\" font-size=\"12.5\" fill=\"{ink}\">{}</text>",
        key_y - 9.0,
        ramp_color(opts.scale, opts.theme, 0.0),
        key_x + 14.0,
        escape(&format_num(lo)),
        key_x + 60.0,
        key_y - 9.0,
        ramp_color(opts.scale, opts.theme, 1.0),
        key_x + 74.0,
        escape(&format_num(hi)),
        ink = opts.theme.ink2(),
    );
    Ok(svg.close(opts, &geo))
}

/// Five-number-summary box plots, one per labeled series: whiskers to
/// min/max, a quartile box, and the median as the emphasized line.
pub fn render_box(series: &[BarSeries], opts: &PlotOptions) -> Result<String> {
    if series.is_empty() || series.iter().any(|s| s.values.is_empty()) {
        return Err(OdsError::InvalidArgument(
            "plot: no data points to draw".to_string(),
        ));
    }
    let all: Vec<f64> = series
        .iter()
        .flat_map(|s| s.values.iter().copied())
        .collect();
    if all.iter().any(|v| !v.is_finite()) {
        return Err(OdsError::InvalidArgument(
            "plot.box: values must be finite".to_string(),
        ));
    }
    let (y_ticks, y_lo, y_hi) = nice_ticks(&all);

    let geo = Geometry::new(opts, false);
    let mut svg = Svg::open(opts, &geo);
    svg.grid_and_axes(&geo, &y_ticks, y_lo, y_hi);

    let slot = geo.plot_w / series.len() as f64;
    let box_w = (slot * 0.44).max(2.0);
    let color = pick(opts, 0);
    for (i, s) in series.iter().enumerate() {
        let mut sorted = s.values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite checked above"));
        let [min, q1, med, q3, max] = [0.0, 0.25, 0.5, 0.75, 1.0].map(|q| quantile(&sorted, q));
        let cx = geo.left + slot * i as f64 + slot / 2.0;
        let x = cx - box_w / 2.0;
        let (py_min, py_q1, py_med, py_q3, py_max) = (
            geo.py(min, y_lo, y_hi),
            geo.py(q1, y_lo, y_hi),
            geo.py(med, y_lo, y_hi),
            geo.py(q3, y_lo, y_hi),
            geo.py(max, y_lo, y_hi),
        );
        let _ = write!(
            svg.body,
            "<line x1=\"{cx:.2}\" y1=\"{py_min:.2}\" x2=\"{cx:.2}\" y2=\"{py_q1:.2}\" stroke=\"{color}\"/>\
             <line x1=\"{cx:.2}\" y1=\"{py_q3:.2}\" x2=\"{cx:.2}\" y2=\"{py_max:.2}\" stroke=\"{color}\"/>\
             <line x1=\"{:.2}\" y1=\"{py_min:.2}\" x2=\"{:.2}\" y2=\"{py_min:.2}\" stroke=\"{color}\"/>\
             <line x1=\"{:.2}\" y1=\"{py_max:.2}\" x2=\"{:.2}\" y2=\"{py_max:.2}\" stroke=\"{color}\"/>\
             <rect x=\"{x:.2}\" y=\"{py_q3:.2}\" width=\"{box_w:.2}\" height=\"{:.2}\" rx=\"2\" \
             fill=\"{color}\" fill-opacity=\"0.28\" stroke=\"{color}\"{attrs}/>\
             <line x1=\"{x:.2}\" y1=\"{py_med:.2}\" x2=\"{:.2}\" y2=\"{py_med:.2}\" \
             stroke=\"{color}\" stroke-width=\"2\"/>",
            cx - box_w / 4.0,
            cx + box_w / 4.0,
            cx - box_w / 4.0,
            cx + box_w / 4.0,
            py_q1 - py_q3,
            x + box_w,
            attrs = data_attrs(
                opts.interactive,
                &[
                    ("s", s.label.clone()),
                    ("med", format_num(med)),
                    ("q1", format_num(q1)),
                    ("q3", format_num(q3)),
                    ("lo", format_num(min)),
                    ("hi", format_num(max)),
                ],
            ),
        );
    }
    let labels: Vec<String> = series.iter().map(|s| s.label.clone()).collect();
    svg.x_category_labels(&geo, &labels, slot);
    Ok(svg.close(opts, &geo))
}

/// Quantile by linear interpolation over a sorted slice (type 7, the
/// same convention the stats namespace uses).
fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = q * (sorted.len() - 1) as f64;
    let base = pos.floor() as usize;
    let frac = pos - base as f64;
    if base + 1 < sorted.len() {
        sorted[base] + frac * (sorted[base + 1] - sorted[base])
    } else {
        sorted[base]
    }
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
    theme: Theme,
}

impl Svg {
    fn open(opts: &PlotOptions, _geo: &Geometry) -> Self {
        let mut body = String::with_capacity(4096);
        let size = if opts.responsive {
            "style=\"width:100%;height:auto\"".to_string()
        } else {
            format!("width=\"{}\" height=\"{}\"", opts.width, opts.height)
        };
        let _ = write!(
            body,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" {size} \
             viewBox=\"0 0 {w} {h}\" font-family=\"{font}\">\
             <rect width=\"{w}\" height=\"{h}\" fill=\"{surface}\"/>",
            w = opts.width,
            h = opts.height,
            font = FONT,
            surface = opts.theme.surface(),
        );
        Self {
            body,
            theme: opts.theme,
        }
    }

    fn grid_and_axes(&mut self, geo: &Geometry, y_ticks: &[f64], y_lo: f64, y_hi: f64) {
        for &t in y_ticks {
            let y = geo.py(t, y_lo, y_hi);
            let _ = write!(
                self.body,
                "<line x1=\"{:.2}\" y1=\"{y:.2}\" x2=\"{:.2}\" y2=\"{y:.2}\" stroke=\"{}\" stroke-width=\"1\"/>\
                 <text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"end\" font-size=\"12.5\" fill=\"{}\">{}</text>",
                geo.left,
                geo.left + geo.plot_w,
                self.theme.grid(),
                geo.left - 8.0,
                y + 4.0,
                self.theme.ink2(),
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
            axis = self.theme.axis(),
        );
    }

    fn x_numeric_ticks(&mut self, geo: &Geometry, ticks: &[f64], lo: f64, hi: f64) {
        for &t in ticks {
            let x = geo.px(t, lo, hi);
            let _ = write!(
                self.body,
                "<text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"middle\" font-size=\"12.5\" fill=\"{}\">{}</text>",
                x,
                geo.top + geo.plot_h + 16.0,
                self.theme.ink2(),
                escape(&format_num(t))
            );
        }
    }

    /// Category tick labels along the x axis, skip-stepped when crowded
    /// and truncated when long — bars, heatmaps, and boxes all share it.
    fn x_category_labels(&mut self, geo: &Geometry, labels: &[String], slot: f64) {
        let step = (labels.len() / 12).max(1);
        for (i, label) in labels.iter().enumerate() {
            if i % step != 0 {
                continue;
            }
            let x = geo.left + slot * i as f64 + slot / 2.0;
            let text = truncate(label, 12);
            let _ = write!(
                self.body,
                "<text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"middle\" font-size=\"12.5\" fill=\"{}\">{}</text>",
                x,
                geo.top + geo.plot_h + 16.0,
                self.theme.ink2(),
                escape(&text)
            );
        }
    }

    fn legend<'a>(&mut self, geo: &Geometry, entries: impl Iterator<Item = (&'a str, &'a str)>) {
        let mut x = geo.left;
        let y = geo.top - 12.0;
        for (label, color) in entries {
            let _ = write!(
                self.body,
                "<rect x=\"{x:.2}\" y=\"{:.2}\" width=\"10\" height=\"10\" rx=\"2\" fill=\"{}\"/>\
                 <text x=\"{:.2}\" y=\"{:.2}\" font-size=\"12.5\" fill=\"{}\">{}</text>",
                y - 9.0,
                color,
                x + 14.0,
                y,
                self.theme.ink2(),
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
                "<text x=\"{:.2}\" y=\"25\" font-size=\"16.5\" font-weight=\"600\" fill=\"{}\">{}</text>",
                geo.left,
                self.theme.ink(),
                escape(&opts.title)
            );
        }
        if !opts.x_label.is_empty() {
            let _ = write!(
                self.body,
                "<text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"middle\" font-size=\"13\" fill=\"{}\">{}</text>",
                geo.left + geo.plot_w / 2.0,
                opts.height as f64 - 12.0,
                self.theme.ink2(),
                escape(&opts.x_label)
            );
        }
        if !opts.y_label.is_empty() {
            let _ = write!(
                self.body,
                "<text x=\"16\" y=\"{:.2}\" text-anchor=\"middle\" font-size=\"13\" fill=\"{}\" \
                 transform=\"rotate(-90 16 {:.2})\">{}</text>",
                geo.top + geo.plot_h / 2.0,
                geo.top + geo.plot_h / 2.0,
                self.theme.ink2(),
                escape(&opts.y_label)
            );
        }
        self.body.push_str("</svg>");
        self.body
    }
}

/// `data-*` attribute text for an interactive mark, or empty.
fn data_attrs(on: bool, pairs: &[(&str, String)]) -> String {
    if !on {
        return String::new();
    }
    let mut out = String::new();
    for (k, v) in pairs {
        let _ = write!(out, " data-{}=\"{}\"", k, escape(v));
    }
    out
}

/// A bar with 4px-rounded *data ends* only: the baseline edge stays
/// square (anchored), the far edge rounds.
fn rounded_bar(x: f64, y: f64, w: f64, h: f64, positive: bool, color: &str, attrs: &str) -> String {
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
    format!("<path d=\"{}\" fill=\"{}\"{}/>", d, color, attrs)
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
            &[XySeries {
                label: String::new(),
                xs: vec![0.0, 1.0, 2.0],
                ys: vec![1.0, 3.0, 2.0],
                kind: XyKind::Line,
                point_colors: vec![],
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
            &[XySeries {
                label: "a".to_string(),
                xs: vec![1.0, 2.0],
                ys: vec![1.0, 2.0],
                kind: XyKind::Scatter,
                point_colors: vec![],
            }],
            &PlotOptions::default(),
        )
        .unwrap();
        assert_eq!(one.matches("<circle").count(), 2);
        // Single series: no legend swatch.
        assert_eq!(one.matches("rx=\"2\"").count(), 0);

        let two = render_xy(
            &[
                XySeries {
                    label: "alpha".to_string(),
                    xs: vec![0.0, 1.0],
                    ys: vec![0.0, 1.0],
                    kind: XyKind::Line,
                    point_colors: vec![],
                },
                XySeries {
                    label: "beta".to_string(),
                    xs: vec![0.0, 1.0],
                    ys: vec![1.0, 0.0],
                    kind: XyKind::Line,
                    point_colors: vec![],
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
        assert!(render_xy(&[], &PlotOptions::default()).is_err());
        let too_many: Vec<XySeries> = (0..11)
            .map(|i| XySeries {
                label: format!("s{}", i),
                xs: vec![0.0],
                ys: vec![0.0],
                kind: XyKind::Line,
                point_colors: vec![],
            })
            .collect();
        assert!(render_xy(&too_many, &PlotOptions::default()).is_err());
        let bad = XySeries {
            label: String::new(),
            xs: vec![0.0, 1.0],
            ys: vec![0.0],
            kind: XyKind::Line,
            point_colors: vec![],
        };
        assert!(render_xy(&[bad], &PlotOptions::default()).is_err());
    }

    #[test]
    fn dark_theme_and_responsive_sizing() {
        let dark = render_xy(
            &[XySeries {
                label: String::new(),
                xs: vec![0.0, 1.0],
                ys: vec![0.0, 1.0],
                kind: XyKind::Line,
                point_colors: vec![],
            }],
            &PlotOptions {
                theme: Theme::Dark,
                responsive: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(dark.contains(Theme::Dark.surface()));
        assert!(dark.contains(SERIES_COLORS_DARK[0]));
        assert!(!dark.contains(SERIES_COLORS[0]));
        // Responsive: the svg tag carries container-driven sizing, not
        // fixed pixel attributes (the background rect keeps viewBox units).
        assert!(dark.starts_with(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" style=\"width:100%;height:auto\""
        ));
        // The viewBox still fixes the aspect ratio.
        assert!(dark.contains("viewBox=\"0 0 720 440\""));
    }

    #[test]
    fn area_fills_below_the_line() {
        let s = render_xy(
            &[XySeries {
                label: String::new(),
                xs: vec![0.0, 1.0, 2.0],
                ys: vec![1.0, 3.0, 2.0],
                kind: XyKind::Area,
                point_colors: vec![],
            }],
            &PlotOptions::default(),
        )
        .unwrap();
        assert!(s.contains("<linearGradient id=\"vzg"));
        assert!(s.contains("stop-opacity=\"0.5\""));
        // The stroke line still draws on top of the fill.
        assert!(s.contains("stroke-width=\"2\""));
    }

    fn two_bar_series() -> Vec<BarSeries> {
        vec![
            BarSeries {
                label: "a".to_string(),
                values: vec![1.0, 2.0],
            },
            BarSeries {
                label: "b".to_string(),
                values: vec![3.0, 4.0],
            },
        ]
    }

    #[test]
    fn grouped_and_stacked_bars() {
        let labels = vec!["q1".to_string(), "q2".to_string()];
        let grouped =
            render_bar_groups(&labels, &two_bar_series(), false, &PlotOptions::default()).unwrap();
        // 2 categories × 2 series bars, both series colors, a legend.
        assert!(grouped.matches("<path").count() >= 4);
        assert!(grouped.contains(SERIES_COLORS[0]) && grouped.contains(SERIES_COLORS[1]));
        assert!(grouped.contains(">a<") && grouped.contains(">b<"));

        let stacked =
            render_bar_groups(&labels, &two_bar_series(), true, &PlotOptions::default()).unwrap();
        assert!(stacked.contains("<rect")); // lower segments are square
        assert!(stacked.contains(SERIES_COLORS[1]));

        // Stacked refuses negatives; grouped allows them.
        let mut neg = two_bar_series();
        neg[0].values[0] = -1.0;
        assert!(render_bar_groups(&labels, &neg, true, &PlotOptions::default()).is_err());
        assert!(render_bar_groups(&labels, &neg, false, &PlotOptions::default()).is_ok());

        // Length mismatch refuses.
        let mut ragged = two_bar_series();
        ragged[1].values.pop();
        assert!(render_bar_groups(&labels, &ragged, false, &PlotOptions::default()).is_err());
    }

    #[test]
    fn heatmap_cells_and_scale() {
        let xs = vec!["mon".to_string(), "tue".to_string()];
        let ys = vec!["am".to_string(), "pm".to_string()];
        let rows = vec![vec![0.0, 1.0], vec![2.0, 3.0]];
        let s = render_heatmap(&xs, &ys, &rows, &PlotOptions::default()).unwrap();
        // 4 cells + 2 key swatches; extreme cells hit the scale endpoints.
        assert_eq!(s.matches("rx=\"2\"").count(), 6);
        assert!(s.contains(&ramp_color(HeatScale::Auto, Theme::Light, 0.0)));
        assert!(s.contains(&ramp_color(HeatScale::Auto, Theme::Light, 1.0)));
        assert!(s.contains(">am<") && s.contains(">mon<"));

        let ragged = vec![vec![0.0, 1.0], vec![2.0]];
        assert!(render_heatmap(&xs, &ys, &ragged, &PlotOptions::default()).is_err());
        assert!(
            render_heatmap(
                &xs,
                &ys,
                &[vec![f64::NAN, 1.0], vec![2.0, 3.0]],
                &PlotOptions::default()
            )
            .is_err()
        );
    }

    #[test]
    fn box_plot_five_number_summary() {
        let s = render_box(
            &[BarSeries {
                label: "sample".to_string(),
                values: vec![5.0, 1.0, 3.0, 2.0, 4.0],
            }],
            &PlotOptions::default(),
        )
        .unwrap();
        // Whisker lines + caps + median line, quartile box, label.
        assert!(s.matches("<line").count() >= 5);
        assert!(s.contains("fill-opacity=\"0.28\""));
        assert!(s.contains(">sample<"));
        assert!(
            render_box(
                &[BarSeries {
                    label: "empty".to_string(),
                    values: vec![],
                }],
                &PlotOptions::default()
            )
            .is_err()
        );
    }

    #[test]
    fn interactive_marks_carry_their_datum() {
        let on = PlotOptions {
            interactive: true,
            ..Default::default()
        };
        let sc = render_xy(
            &[XySeries {
                label: "obs".to_string(),
                xs: vec![1.0, 2.0],
                ys: vec![3.0, 4.5],
                kind: XyKind::Scatter,
                point_colors: vec![],
            }],
            &on,
        )
        .unwrap();
        assert!(sc.contains("data-s=\"obs\""));
        assert!(sc.contains("data-x=\"2\"") && sc.contains("data-y=\"4.5\""));

        let bars = render_bars(&["a<b".to_string()], &[7.0], &on).unwrap();
        // Labels are escaped inside attributes, like everywhere else.
        assert!(bars.contains("data-label=\"a&lt;b\"") && bars.contains("data-value=\"7\""));

        let hm = render_heatmap(&["m".to_string()], &["r".to_string()], &[vec![2.0]], &on).unwrap();
        assert!(hm.contains("data-xl=\"m\"") && hm.contains("data-yl=\"r\""));

        let bx = render_box(
            &[BarSeries {
                label: "d".to_string(),
                values: vec![1.0, 2.0, 3.0],
            }],
            &on,
        )
        .unwrap();
        assert!(bx.contains("data-med=\"2\"") && bx.contains("data-q3=\"2.5\""));

        // Off by default: no data attributes anywhere.
        let off = render_bars(&["a".to_string()], &[1.0], &PlotOptions::default()).unwrap();
        assert!(!off.contains("data-"));
    }

    #[test]
    fn quantiles_interpolate() {
        let sorted = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(quantile(&sorted, 0.0), 1.0);
        assert_eq!(quantile(&sorted, 0.5), 2.5);
        assert_eq!(quantile(&sorted, 1.0), 4.0);
        assert_eq!(quantile(&[7.0], 0.5), 7.0);
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
