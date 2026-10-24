//! Motion (loom/SPEC.md §7): a node's `transition` — its opacity, its
//! box, its colours, and how it comes and goes — run by the engine on
//! the window's clock at the display's refresh, with no program turn;
//! and the curves motion runs on (cubic Béziers and springs).
//!
//! A node declares it in its style:
//!
//!   "style": #{ "transition": #{ "duration": 160, "curve": "ease_out",
//!       "animate": ["opacity", "box", "color"],
//!       "enter": #{ "opacity": 0, "scale": 0.97, "y": -4 },
//!       "exit":  #{ "opacity": 0 } } }
//!
//! - `animate`: what moves when a frame changes it — `opacity` (the
//!   style's), `box` (its place and size in its parent), `color` (its
//!   background, border and text colours), `hover` (its colours only as
//!   the pointer comes and goes: a hover look fades, a selection or a
//!   press lands at once). Opacity, box and colour when it is not said.
//! - `enter`: the pose it comes from when a patch adds it (an opacity, a
//!   move `x`/`y` in logical pixels, a `scale` about its centre); `exit`
//!   the pose it leaves to when a patch removes it — drawn by the engine
//!   from what it last showed, under nothing, said to nobody.
//! - `duration` (ms, 200), `delay` (ms, 0), `curve`: `ease` (the
//!   default), `ease_in`, `ease_out`, `ease_in_out`, `linear`, or
//!   `spring` (`duration` its response, `bounce` 0 … 0.9: a spring
//!   retargeted mid-flight keeps its velocity).
//!
//! Reduced motion makes every transition instant, as does a headless
//! window unless a test asks for motion (`gui.set(w, #{ motion: true })`):
//! a test's frames do not wait on a clock it does not move.
//!
//! The window keeps a [`Record`] a node with a transition (window.rs
//! paints by what it says) and a [`Ghost`] a node leaving. Damage needs
//! nothing of its own: a moving node's tiles change, and only they are
//! drawn again (damage.rs).

use super::scene::Scene;
use super::text::Color;
use super::values::*;
use crate::ast::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// What a transition animates when a frame changes it.
pub const OPACITY: u8 = 1;
pub const BOX: u8 = 2;
pub const COLOR: u8 = 4;
/// Colours ease only as the pointer comes and goes (a hover look fades; a
/// selection or a press lands at once).
pub const HOVER: u8 = 8;

/// How a value travels from where it was to where it goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Linear,
    /// CSS's `cubic-bezier(x1, y1, x2, y2)`.
    Bezier([f32; 4]),
    /// A damped spring: `omega` its angular frequency (per second),
    /// `zeta` its damping ratio (1: no overshoot).
    Spring {
        omega: f32,
        zeta: f32,
    },
}

impl Curve {
    /// The share of the way travelled at `t` of `1` (a Bézier, linear).
    pub fn ease(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match *self {
            Curve::Linear | Curve::Spring { .. } => t,
            Curve::Bezier([x1, y1, x2, y2]) => bezier(x1, y1, x2, y2, t),
        }
    }
}

/// `cubic-bezier(x1, y1, x2, y2)` at `x`: the curve's `y` where its `x`
/// is `x` (Newton's steps, then halving where they fail).
fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let (x1, y1, x2, y2, x) = (x1 as f64, y1 as f64, x2 as f64, y2 as f64, x as f64);
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;
    let sx = |t: f64| ((ax * t + bx) * t + cx) * t;
    let dx = |t: f64| (3.0 * ax * t + 2.0 * bx) * t + cx;
    let mut t = x;
    let mut solved = false;
    for _ in 0..8 {
        let e = sx(t) - x;
        if e.abs() < 1e-6 {
            solved = true;
            break;
        }
        let d = dx(t);
        if d.abs() < 1e-6 {
            break;
        }
        t -= e / d;
    }
    if !solved {
        let (mut lo, mut hi) = (0.0, 1.0);
        t = x;
        for _ in 0..40 {
            let v = sx(t);
            if (v - x).abs() < 1e-6 {
                break;
            }
            if v < x {
                lo = t;
            } else {
                hi = t;
            }
            t = (lo + hi) / 2.0;
        }
    }
    (((ay * t + by) * t + cy) * t) as f32
}

