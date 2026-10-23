//! Scroll physics as AppKit's: a trackpad's gesture pulls a scroller past
//! its ends against a rubber band and lets go with a spring; its momentum
//! (the system's, after the fingers lift) carries on to an end and
//! bounces there; a mouse wheel's notches glide rather than jump.
//!
//! The window keeps an [`Elastic`] a scroller (by key) while it is
//! stretched, springing back or gliding: the stretch is drawn (the
//! content moved past its end; the program's offset stays inside it and
//! hears nothing of it), a glide moves the offset itself (each step said
//! as the wheel's are). Pure: logical pixels and the window's clock (ms).

/// Where a scroll event is in its gesture (NSEvent's `phase` and
/// `momentumPhase`). `Legacy`: none said (a test's, another platform's):
/// the offset moves at once, with no stretch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Legacy,
    Began,
    Changed,
    Ended,
    MomentumBegan,
    Momentum,
    MomentumEnded,
}

impl Phase {
    pub fn parse(s: &str) -> Option<Phase> {
        Some(match s {
            "began" => Phase::Began,
            "changed" => Phase::Changed,
            "ended" => Phase::Ended,
            "momentum_began" => Phase::MomentumBegan,
            "momentum" => Phase::Momentum,
            "momentum_ended" => Phase::MomentumEnded,
            "none" | "legacy" => Phase::Legacy,
            _ => return None,
        })
    }

    pub fn momentum(self) -> bool {
        matches!(
            self,
            Phase::MomentumBegan | Phase::Momentum | Phase::MomentumEnded
        )
    }

    pub fn gesture(self) -> bool {
        matches!(self, Phase::Began | Phase::Changed | Phase::Ended)
    }
}

/// UIKit's and AppKit's rubber band: a pull of `pull` past the end of a
/// view `dim` long shows as this much (it never reaches `dim`).
pub fn rubber(pull: f32, dim: f32) -> f32 {
    if dim <= 0.0 || pull == 0.0 {
        return 0.0;
    }
    let c = 0.55;
    pull.signum() * (1.0 - 1.0 / (pull.abs() * c / dim + 1.0)) * dim
}

/// The pull that shows as `shown` (the rubber band's inverse).
pub fn unrubber(shown: f32, dim: f32) -> f32 {
    if dim <= 0.0 || shown == 0.0 {
        return 0.0;
    }
    let c = 0.55;
    let r = (shown.abs() / dim).min(0.999);
    shown.signum() * (1.0 / (1.0 - r) - 1.0) * dim / c
}

/// The spring a stretch goes back on: critically damped, a response of
/// 400 ms (what NSScrollView's bounce looks like side by side).
const BACK_OMEGA: f32 = 2.0 * std::f32::consts::PI / 0.4;
/// A notch's glide (ms), eased out; notches in a row extend it.
pub const GLIDE_MS: f32 = 160.0;
/// The fastest a momentum may arrive at an end (px a second): its bounce
/// is never wild.
const MAX_BOUNCE_V: f32 = 3000.0;

/// A stretch going back: from `from` (shown, px) with velocity `vel`
/// (px/s), since `start`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Back {
    pub from: [f32; 2],
    pub vel: [f32; 2],
    pub start: f64,
}

/// A wheel's glide: the offset from `from` to `to` since `start`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glide {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub start: f64,
}

impl Glide {
    /// The offset at `now`, and whether it has arrived.
    pub fn at(&self, now: f64) -> ([f32; 2], bool) {
        let t = ((now - self.start) as f32 / GLIDE_MS).clamp(0.0, 1.0);
        if t >= 1.0 {
            return (self.to, true);
        }
        // ease out (cubic)
        let p = 1.0 - (1.0 - t).powi(3);
        (
            [
                self.from[0] + (self.to[0] - self.from[0]) * p,
                self.from[1] + (self.to[1] - self.from[1]) * p,
            ],
            false,
        )
    }
}

/// One scroller's elastic state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Elastic {
    /// The pull past its ends (logical px; positive past the start: the
    /// content shows moved right or down).
    pub pull: [f32; 2],
    pub back: Option<Back>,
    /// The fingers are on the trackpad.
    pub gesture: bool,
    /// The momentum hit an end (or the fingers lifted while stretched):
    /// the rest of it is not this scroller's.
    pub bounced: bool,
    /// When the last event came (the window's clock).
    pub last_at: f64,
    pub glide: Option<Glide>,
}

fn spring(x0: f32, v0: f32, t: f32) -> (f32, f32) {
    let w = BACK_OMEGA;
    let e = (-w * t).exp();
    let b = v0 + w * x0;
    let x = e * (x0 + b * t);
    let v = e * (b - w * (x0 + b * t));
    (x, v)
}

impl Elastic {
    /// How far its content shows moved past its ends now (px), for a view
    /// `dims` (width, height); and whether it is still moving.
    pub fn shown(&self, now: f64, dims: [f32; 2]) -> ([f32; 2], bool) {
        match self.back {
            Some(b) => {
                let t = ((now - b.start) as f32 / 1000.0).max(0.0);
                let mut out = [0.0; 2];
                let mut moving = false;
                for (o, (&from, &vel)) in out.iter_mut().zip(b.from.iter().zip(&b.vel)) {
                    let (x, v) = spring(from, vel, t);
                    // settled under a tenth of a pixel
                    if x.abs() > 0.1 || v.abs() > 2.0 {
                        moving = true;
                        *o = x;
                    }
                }
                (out, moving)
            }
            None => (
                [rubber(self.pull[0], dims[0]), rubber(self.pull[1], dims[1])],
                false,
            ),
        }
    }

