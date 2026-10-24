//! Wide-gamut colours: Display P3 colours past sRGB.
//!
//! A colour is `[u8; 4]` sRGB everywhere (`text::Color`). A wide colour
//! is one given in Display P3 (`"color(display-p3 r g b)"`, or with
//! `/ a`): it is its sRGB fallback (the colour clipped to sRGB) wherever
//! colours are sRGB — the software renderer, a headless window, a screen
//! that is not wide — and on the GPU, where the window's frame is kept in
//! extended sRGB (Rgba16Float, values past 0…1), the colour itself.
//!
//! The GPU learns which colours are wide from a table of sRGB fallbacks:
//! a colour drawn whose red, green and blue are a fallback in the table is
//! drawn as its wide colour (at the drawn colour's alpha, so a token at an
//! opacity stays wide). The table is a window's own (`gui.set`'s `wide`,
//! a theme's P3 tokens keyed by their sRGB ones) and every colour given in
//! Display P3 anywhere (keyed by its clipped fallback).

use super::text::Color;
use super::values::Res;
use crate::ast::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// sRGB fallbacks (red, green, blue) and the wide colours they stand for,
/// in extended sRGB (encoded, past 0…1 where the colour is past sRGB).
pub type WideMap = HashMap<[u8; 3], [f32; 3]>;

/// sRGB's transfer curve, odd (extended sRGB carries the sign).
fn decode(c: f32) -> f32 {
    let a = c.abs();
    let l = if a <= 0.04045 {
        a / 12.92
    } else {
        ((a + 0.055) / 1.055).powf(2.4)
    };
    l.copysign(c)
}

fn encode(l: f32) -> f32 {
    let a = l.abs();
    let c = if a <= 0.003_130_8 {
        a * 12.92
    } else {
        1.055 * a.powf(1.0 / 2.4) - 0.055
    };
    c.copysign(l)
}

/// A Display P3 colour (encoded, 0…1: P3 shares sRGB's curve) in extended
/// sRGB (encoded).
pub fn p3_to_srgb(p: [f32; 3]) -> [f32; 3] {
    let l = p.map(decode);
    let s = [
        1.224_940_2 * l[0] - 0.224_940_4 * l[1],
        -0.042_056_9 * l[0] + 1.042_057_1 * l[1],
        -0.019_637_6 * l[0] - 0.078_636_1 * l[1] + 1.098_273_5 * l[2],
    ];
    s.map(encode)
}