/// Where a node is drawn relative to where it is: its opacity, a move
/// (logical pixels) and a scale about its centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub opacity: f32,
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

impl Pose {
    pub const REST: Pose = Pose {
        opacity: 1.0,
        x: 0.0,
        y: 0.0,
        scale: 1.0,
    };

    fn parse(v: &Value, what: &str) -> Res<Pose> {
        if fields(v).is_none() {
            return Err(format!("{what}: a pose is a map (opacity, x, y, scale)"));
        }
        for k in fields(v)
            .map(|f| f.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
        {
            if !["opacity", "x", "y", "scale"].contains(&k.as_str()) {
                return Err(format!(
                    "{what}: unknown \"{k}\" (a pose is opacity, x, y, scale)"
                ));
            }
        }
        Ok(Pose {
            opacity: get_num(v, "opacity", what)?.unwrap_or(1.0).clamp(0.0, 1.0),
            x: get_num(v, "x", what)?.unwrap_or(0.0),
            y: get_num(v, "y", what)?.unwrap_or(0.0),
            scale: get_num(v, "scale", what)?.unwrap_or(1.0).clamp(0.05, 20.0),
        })
    }

    fn vec(&self) -> Vec<f32> {
        vec![self.opacity, self.x, self.y, self.scale]
    }

    fn of(v: &[f32]) -> Pose {
        Pose {
            opacity: v[0].clamp(0.0, 1.0),
            x: v[1],
            y: v[2],
            scale: v[3].max(0.01),
        }
    }
}

/// A node's transition, as its style says it.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub ms: f32,
    pub delay: f32,
    pub curve: Curve,
    pub animate: u8,
    pub enter: Option<Pose>,
    pub exit: Option<Pose>,
}

const TRANSITION_KEYS: &[&str] = &[
    "duration", "delay", "curve", "bounce", "animate", "enter", "exit",
];

/// A style's `transition` map.
pub fn parse(v: &Value, what: &str) -> Res<Arc<Transition>> {
    let what = &format!("{what}: \"transition\"");
    let Some(f) = fields(v) else {
        return Err(format!("{what} must be a map"));
    };
    for k in f.keys() {
        if !TRANSITION_KEYS.contains(&k.as_str()) {
            return Err(format!(
                "{what}: unknown \"{k}\" (the keys are {})",
                TRANSITION_KEYS.join(", ")
            ));
        }
    }
    let ms = get_num(v, "duration", what)?
        .unwrap_or(200.0)
        .clamp(0.0, 10_000.0);
    let delay = get_num(v, "delay", what)?
        .unwrap_or(0.0)
        .clamp(0.0, 10_000.0);
    let bounce = get_num(v, "bounce", what)?.unwrap_or(0.0).clamp(0.0, 0.9);
    let curve = match get_str(v, "curve", what)?.unwrap_or("ease") {
        "ease" => Curve::Bezier([0.25, 0.1, 0.25, 1.0]),
        "ease_in" => Curve::Bezier([0.42, 0.0, 1.0, 1.0]),
        "ease_out" => Curve::Bezier([0.0, 0.0, 0.58, 1.0]),
        "ease_in_out" => Curve::Bezier([0.42, 0.0, 0.58, 1.0]),
        "linear" => Curve::Linear,
        // SwiftUI's `spring(duration:bounce:)`: the duration its response
        "spring" => Curve::Spring {
            omega: 2.0 * std::f32::consts::PI / (ms.max(1.0) / 1000.0),
            zeta: 1.0 - bounce,
        },
        other => {
            return Err(format!(
                "{what}: unknown curve \"{other}\" (ease, ease_in, ease_out, ease_in_out, linear, spring)"
            ));
        }
    };
    let animate = match get(v, "animate") {
        None => OPACITY | BOX | COLOR,
        Some(Value::String(s)) if s.as_str() == "all" => OPACITY | BOX | COLOR,
        Some(Value::String(s)) if s.as_str() == "none" => 0,
        Some(x) => {
            let items: &[Value] = match x {
                Value::List(l) => l,
                Value::Tuple(t) => t,
                _ => {
                    return Err(format!(
                        "{what}: \"animate\" is a list (opacity, box, color, hover)"
                    ));
                }
            };
            let mut bits = 0;
            for i in items {
                bits |= match i {
                    Value::String(s) if s.as_str() == "opacity" => OPACITY,
                    Value::String(s) if s.as_str() == "box" => BOX,
                    Value::String(s) if s.as_str() == "color" => COLOR,
                    Value::String(s) if s.as_str() == "hover" => HOVER,
                    _ => {
                        return Err(format!(
                            "{what}: \"animate\" names opacity, box, color, hover (got {})",
                            i.type_name()
                        ));
                    }
                };
            }
            bits
        }
    };
    let enter = get(v, "enter").map(|p| Pose::parse(p, what)).transpose()?;
    let exit = get(v, "exit").map(|p| Pose::parse(p, what)).transpose()?;
    Ok(Arc::new(Transition {
        ms,
        delay,
        curve,
        animate,
        enter,
        exit,
    }))
}

