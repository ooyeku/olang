//! Overlay scroll bars, as the Mac draws them: a thin thumb over the
//! right edge of anything that scrolls further than it shows (a scroll
//! region, a styled field), shown while it scrolls and for a moment
//! after, faded out, and shown again while the pointer is over its
//! edge — then wider, on a quiet track. The thumb is dragged; a press
//! on the track pages towards the pointer. A code field may put marks
//! on the track (its problems, a find's matches) and the caret's line
//! is marked on it: those show whether or not the thumb does.
//!
//! The window keeps a [`Bar`] a scroller (by key) and, each frame, the
//! [`Hit`]s it drew, so the pointer finds a bar before the content under
//! it. The geometry here is pure (logical pixels in, logical pixels
//! out) and tested on its own.

use super::text::Color;

/// How long a bar stays after it last moved (ms), then how long it
/// takes to fade.
pub const SHOW_MS: f64 = 900.0;
pub const FADE_MS: f64 = 250.0;
/// The thumb's width at rest and under the pointer, its least length,
/// its inset from the edge (logical pixels).
pub const THIN: f32 = 6.0;
pub const WIDE: f32 = 10.0;
pub const MIN_THUMB: f32 = 24.0;
pub const INSET: f32 = 2.0;
/// The edge the pointer shows a bar in (logical pixels from the right).
pub const ZONE: f32 = 14.0;

/// One scroller's bar: when it last moved (the window's clock, ms),
/// whether the pointer is over its edge, and a drag (where the thumb
/// was taken, logical pixels from its top).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bar {
    pub moved_at: Option<f64>,
    pub hover: bool,
    pub drag: Option<f32>,
}

/// A bar as the last frame drew it, logical pixels in the window: the
/// zone the pointer finds it in, its track, the view's length, the
/// whole length, the offset, the most it scrolls, and whether it is a
/// styled field's (whose offset is the field's own).
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub key: String,
    pub zone: [f32; 4],
    pub track: (f32, f32),
    pub view: f32,
    pub total: f32,
    pub off: f32,
    pub max: f32,
    pub rich: bool,
}

impl Hit {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.zone[0] && x < self.zone[2] && y >= self.zone[1] && y < self.zone[3]
    }

    /// The thumb: its top and length on the track.
    pub fn thumb(&self) -> (f32, f32) {
        thumb(
            self.track.0,
            self.track.1,
            self.view,
            self.total,
            self.off,
            self.max,
        )
    }

    /// The offset that puts the thumb's top at `top`.
    pub fn offset_for(&self, top: f32) -> f32 {
        let (_, h) = self.thumb();
        let room = (self.track.1 - h).max(1.0);
        ((top - self.track.0) / room).clamp(0.0, 1.0) * self.max
    }

    /// A press on the track at `y` (not on the thumb): a page towards it.
    pub fn page_toward(&self, y: f32) -> f32 {
        let (t, h) = self.thumb();
        let page = (self.view * 0.9).max(1.0);
        if y < t {
            (self.off - page).max(0.0)
        } else if y >= t + h {
            (self.off + page).min(self.max)
        } else {
            self.off
        }
    }
}