    /// Fingers back on the trackpad: a stretch going back is held where it
    /// shows (a pull again).
    pub fn take_hold(&mut self, now: f64, dims: [f32; 2]) {
        if self.back.is_some() {
            let (s, _) = self.shown(now, dims);
            self.pull = [unrubber(s[0], dims[0]), unrubber(s[1], dims[1])];
            self.back = None;
        }
        self.gesture = true;
        self.bounced = false;
    }

    /// A delta `d` along axis `i` while pulled: what of it goes to the
    /// pull (back toward the content first), and what is left to scroll.
    pub fn relax(&mut self, i: usize, d: f32) -> f32 {
        let p = self.pull[i];
        if p == 0.0 || d == 0.0 || d.signum() == p.signum() {
            return d;
        }
        let np = p + d;
        if np.signum() == p.signum() {
            self.pull[i] = np;
            0.0
        } else {
            self.pull[i] = 0.0;
            np
        }
    }

    /// Let go: the stretch goes back from where it shows, at `vel` (px/s).
    pub fn release(&mut self, now: f64, dims: [f32; 2], vel: [f32; 2]) {
        let (s, _) = self.shown(now, dims);
        if s == [0.0, 0.0] && vel == [0.0, 0.0] {
            self.pull = [0.0; 2];
            self.back = None;
            return;
        }
        let v = vel.map(|v| v.clamp(-MAX_BOUNCE_V, MAX_BOUNCE_V));
        self.back = Some(Back {
            from: s,
            vel: v,
            start: now,
        });
        self.pull = [0.0; 2];
    }

    /// Nothing to draw or do: the window may let it go.
    pub fn idle(&self, now: f64, dims: [f32; 2]) -> bool {
        self.glide.is_none()
            && !self.gesture
            && self.pull == [0.0, 0.0]
            && self.back.is_none_or(|_| !self.shown(now, dims).1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rubber_band_resists_more_the_further_it_is_pulled_and_never_reaches_the_view() {
        let d = 500.0;
        assert_eq!(rubber(0.0, d), 0.0);
        let a = rubber(50.0, d);
        let b = rubber(100.0, d);
        assert!(a < 50.0 && b < 100.0 && b > a);
        assert!(b - a < a, "each pull shows less than the one before");
        assert!(rubber(100_000.0, d) < d);
        assert_eq!(rubber(-50.0, d), -a);
        for p in [3.0, 40.0, 300.0, -120.0] {
            assert!((unrubber(rubber(p, d), d) - p).abs() < 0.01 * p.abs().max(1.0));
        }
    }

    #[test]
    fn a_pull_relaxes_before_the_content_moves() {
        let mut e = Elastic {
            pull: [0.0, 30.0],
            ..Default::default()
        };
        assert_eq!(e.relax(1, 10.0), 10.0, "further out is the pull's caller's");
        assert_eq!(e.relax(1, -20.0), 0.0);
        assert_eq!(e.pull[1], 10.0);
        assert_eq!(e.relax(1, -25.0), -15.0, "past it: the rest scrolls");
        assert_eq!(e.pull[1], 0.0);
    }

    #[test]
    fn let_go_it_springs_back_without_passing_the_end_and_settles() {
        let dims = [300.0, 400.0];
        let mut e = Elastic {
            pull: [0.0, 120.0],
            ..Default::default()
        };
        let (s0, _) = e.shown(0.0, dims);
        e.release(0.0, dims, [0.0, 0.0]);
        let mut last = s0[1];
        for ms in (0..1200).step_by(8) {
            let (s, _) = e.shown(ms as f64, dims);
            assert!(
                s[1] <= last + 1e-3 && s[1] >= -1e-3,
                "{ms}: {} after {last}",
                s[1]
            );
            last = s[1];
        }
        assert_eq!(e.shown(1200.0, dims), ([0.0, 0.0], false));
        assert!(e.idle(1200.0, dims));
    }

    #[test]
    fn momentum_at_an_end_bounces_out_and_comes_back() {
        let dims = [300.0, 400.0];
        let mut e = Elastic::default();
        // arriving at 1500 px/s past the bottom (content moving up: negative)
        e.release(0.0, dims, [0.0, -1500.0]);
        let peak = (0..600)
            .step_by(4)
            .map(|ms| e.shown(ms as f64, dims).0[1])
            .fold(0.0f32, f32::min);
        assert!(peak < -20.0 && peak > -120.0, "overshoot {peak}");
        assert!(!e.shown(1500.0, dims).1);
    }

    #[test]
    fn a_glide_eases_out_to_its_end() {
        let g = Glide {
            from: [0.0, 0.0],
            to: [0.0, 96.0],
            start: 100.0,
        };
        assert_eq!(g.at(100.0).0[1], 0.0);
        let (mid, done) = g.at(100.0 + GLIDE_MS as f64 / 2.0);
        assert!(
            !done && mid[1] > 48.0,
            "ahead of linear at half: {}",
            mid[1]
        );
        assert_eq!(g.at(100.0 + GLIDE_MS as f64), ([0.0, 96.0], true));
    }

    #[test]
    fn phases_read_as_the_platform_says_them() {
        assert_eq!(Phase::parse("began"), Some(Phase::Began));
        assert_eq!(Phase::parse("momentum"), Some(Phase::Momentum));
        assert!(Phase::Momentum.momentum() && !Phase::Changed.momentum());
        assert_eq!(Phase::parse("fling"), None);
    }
}