/// One value (of a few numbers) on its way: from where it was to where it
/// goes, since `start` on the window's clock (ms).
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub from: Vec<f32>,
    pub to: Vec<f32>,
    /// A spring's velocity at the start (units a second).
    pub vel: Vec<f32>,
    pub start: f64,
    pub ms: f32,
    pub curve: Curve,
    /// When a spring has settled (ms after the start).
    settle: f32,
}

impl Track {
    pub fn new(
        from: Vec<f32>,
        to: Vec<f32>,
        vel: Vec<f32>,
        start: f64,
        ms: f32,
        curve: Curve,
    ) -> Track {
        let settle = match curve {
            Curve::Spring { omega, zeta } => {
                // the envelope under a thousandth of the way, or of a
                // pixel's tenth: settled (at most four responses)
                let mut amp: f32 = 0.0;
                let mut span: f32 = 0.0;
                let wd = omega * (1.0 - zeta * zeta).max(0.0).sqrt();
                for i in 0..from.len() {
                    let x0 = from[i] - to[i];
                    let v0 = vel.get(i).copied().unwrap_or(0.0);
                    let b = if wd > 1e-3 {
                        (v0 + zeta * omega * x0) / wd
                    } else {
                        (v0 + omega * x0) / omega
                    };
                    amp = amp.max((x0 * x0 + b * b).sqrt());
                    span = span.max(x0.abs());
                }
                let eps = (span * 1e-3).max(1e-3);
                let rate = (zeta * omega).max(1e-3);
                let t = if amp <= eps {
                    0.0
                } else {
                    (amp / eps).ln() / rate
                };
                // critical damping's polynomial term needs a little more
                let t = if zeta >= 0.999 { t * 1.25 } else { t };
                let response = 2.0 * std::f32::consts::PI / omega.max(1e-3) * 1000.0;
                (t * 1000.0).min(4.0 * response)
            }
            _ => ms,
        };
        Track {
            from,
            to,
            vel,
            start,
            ms,
            curve,
            settle,
        }
    }

    /// The value at `now`, and whether it has arrived.
    pub fn at(&self, now: f64) -> (Vec<f32>, bool) {
        let t = (now - self.start) as f32;
        if t <= 0.0 {
            return (self.from.clone(), false);
        }
        match self.curve {
            Curve::Spring { omega, zeta } => {
                if t >= self.settle {
                    return (self.to.clone(), true);
                }
                let secs = t / 1000.0;
                let v = (0..self.from.len())
                    .map(|i| {
                        self.to[i]
                            + spring_x(
                                self.from[i] - self.to[i],
                                self.vel.get(i).copied().unwrap_or(0.0),
                                omega,
                                zeta,
                                secs,
                            )
                    })
                    .collect();
                (v, false)
            }
            c => {
                if t >= self.ms || self.ms <= 0.0 {
                    return (self.to.clone(), true);
                }
                let p = c.ease(t / self.ms);
                let v = (0..self.from.len())
                    .map(|i| self.from[i] + (self.to[i] - self.from[i]) * p)
                    .collect();
                (v, false)
            }
        }
    }