/// The thumb on a track from `y0`, `len` long, for a view of `view` in
/// `total` scrolled `off` of `max`: its top and its length (at least
/// `MIN_THUMB`, never longer than the track).
pub fn thumb(y0: f32, len: f32, view: f32, total: f32, off: f32, max: f32) -> (f32, f32) {
    let whole = total.max(view + max).max(1.0);
    let h = (len * view / whole).max(MIN_THUMB).min(len);
    let frac = if max > 0.0 {
        (off / max).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (y0 + (len - h) * frac, h)
}

/// How much of a bar shows now (0 to 1): whole while the pointer is over
/// it or drags it, and for `SHOW_MS` after it moved, then fading.
pub fn alpha(b: &Bar, now: f64, reduce_motion: bool) -> f32 {
    if b.hover || b.drag.is_some() {
        return 1.0;
    }
    let Some(t) = b.moved_at else { return 0.0 };
    let age = now - t;
    if age < SHOW_MS {
        1.0
    } else if reduce_motion || age >= SHOW_MS + FADE_MS {
        0.0
    } else {
        (1.0 - (age - SHOW_MS) / FADE_MS) as f32
    }
}

/// When a bar showing now must be drawn again (its fade's next step, or
/// its end), on the window's clock; `None` when it is still.
pub fn due(b: &Bar, now: f64) -> Option<f64> {
    if b.hover || b.drag.is_some() {
        return None;
    }
    let t = b.moved_at?;
    let age = now - t;
    if age < SHOW_MS {
        Some(t + SHOW_MS)
    } else if age < SHOW_MS + FADE_MS {
        Some(now + 16.0)
    } else {
        None
    }
}

/// The thumb's colour over a background: black over a light one, white
/// over a dark one, at `a` of its strength.
pub fn ink(bg: Color, a: f32) -> Color {
    let lum = 0.2126 * bg[0] as f32 + 0.7152 * bg[1] as f32 + 0.0722 * bg[2] as f32;
    let v = if lum < 128.0 { 255 } else { 0 };
    [v, v, v, (255.0 * a).round().clamp(0.0, 255.0) as u8]
}

/// A mark on the track: where on it a line `y` of `total` falls.
pub fn mark_y(y0: f32, len: f32, y: f32, total: f32) -> f32 {
    if total <= 0.0 {
        y0
    } else {
        y0 + len * (y / total).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(off: f32) -> Hit {
        Hit {
            key: "s".into(),
            zone: [86.0, 0.0, 100.0, 100.0],
            track: (2.0, 96.0),
            view: 100.0,
            total: 400.0,
            off,
            max: 300.0,
            rich: false,
        }
    }

    #[test]
    fn the_thumb_is_the_view_s_share_of_the_track_and_moves_with_the_offset() {
        assert_eq!(thumb(0.0, 100.0, 100.0, 400.0, 0.0, 300.0), (0.0, 25.0));
        assert_eq!(thumb(0.0, 100.0, 100.0, 400.0, 300.0, 300.0), (75.0, 25.0));
        // a long text: the thumb keeps a length a pointer can take
        assert_eq!(
            thumb(0.0, 100.0, 100.0, 100_000.0, 0.0, 99_900.0).1,
            MIN_THUMB
        );
        // nothing to scroll: the whole track
        assert_eq!(thumb(0.0, 100.0, 100.0, 100.0, 0.0, 0.0), (0.0, 100.0));
    }

    #[test]
    fn a_drag_puts_the_thumb_s_top_where_the_pointer_takes_it() {
        let h = hit(0.0);
        assert_eq!(h.offset_for(2.0), 0.0);
        assert_eq!(h.offset_for(2.0 + 72.0), 300.0);
        assert_eq!(h.offset_for(500.0), 300.0);
        assert_eq!(h.offset_for(-5.0), 0.0);
    }

    #[test]
    fn a_press_on_the_track_pages_towards_it() {
        let h = hit(100.0);
        let (t, len) = h.thumb();
        assert_eq!(h.page_toward(t + len + 5.0), 190.0);
        assert_eq!(h.page_toward(t - 5.0), 10.0);
        assert_eq!(h.page_toward(t + 1.0), 100.0);
        assert_eq!(hit(250.0).page_toward(99.0), 300.0);
    }

    #[test]
    fn a_bar_shows_while_it_moves_and_under_the_pointer_then_fades() {
        let mut b = Bar::default();
        assert_eq!(alpha(&b, 0.0, false), 0.0);
        b.moved_at = Some(1000.0);
        assert_eq!(alpha(&b, 1500.0, false), 1.0);
        assert_eq!(due(&b, 1500.0), Some(1900.0));
        let mid = alpha(&b, 1000.0 + SHOW_MS + FADE_MS / 2.0, false);
        assert!(mid > 0.4 && mid < 0.6);
        assert_eq!(alpha(&b, 1000.0 + SHOW_MS + 1.0, true), 0.0);
        assert_eq!(alpha(&b, 5000.0, false), 0.0);
        assert_eq!(due(&b, 5000.0), None);
        b.hover = true;
        assert_eq!(alpha(&b, 5000.0, false), 1.0);
    }

    #[test]
    fn the_ink_stands_against_its_background() {
        assert_eq!(ink([255, 255, 255, 255], 0.5)[0], 0);
        assert_eq!(ink([30, 30, 32, 255], 0.5)[0], 255);
        assert_eq!(ink([30, 30, 32, 255], 0.5)[3], 128);
        assert_eq!(mark_y(0.0, 100.0, 50.0, 200.0), 25.0);
    }
}
