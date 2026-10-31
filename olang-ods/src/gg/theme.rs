//! Themes: what the frame of a plot looks like. The default, `minimal`,
//! is Studio's quiet instrument: no panel fill, hairline gridlines at the
//! major breaks, no ticks or axis lines, one type scale, ink for words
//! and a second ink for everything that only orients. Every frame colour
//! is a role (`Paint::Role`) so a host draws it in its own tokens and an
//! SVG resolves it here, for the light, dark or high-contrast appearance.

use super::scene::{Rgba, Role};
use super::{Params, p_num, p_str};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    Light,
    Dark,
    Contrast,
}

impl Appearance {
    pub fn parse(s: &str) -> Option<Appearance> {
        Some(match s {
            "light" => Appearance::Light,
            "dark" => Appearance::Dark,
            "contrast" | "high-contrast" | "high_contrast" => Appearance::Contrast,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
            Appearance::Contrast => "contrast",
        }
    }

    /// A role's colour in this appearance.
    pub fn role(self, r: Role) -> Rgba {
        let hex = match (self, r) {
            (Appearance::Light, Role::Surface) => "#fcfcfb",
            (Appearance::Light, Role::Ink) => "#18181b",
            (Appearance::Light, Role::Ink2) => "#5d5c58",
            (Appearance::Light, Role::Grid) => "#e6e5e1",
            (Appearance::Light, Role::Axis) => "#bdbcb6",
            (Appearance::Light, Role::Strip) => "#f1f0ec",
            (Appearance::Light, Role::Panel) => "#f4f3ef",
            (Appearance::Dark, Role::Surface) => "#16181d",
            (Appearance::Dark, Role::Ink) => "#e6e8ec",
            (Appearance::Dark, Role::Ink2) => "#9aa1ad",
            (Appearance::Dark, Role::Grid) => "#262a32",
            (Appearance::Dark, Role::Axis) => "#4a505c",
            (Appearance::Dark, Role::Strip) => "#1f2228",
            (Appearance::Dark, Role::Panel) => "#1c1f25",
            (Appearance::Contrast, Role::Surface) => "#ffffff",
            (Appearance::Contrast, Role::Ink) => "#000000",
            (Appearance::Contrast, Role::Ink2) => "#1a1a1a",
            (Appearance::Contrast, Role::Grid) => "#8c8c8c",
            (Appearance::Contrast, Role::Axis) => "#000000",
            (Appearance::Contrast, Role::Strip) => "#e8e8e8",
            (Appearance::Contrast, Role::Panel) => "#f2f2f2",
        };
        Rgba::hex(hex).unwrap()
    }

    pub fn dark(self) -> bool {
        self == Appearance::Dark
    }
}

/// The categorical palette ("quiet"): eight hues of one lightness band,
/// assigned in a fixed order, each readable on the surface, tuned per
/// appearance (lighter on the dark surface, deeper in high contrast).
pub fn quiet(app: Appearance) -> &'static [&'static str] {
    match app {
        Appearance::Light => &[
            "#3a6fd8", "#e2783a", "#25a27e", "#c64f8e", "#8c6be0", "#c99a1e", "#4f8f3a", "#d24b4b",
            "#2b98b5", "#8c7765",
        ],
        Appearance::Dark => &[
            "#6c9cf2", "#f2955e", "#4cc6a0", "#e57ab0", "#a68ef0", "#e3b847", "#80bd62", "#ee7676",
            "#5cc0dc", "#b3a08f",
        ],
        Appearance::Contrast => &[
            "#0b4fc9", "#c2500f", "#007a55", "#a3216b", "#5a3cc0", "#8a6400", "#2f6d1c", "#b01e1e",
            "#006f8c", "#5e4a3a",
        ],
    }
}

/// A theme: the style and its sizes.
#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub app: Appearance,
    pub base: f64,
    pub font: String,
    /// "right", "bottom", "top", "left" or "none".
    pub legend: String,
    /// Gridlines: "both", "x", "y", "none".
    pub grid: String,
    pub panel_fill: bool,
    pub panel_border: bool,
    pub axis_lines: bool,
    pub ticks: bool,
    pub strip_fill: bool,
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// The background as any CSS colour (`"transparent"` lets the page
    /// show through); None: the appearance's surface.
    pub paper: Option<String>,
    /// Every word, tick, gridline and axis line in one CSS colour
    /// (`"currentColor"` takes the page's text colour), the lines at a
    /// reduced opacity; None: the appearance's inks.
    pub ink: Option<String>,
    /// The prefix of every id the SVG names (its clip paths); None: one
    /// derived from the SVG's content.
    pub id: Option<String>,
}

impl Theme {
    pub fn of(p: &Params, default_app: Appearance) -> Theme {
        let name = p_str(p, "name").unwrap_or("minimal").to_string();
        let mut app = p_str(p, "appearance")
            .and_then(Appearance::parse)
            .unwrap_or(default_app);
        match name.as_str() {
            "dark" => app = Appearance::Dark,
            "light" => app = Appearance::Light,
            "contrast" => app = Appearance::Contrast,
            _ => {}
        }
        let (panel_fill, panel_border, axis_lines, ticks, strip_fill, grid) = match name.as_str() {
            "classic" => (false, false, true, true, false, "none"),
            "bw" => (false, true, false, true, true, "both"),
            "gray" | "grey" => (true, false, false, true, true, "both"),
            "void" => (false, false, false, false, false, "none"),
            "linedraw" => (false, true, false, true, false, "both"),
            _ => (false, false, false, false, false, "both"),
        };
        Theme {
            name,
            app,
            base: p_num(p, "base_size").unwrap_or(11.0).clamp(6.0, 32.0),
            font: p_str(p, "font")
                .unwrap_or("system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif")
                .to_string(),
            legend: p_str(p, "legend_position").unwrap_or("right").to_string(),
            grid: p_str(p, "grid").unwrap_or(grid).to_string(),
            panel_fill,
            panel_border,
            axis_lines,
            ticks,
            strip_fill,
            width: p_num(p, "width"),
            height: p_num(p, "height"),
            paper: p_str(p, "paper").map(str::to_string),
            ink: p_str(p, "ink").map(str::to_string),
            id: p_str(p, "id").map(str::to_string),
        }
    }

    // ── the one type scale ──────────────────────────────────────────
    pub fn tick(&self) -> f64 {
        (self.base * 0.91 * 10.0).round() / 10.0
    }
    pub fn axis_title(&self) -> f64 {
        self.base
    }
    pub fn legend_text(&self) -> f64 {
        self.tick()
    }
    pub fn legend_title(&self) -> f64 {
        self.base
    }
    pub fn strip(&self) -> f64 {
        self.base
    }
    pub fn title(&self) -> f64 {
        (self.base * 1.36 * 10.0).round() / 10.0
    }
    pub fn subtitle(&self) -> f64 {
        self.base
    }
    pub fn caption(&self) -> f64 {
        self.tick()
    }

    pub fn void(&self) -> bool {
        self.name == "void"
    }

    pub fn grid_x(&self) -> bool {
        self.grid == "both" || self.grid == "x"
    }
    pub fn grid_y(&self) -> bool {
        self.grid == "both" || self.grid == "y"
    }
}