    /// The velocity at `now` (units a second): a spring retargeted keeps it.
    pub fn velocity(&self, now: f64) -> Vec<f32> {
        let (a, _) = self.at(now - 1.0);
        let (b, done) = self.at(now);
        if done {
            return vec![0.0; b.len()];
        }
        a.iter().zip(&b).map(|(a, b)| (b - a) * 1000.0).collect()
    }
}

/// A damped spring's displacement at `t` seconds, from `x0` with velocity
/// `v0`.
fn spring_x(x0: f32, v0: f32, omega: f32, zeta: f32, t: f32) -> f32 {
    if zeta < 0.999 {
        let wd = omega * (1.0 - zeta * zeta).sqrt();
        let e = (-zeta * omega * t).exp();
        e * (x0 * (wd * t).cos() + (v0 + zeta * omega * x0) / wd * (wd * t).sin())
    } else {
        (-omega * t).exp() * (x0 + (v0 + omega * x0) * t)
    }
}

/// A channel of a node: what the frame last asked for, and the track
/// that takes it there.
#[derive(Clone, Debug, Default)]
struct Chan {
    target: Vec<f32>,
    track: Option<Track>,
}

impl Chan {
    fn new(target: Vec<f32>) -> Chan {
        Chan {
            target,
            track: None,
        }
    }

    /// The value now, the frame asking for `target`: a change starts a
    /// track from where it shows (`animate`) or lands at once. `None`:
    /// at the target.
    fn drive(
        &mut self,
        target: &[f32],
        tr: &Transition,
        animate: bool,
        now: f64,
    ) -> Option<Vec<f32>> {
        if self.target.as_slice() != target {
            if animate {
                let (from, vel) = match &self.track {
                    Some(t) => (t.at(now).0, t.velocity(now)),
                    None => (self.target.clone(), vec![0.0; target.len()]),
                };
                // a spring keeps its velocity; a curve starts afresh
                let vel = if matches!(tr.curve, Curve::Spring { .. }) && vel.len() == target.len() {
                    vel
                } else {
                    vec![0.0; target.len()]
                };
                self.track = (from.len() == target.len()).then(|| {
                    Track::new(
                        from,
                        target.to_vec(),
                        vel,
                        now + tr.delay as f64,
                        tr.ms,
                        tr.curve,
                    )
                });
            } else {
                self.track = None;
            }
            self.target = target.to_vec();
        }
        let t = self.track.as_ref()?;
        let (v, done) = t.at(now);
        if done {
            self.track = None;
            return None;
        }
        Some(v)
    }

    fn moving(&self) -> bool {
        self.track.is_some()
    }
}

/// A node with a transition, as the window last drew it.
#[derive(Clone, Debug, Default)]
pub struct Record {
    seen: u64,
    rect: Chan,
    opacity: Chan,
    colors: Chan,
    /// Its entrance: from the `enter` pose to rest.
    pose: Option<Track>,
    hovered: bool,
}

/// What to draw a node with a transition as, now.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub rect: [f32; 4],
    pub opacity: f32,
    pub bg: Option<Color>,
    pub border: Option<Color>,
    pub color: Color,
    pub pose: Pose,
}

/// What a node is asked to be this frame.
pub struct Target {
    pub rect: [f32; 4],
    pub opacity: f32,
    pub bg: Option<Color>,
    pub border: Option<Color>,
    pub color: Color,
    /// The pointer is over it (its hover look is in the colours).
    pub hovered: bool,
}

/// A node that left with an `exit`: the subtree as it last was, drawn
/// from `rect` (logical pixels in the window) to the exit pose.
pub struct Ghost {
    pub key: String,
    pub scene: Scene,
    pub start: f64,
    pub track: Track,
}

impl Ghost {
    /// The pose now, and whether it has gone.
    pub fn pose(&self, now: f64) -> (Pose, bool) {
        let (v, done) = self.track.at(now);
        (Pose::of(&v), done)
    }
}