/// An extended sRGB colour clipped to sRGB, as bytes.
pub fn clip(e: [f32; 3]) -> [u8; 3] {
    e.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// `color(display-p3 r g b)` or `color(display-p3 r g b / a)` (each a
/// number 0…1 or a percentage; CSS's form): its sRGB fallback (clipped),
/// with alpha, and the colour in extended sRGB.
pub fn parse(t: &str) -> Option<(Color, [f32; 3])> {
    let body = t.trim().strip_prefix("color(")?.strip_suffix(')')?.trim();
    let rest = body.strip_prefix("display-p3")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let (chans, alpha) = match rest.split_once('/') {
        Some((c, a)) => (c, Some(a)),
        None => (rest, None),
    };
    let num = |w: &str| -> Option<f32> {
        let v = match w.strip_suffix('%') {
            Some(p) => p.parse::<f32>().ok()? / 100.0,
            None => w.parse::<f32>().ok()?,
        };
        v.is_finite().then_some(v.clamp(0.0, 1.0))
    };
    let ws: Vec<&str> = chans.split_whitespace().collect();
    let [r, g, b] = ws.as_slice() else {
        return None;
    };
    let p = [num(r)?, num(g)?, num(b)?];
    let a = match alpha {
        Some(a) => {
            let ws: Vec<&str> = a.split_whitespace().collect();
            let [a] = ws.as_slice() else {
                return None;
            };
            num(a)?
        }
        None => 1.0,
    };
    let e = p3_to_srgb(p);
    let f = clip(e);
    Some(([f[0], f[1], f[2], (a * 255.0).round() as u8], e))
}

/// Every colour given in Display P3 so far (its fallback and itself), and
/// a count that moves as it grows.
static SEEN: RwLock<(u64, Option<Arc<WideMap>>)> = RwLock::new((0, None));

/// The most colours [`SEEN`] keeps (a program making colours without end
/// is drawn in their fallbacks past it).
const SEEN_MAX: usize = 4096;

/// A colour given in Display P3: known to the GPU by its fallback.
pub fn note(fallback: Color, e: [f32; 3]) {
    let key = [fallback[0], fallback[1], fallback[2]];
    // a colour inside sRGB is its fallback: nothing to draw differently
    if e.iter().all(|c| (0.0..=1.0).contains(c)) {
        return;
    }
    if let Ok(r) = SEEN.read()
        && r.1.as_ref().is_some_and(|m| m.get(&key) == Some(&e))
    {
        return;
    }
    let Ok(mut w) = SEEN.write() else {
        return;
    };
    let mut m = w.1.as_deref().cloned().unwrap_or_default();
    if m.len() >= SEEN_MAX {
        return;
    }
    m.insert(key, e);
    w.0 += 1;
    w.1 = Some(Arc::new(m));
}

/// The colours given in Display P3, and their count's mark.
pub fn seen() -> (u64, Option<Arc<WideMap>>) {
    SEEN.read().map(|r| r.clone()).unwrap_or((0, None))
}

/// A window's table (`gui.set`'s `wide`): a map from sRGB colours
/// (`"#rrggbb"`) to the colours they stand for on a wide screen (P3).
pub fn parse_table(v: &Value, what: &str) -> Res<WideMap> {
    let mut out = WideMap::new();
    let Value::Map(m) = v else {
        return Err(format!(
            "{what}: \"wide\" is a map of sRGB colours to display-p3 ones"
        ));
    };
    for (k, c) in m.iter() {
        let from = super::values::parse_color(&Value::String(Arc::new(k.clone())))
            .ok_or_else(|| format!("{what}: \"wide\": {k:?} is not a colour"))?;
        let to = match c {
            Value::String(t) => parse(t),
            _ => None,
        }
        .ok_or_else(|| {
            format!("{what}: \"wide\": {k:?} must name a colour as color(display-p3 r g b)")
        })?;
        out.insert([from[0], from[1], from[2]], to.1);
    }
    Ok(out)
}

/// `c` premultiplied, its red, green and blue the wide colour `wide` has
/// for them (extended sRGB) when it has one.
pub fn premul(c: Color, wide: Option<&WideMap>) -> [f32; 4] {
    let a = c[3] as f32 / 255.0;
    let rgb = wide
        .and_then(|m| m.get(&[c[0], c[1], c[2]]))
        .copied()
        .unwrap_or([
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
        ]);
    [rgb[0] * a, rgb[1] * a, rgb[2] * a, a]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_p3_is_parsed_clipped_and_kept_wide() {
        // P3's red is past sRGB's: clipped to sRGB's red, itself beyond it
        let (f, e) = parse("color(display-p3 1 0 0)").unwrap();
        assert_eq!(f, [255, 0, 0, 255]);
        assert!(e[0] > 1.05 && e[1] < 0.0 && e[2] < 0.0, "{e:?}");
        // a grey is itself in both
        let (f, e) = parse("color(display-p3 0.5 0.5 0.5 / 50%)").unwrap();
        assert!(
            f[..3].iter().all(|c| c.abs_diff(128) <= 1) && f[3] == 128,
            "{f:?}"
        );
        assert!(e.iter().all(|c| (c - 0.5).abs() < 1e-3), "{e:?}");
        // sRGB's green in P3 is sRGB's green again
        let (f, _) = parse("color(display-p3 0.4584 0.9853 0.2983)").unwrap();
        assert!(f[0] <= 1 && f[1] >= 254 && f[2] <= 1, "{f:?}");
        for bad in [
            "color(display-p3 1 0)",
            "color(srgb 1 0 0)",
            "color(display-p31 0 0)",
            "color(display-p3 1 0 0 / )",
            "color(display-p3 a b c)",
            "display-p3 1 0 0",
        ] {
            assert!(parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn premultiplied_wide_where_the_table_says() {
        let mut m = WideMap::new();
        m.insert([255, 0, 0], [1.1, -0.2, -0.1]);
        let p = premul([255, 0, 0, 128], Some(&m));
        assert!(p[0] > 0.5 && p[1] < 0.0);
        assert_eq!(premul([0, 0, 255, 255], Some(&m)), [0.0, 0.0, 1.0, 1.0]);
    }
}
