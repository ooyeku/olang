//! Text in the world: a text set by `gui`'s text system (its fonts,
//! shaping and glyphs) and drawn once into a picture, then drawn as a
//! sprite like any other. A text is set at the size it shows on screen,
//! rounded up to a step of a fifth of an octave, so it stays sharp at any
//! zoom without a picture for every size. Pictures are kept by what they
//! show; the oldest go when there are many.

use crate::stdlib::gui::canvas::{Picture, next_picture_id};
use crate::stdlib::gui::soft;
use crate::stdlib::gui::text::{self, Align, Font};
use crate::stdlib::gui::window::{Clip, DisplayList, push_glyphs};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Pictures kept at most.
const KEPT: usize = 512;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    text: String,
    family: String,
    weight: u32,
    italic: bool,
    px: u32,
    align: u8,
}

type Cache = HashMap<Key, (Arc<Picture>, u64)>;

fn cache() -> &'static Mutex<(Cache, u64)> {
    static C: OnceLock<Mutex<(Cache, u64)>> = OnceLock::new();
    C.get_or_init(|| Mutex::new((HashMap::new(), 0)))
}

/// The size a text is set at for `px` pixels on screen: the next step up
/// of a fifth of an octave, within 4 and 512.
pub fn step_px(px: f32) -> f32 {
    let px = px.clamp(4.0, 512.0);
    (2f32).powf((px.log2() * 5.0).ceil() / 5.0).min(512.0)
}

/// `text` set in `family` at `px` pixels (white, premultiplied, with a
/// margin of a quarter of the size), and the margin.
pub fn picture(
    text: &str,
    family: &str,
    weight: f32,
    italic: bool,
    px: f32,
    align: Align,
) -> Option<(Arc<Picture>, f32)> {
    let key = Key {
        text: text.to_string(),
        family: family.to_string(),
        weight: weight.round() as u32,
        italic,
        px: px.round() as u32,
        align: align as u8,
    };
    let pad = (px * 0.25).ceil();
    {
        let mut c = cache().lock().ok()?;
        c.1 += 1;
        let now = c.1;
        if let Some(hit) = c.0.get_mut(&key) {
            hit.1 = now;
            return Some((hit.0.clone(), pad));
        }
    }
    let font = Font {
        family: family.to_string(),
        size: px,
        weight,
        italic,
        ..Default::default()
    };
    let shaped = {
        let mut ts = text::system().lock().ok()?;
        ts.shape(text, &font, [255, 255, 255, 255], None, align, 1.0)
    };
    let w = (shaped.width + pad * 2.0).ceil().max(1.0) as u32;
    let h = (shaped.height + pad * 2.0).ceil().max(1.0) as u32;
    let mut dl = DisplayList {
        width: w,
        height: h,
        clear: [0, 0, 0, 0],
        prims: Vec::new(),
        scrolls: Vec::new(),
        flash: None,
        redraw: true,
        see_through: false,
        wide: None,
    };
    push_glyphs(
        &shaped,
        pad,
        pad,
        Clip::rect(0.0, 0.0, w as f32, h as f32),
        &mut dl,
    );
    let pm = soft::render(&dl);
    let pic = Arc::new(Picture {
        id: next_picture_id(),
        width: w,
        height: h,
        rgba: pm.data().to_vec(),
    });
    let mut c = cache().lock().ok()?;
    if c.0.len() >= KEPT {
        // the older half goes
        let mut ages: Vec<u64> = c.0.values().map(|v| v.1).collect();
        ages.sort_unstable();
        let cut = ages[ages.len() / 2];
        c.0.retain(|_, v| v.1 > cut);
    }
    let now = c.1;
    c.0.insert(key, (pic.clone(), now));
    Some((pic, pad))
}