/// A window's motion: the nodes with transitions, the nodes a patch added
/// since the last frame (an `enter` plays only for them), the ghosts.
#[derive(Default)]
pub struct Motion {
    /// Transitions run (a real window; a headless one when a test asks):
    /// else, as under reduced motion, every one is instant.
    pub enabled: bool,
    pub records: HashMap<String, Record>,
    pub fresh: HashSet<String>,
    pub ghosts: Vec<Ghost>,
    /// Frames drawn (the first enters nothing: a window opens as it is).
    pub frame: u64,
    /// Something moved in the frame being drawn.
    pub active: bool,
    /// The ghosts are being drawn: the transitions in them are not run.
    pub ghosting: bool,
    /// The window's wide colours (wide.rs): a colour moving to or from
    /// one moves through wide colours, not through sRGB (a hover fade on
    /// a P3 accent stays vivid all the way).
    pub wide: Option<std::sync::Arc<super::wide::WideMap>>,
    /// The wide colours the frame's moving colours are at (each one's
    /// sRGB fallback, and itself in extended sRGB), for the frame's table.
    pub wide_moving: Vec<([u8; 3], [f32; 3])>,
}

/// A colour premultiplied (a fade from none to a colour passes through no
/// black), as four numbers; a wide colour (in `wide`) as itself, its
/// channels past 0…255.
fn premul(c: Option<Color>, like: Option<Color>, wide: Option<&super::wide::WideMap>) -> [f32; 4] {
    match c.or(like.map(|l| [l[0], l[1], l[2], 0])) {
        Some(c) => {
            let a = c[3] as f32 / 255.0;
            let rgb = wide
                .and_then(|m| m.get(&[c[0], c[1], c[2]]))
                .map(|e| e.map(|v| v * 255.0))
                .unwrap_or([c[0] as f32, c[1] as f32, c[2] as f32]);
            [rgb[0] * a, rgb[1] * a, rgb[2] * a, c[3] as f32]
        }
        None => [0.0; 4],
    }
}

/// A moving colour past sRGB (premultiplied, 0…255 scale): its fallback
/// and itself in extended sRGB, else `None`.
fn wide_at(v: &[f32]) -> Option<([u8; 3], [f32; 3])> {
    if v[3] < 0.5 {
        return None;
    }
    let e = [v[0] / v[3], v[1] / v[3], v[2] / v[3]];
    if e.iter().all(|c| (0.0..=1.0).contains(c)) {
        return None;
    }
    let c = unpremul(v);
    Some(([c[0], c[1], c[2]], e))
}

fn unpremul(v: &[f32]) -> Color {
    let a = v[3].clamp(0.0, 255.0);
    if a < 0.5 {
        return [0, 0, 0, 0];
    }
    let k = 255.0 / a;
    [
        (v[0] * k).round().clamp(0.0, 255.0) as u8,
        (v[1] * k).round().clamp(0.0, 255.0) as u8,
        (v[2] * k).round().clamp(0.0, 255.0) as u8,
        a.round() as u8,
    ]
}

impl Motion {
    /// Transitions run in this frame (not reduced, not a still test).
    pub fn runs(&self, reduce_motion: bool) -> bool {
        self.enabled && !reduce_motion
    }

    /// A frame begins at `now`.
    pub fn begin(&mut self) {
        self.frame += 1;
        self.active = false;
    }

    /// The frame is drawn: records of nodes it did not draw go (they come
    /// back as they were, never entering), as do the frame's new nodes.
    pub fn end(&mut self) {
        let f = self.frame;
        if !self.records.is_empty() {
            self.records.retain(|_, r| r.seen == f);
        }
        self.fresh.clear();
    }

    /// Node `key` with transition `tr`, asked to be `target`: what it
    /// shows now.
    pub fn drive(
        &mut self,
        key: &str,
        tr: &Transition,
        target: &Target,
        now: f64,
        runs: bool,
    ) -> Shown {
        let frame = self.frame;
        let fresh = self.fresh.contains(key);
        let wide = self.wide.clone();
        let wide = wide.as_deref();
        let colors_of = |t: &Target| -> Vec<f32> {
            let mut v = Vec::with_capacity(12);
            v.extend(premul(t.bg, Some(t.color), wide));
            v.extend(premul(t.border, t.bg.or(Some(t.color)), wide));
            v.extend(premul(Some(t.color), None, wide));
            v
        };
        let colors = colors_of(target);
        let rec = self.records.entry(key.to_string()).or_insert_with(|| {
            let mut r = Record {
                seen: frame,
                rect: Chan::new(target.rect.to_vec()),
                opacity: Chan::new(vec![target.opacity]),
                colors: Chan::new(colors.clone()),
                pose: None,
                hovered: target.hovered,
            };
            // added by a patch since the last frame, into a window already
            // shown: it comes in from its `enter` pose
            if fresh
                && runs
                && frame > 1
                && let Some(p) = tr.enter
            {
                r.pose = Some(Track::new(
                    p.vec(),
                    Pose::REST.vec(),
                    vec![0.0; 4],
                    now + tr.delay as f64,
                    tr.ms,
                    tr.curve,
                ));
            }
            r
        });
        rec.seen = frame;
        let hover_moved = rec.hovered != target.hovered;
        rec.hovered = target.hovered;
        let rect = rec
            .rect
            .drive(&target.rect, tr, runs && tr.animate & BOX != 0, now);
        let opacity = rec
            .opacity
            .drive(
                &[target.opacity],
                tr,
                runs && tr.animate & OPACITY != 0,
                now,
            )
            .map_or(target.opacity, |v| v[0]);
        let color_runs = tr.animate & COLOR != 0 || (tr.animate & HOVER != 0 && hover_moved);
        let cv = rec.colors.drive(&colors, tr, runs && color_runs, now);
        let pose = match &rec.pose {
            Some(t) if runs => {
                let (v, done) = t.at(now);
                if done {
                    rec.pose = None;
                }
                Pose::of(&v)
            }
            _ => {
                rec.pose = None;
                Pose::REST
            }
        };
        if rec.rect.moving() || rec.opacity.moving() || rec.colors.moving() || rec.pose.is_some() {
            self.active = true;
        }
        let rect = rect.map_or(target.rect, |r| [r[0], r[1], r[2], r[3]]);
        let (bg, border, color) = match &cv {
            Some(cv) => {
                if wide.is_some() {
                    for i in 0..3 {
                        if let Some(w) = wide_at(&cv[i * 4..i * 4 + 4]) {
                            self.wide_moving.push(w);
                        }
                    }
                }
                let colored = |i: usize, was: Option<Color>| -> Option<Color> {
                    let c = unpremul(&cv[i * 4..i * 4 + 4]);
                    if was.is_none() && c[3] == 0 {
                        None
                    } else {
                        Some(c)
                    }
                };
                (
                    colored(0, target.bg),
                    colored(1, target.border).or(target.border),
                    unpremul(&cv[8..12]),
                )
            }
            None => (target.bg, target.border, target.color),
        };
        Shown {
            rect,
            opacity: opacity.clamp(0.0, 1.0),
            bg,
            border,
            color,
            pose,
        }
    }

    /// A node with an `exit` left: its subtree goes on being drawn, from
    /// rest to that pose.
    pub fn leave(&mut self, key: String, scene: Scene, tr: &Transition, now: f64) {
        let Some(exit) = tr.exit else { return };
        self.ghosts.retain(|g| g.key != key);
        self.records.remove(&key);
        let track = Track::new(
            Pose::REST.vec(),
            exit.vec(),
            vec![0.0; 4],
            now + tr.delay as f64,
            tr.ms,
            tr.curve,
        );
        self.ghosts.push(Ghost {
            key,
            scene,
            start: now,
            track,
        });
    }

    /// Anything still moving after the frame being drawn.
    pub fn moving(&self) -> bool {
        self.active || !self.ghosts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(curve: &str, ms: f32) -> Transition {
        let v = map(vec![
            ("curve", s(curve)),
            ("duration", Value::Float(ms as f64)),
        ]);
        (*parse(&v, "t").unwrap()).clone()
    }

    fn target(x: f32, opacity: f32, bg: Option<Color>) -> Target {
        Target {
            rect: [x, 0.0, 100.0, 20.0],
            opacity,
            bg,
            border: None,
            color: [0, 0, 0, 255],
            hovered: false,
        }
    }

    #[test]
    fn the_curves_start_at_zero_end_at_one_and_ease() {
        for c in ["ease", "ease_in", "ease_out", "ease_in_out", "linear"] {
            let t = tr(c, 100.0);
            assert!(t.curve.ease(0.0).abs() < 1e-4, "{c}");
            assert!((t.curve.ease(1.0) - 1.0).abs() < 1e-4, "{c}");
        }
        // CSS's ease at its middle is about 0.80; ease_in lags, ease_out leads
        assert!((tr("ease", 1.0).curve.ease(0.5) - 0.8024).abs() < 0.01);
        assert!(tr("ease_in", 1.0).curve.ease(0.5) < 0.5);
        assert!(tr("ease_out", 1.0).curve.ease(0.5) > 0.5);
        assert_eq!(tr("linear", 1.0).curve.ease(0.25), 0.25);
    }

    #[test]
    fn a_spring_arrives_in_about_its_duration_and_a_bouncy_one_overshoots() {
        let t = Track::new(
            vec![0.0],
            vec![100.0],
            vec![0.0],
            0.0,
            300.0,
            tr("spring", 300.0).curve,
        );
        let (mid, _) = t.at(150.0);
        assert!(mid[0] > 50.0 && mid[0] < 100.0, "{mid:?}");
        let (end, done) = t.at(t.settle as f64 + 1.0);
        assert!(done && end[0] == 100.0);
        assert!(t.settle < 500.0, "{}", t.settle);
        // never past its target when it does not bounce
        assert!((0..300).all(|ms| t.at(ms as f64).0[0] <= 100.0001));
        let v = map(vec![
            ("curve", s("spring")),
            ("duration", Value::Integer(300)),
            ("bounce", Value::Float(0.4)),
        ]);
        let b = parse(&v, "t").unwrap();
        let t = Track::new(vec![0.0], vec![100.0], vec![0.0], 0.0, 300.0, b.curve);
        assert!((0..600).any(|ms| t.at(ms as f64).0[0] > 101.0));
    }

    #[test]
    fn a_spring_retargeted_keeps_its_velocity_a_curve_starts_from_where_it_shows() {
        let mut m = Motion {
            enabled: true,
            ..Default::default()
        };
        let t = tr("spring", 300.0);
        m.begin();
        m.drive("a", &t, &target(0.0, 1.0, None), 0.0, true);
        m.end();
        m.begin();
        m.drive("a", &t, &target(100.0, 1.0, None), 0.0, true);
        m.end();
        m.begin();
        let at = m.drive("a", &t, &target(100.0, 1.0, None), 60.0, true).rect[0];
        m.end();
        // turned back: it goes on forward a moment (its velocity) first
        m.begin();
        let next = m.drive("a", &t, &target(0.0, 1.0, None), 61.0, true).rect[0];
        m.end();
        m.begin();
        let later = m.drive("a", &t, &target(0.0, 1.0, None), 70.0, true).rect[0];
        m.end();
        assert!(next >= at - 0.01 && later > at, "{at} {next} {later}");
        // a curve: from where it showed, no jump
        let mut m = Motion {
            enabled: true,
            ..Default::default()
        };
        let t = tr("linear", 100.0);
        for (now, x) in [(0.0, 0.0), (0.0, 100.0)] {
            m.begin();
            m.drive("a", &t, &target(x, 1.0, None), now, true);
            m.end();
        }
        m.begin();
        assert_eq!(
            m.drive("a", &t, &target(100.0, 1.0, None), 50.0, true).rect[0],
            50.0
        );
        m.end();
        m.begin();
        let sh = m.drive("a", &t, &target(0.0, 1.0, None), 50.0, true);
        assert_eq!(sh.rect[0], 50.0);
        assert!(m.active);
        m.end();
        m.begin();
        assert_eq!(
            m.drive("a", &t, &target(0.0, 1.0, None), 100.0, true).rect[0],
            25.0
        );
        m.end();
    }

    #[test]
    fn instant_when_motion_does_not_run_and_only_what_it_animates_moves() {
        let mut m = Motion::default();
        let t = tr("linear", 100.0);
        m.begin();
        m.drive("a", &t, &target(0.0, 1.0, None), 0.0, false);
        m.end();
        m.begin();
        let sh = m.drive("a", &t, &target(80.0, 0.5, None), 10.0, false);
        assert_eq!((sh.rect[0], sh.opacity), (80.0, 0.5));
        assert!(!m.moving());
        m.end();
        // opacity only: the box lands at once
        let v = map(vec![
            ("curve", s("linear")),
            ("duration", Value::Integer(100)),
            ("animate", Value::List(Arc::new(vec![s("opacity")]))),
        ]);
        let t = parse(&v, "t").unwrap();
        let mut m = Motion::default();
        for (now, x, o) in [(0.0, 0.0, 0.0), (0.0, 50.0, 1.0)] {
            m.begin();
            m.drive("a", &t, &target(x, o, None), now, true);
            m.end();
        }
        m.begin();
        let sh = m.drive("a", &t, &target(50.0, 1.0, None), 50.0, true);
        assert_eq!(sh.rect[0], 50.0);
        assert!((sh.opacity - 0.5).abs() < 1e-4);
        m.end();
    }

    #[test]
    fn a_colour_fades_in_from_none_without_passing_through_black() {
        let mut m = Motion::default();
        let t = tr("linear", 100.0);
        let red = Some([200, 40, 40, 255]);
        for (now, bg) in [(0.0, None), (0.0, red)] {
            m.begin();
            m.drive("a", &t, &target(0.0, 1.0, bg), now, true);
            m.end();
        }
        m.begin();
        let sh = m.drive("a", &t, &target(0.0, 1.0, red), 50.0, true);
        let bg = sh.bg.unwrap();
        assert_eq!(&bg[..3], &[200, 40, 40]);
        assert!((bg[3] as i32 - 128).abs() <= 1, "{bg:?}");
        m.end();
    }

    #[test]
    fn a_node_enters_only_when_a_patch_added_it_to_a_window_already_shown() {
        let v = map(vec![
            ("curve", s("linear")),
            ("duration", Value::Integer(100)),
            (
                "enter",
                map(vec![
                    ("opacity", Value::Integer(0)),
                    ("scale", Value::Float(0.5)),
                ]),
            ),
        ]);
        let t = parse(&v, "t").unwrap();
        let mut m = Motion::default();
        // the first frame: as it is
        m.fresh.insert("a".into());
        m.begin();
        assert_eq!(
            m.drive("a", &t, &target(0.0, 1.0, None), 0.0, true).pose,
            Pose::REST
        );
        m.end();
        // later, added again: from the pose
        m.records.clear();
        m.fresh.insert("a".into());
        m.begin();
        let p = m.drive("a", &t, &target(0.0, 1.0, None), 0.0, true).pose;
        assert_eq!((p.opacity, p.scale), (0.0, 0.5));
        m.end();
        m.begin();
        let p = m.drive("a", &t, &target(0.0, 1.0, None), 50.0, true).pose;
        assert_eq!((p.opacity, p.scale), (0.5, 0.75));
        m.end();
        // drawn again without a patch adding it (scrolled back in): still
        m.records.clear();
        m.begin();
        assert_eq!(
            m.drive("a", &t, &target(0.0, 1.0, None), 60.0, true).pose,
            Pose::REST
        );
        m.end();
    }

    #[test]
    fn a_transition_says_what_is_wrong_with_it() {
        let bad = |v: Value| parse(&v, "t").unwrap_err();
        assert!(bad(map(vec![("curve", s("bouncy"))])).contains("unknown curve"));
        assert!(bad(map(vec![("speed", Value::Integer(1))])).contains("unknown \"speed\""));
        assert!(
            bad(map(vec![(
                "enter",
                map(vec![("rotate", Value::Integer(1))])
            )]))
            .contains("rotate")
        );
        assert!(
            bad(map(vec![(
                "animate",
                Value::List(Arc::new(vec![s("size")]))
            )]))
            .contains("animate")
        );
        assert!(bad(s("fast")).contains("must be a map"));
    }
}
