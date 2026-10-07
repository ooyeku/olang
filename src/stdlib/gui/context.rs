//! The system's settings a window answers without the program branching
//! on them (Loom's SPEC §11): the dark appearance, increased contrast,
//! reduced motion, reduced transparency, and the accent colour. Read
//! where the platform says (macOS); elsewhere the window's own theme
//! answers dark, and the rest are off.
//!
//! **Live.** While the event loop runs it keeps the settings ([`live`]),
//! read on its (the main) thread from AppKit — `NSApp.effectiveAppearance`
//! and NSWorkspace's accessibility display options — and read again when
//! AppKit says one changed (`platform`'s observers:
//! `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification`,
//! `NSSystemColorsDidChangeNotification`, and a window's theme changing).
//! Each window then hears an `appearance` event, and its own copy
//! ([`WinState::settings`](super::window::WinState)) changes with it (a
//! picture stops animating under reduced motion).

use super::values::*;
use crate::ast::Value;
use std::sync::Mutex;

/// The system's settings at one moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Settings {
    /// The dark appearance; `None` when only a window can tell.
    pub dark: Option<bool>,
    pub contrast: bool,
    pub reduce_motion: bool,
    pub reduce_transparency: bool,
    /// The accent colour the person chose (sRGB), when the system says.
    pub accent: Option<[u8; 3]>,
}

/// The settings the event loop keeps, once it runs.
static LIVE: Mutex<Option<Settings>> = Mutex::new(None);

/// The settings as the event loop last read them, or `None` before it runs.
pub fn live() -> Option<Settings> {
    *LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Keep `s` as the settings now: answers whether they changed.
pub fn set_live(s: Settings) -> bool {
    let mut l = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let changed = *l != Some(s);
    *l = Some(s);
    changed
}

/// What the system asks for now, read from the platform. On the main
/// thread (the event loop's) the appearance is the application's
/// effective one, which follows Auto; on another thread it is the
/// `AppleInterfaceStyle` default.
pub fn read() -> Settings {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSColor, NSColorSpace, NSWorkspace};
        use objc2_foundation::{NSUserDefaults, ns_string};
        let ws = NSWorkspace::sharedWorkspace();
        let dark = effective_dark().unwrap_or_else(|| {
            NSUserDefaults::standardUserDefaults()
                .stringForKey(ns_string!("AppleInterfaceStyle"))
                .map(|s| s.to_string().eq_ignore_ascii_case("dark"))
                .unwrap_or(false)
        });
        let accent = NSColor::controlAccentColor()
            .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
            .map(|c| {
                let ch = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                [ch(c.redComponent()), ch(c.greenComponent()), ch(c.blueComponent())]
            });
        Settings {
            dark: Some(dark),
            contrast: ws.accessibilityDisplayShouldIncreaseContrast(),
            reduce_motion: ws.accessibilityDisplayShouldReduceMotion(),
            reduce_transparency: ws.accessibilityDisplayShouldReduceTransparency(),
            accent,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Settings::default()
    }
}

/// The application's effective appearance, dark or not — on the main
/// thread only (`None` elsewhere).
#[cfg(target_os = "macos")]
fn effective_dark() -> Option<bool> {
    use objc2_app_kit::{NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication};
    use objc2_foundation::{MainThreadMarker, NSArray};
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let names = unsafe { NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]) };
    let best = app.effectiveAppearance().bestMatchFromAppearancesWithNames(&names)?;
    Some(&*best == unsafe { NSAppearanceNameDarkAqua })
}

/// The settings now: the event loop's while it runs, else read here.
pub fn system() -> Settings {
    live().unwrap_or_else(read)
}

/// `#rrggbb`.
pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn fields(st: Settings, window_dark: Option<bool>) -> Vec<(&'static str, Value)> {
    vec![
        (
            "dark",
            Value::Boolean(st.dark.or(window_dark).unwrap_or(false)),
        ),
        ("contrast", Value::Boolean(st.contrast)),
        ("reduce_motion", Value::Boolean(st.reduce_motion)),
        ("reduce_transparency", Value::Boolean(st.reduce_transparency)),
        (
            "accent",
            match st.accent {
                Some(c) => s(&hex(c)),
                None => Value::Unit,
            },
        ),
    ]
}

/// The settings as a map: `#{ dark, contrast, reduce_motion,
/// reduce_transparency, accent }` (`accent` `"#rrggbb"` or `()`).
pub fn as_value(window_dark: Option<bool>) -> Value {
    map(fields(system(), window_dark))
}

/// The `appearance` event for window `id`: settings `s` (dark as the
/// window's theme says when the system cannot: a window whose appearance
/// the program chose is not the system's), and `why` it was sent (`"open"`, `"focus"`,
/// `"theme"`, `"settings"`, `"test"`).
pub fn event_of(id: u64, st: Settings, window_dark: Option<bool>, why: &str) -> Value {
    let mut f = vec![("window", Value::Integer(id as i64))];
    f.extend(fields(st, window_dark));
    f.push(("why", s(why)));
    event("appearance", f)
}

/// The `appearance` event for window `id` with the settings now.
pub fn event_for(id: u64, window_dark: Option<bool>, why: &str) -> Value {
    event_of(id, system(), window_dark, why)
}

/// A colour `#rrggbb` (or `#rgb`) as its channels.
pub fn parse_hex(t: &str) -> Option<[u8; 3]> {
    let h = t.strip_prefix('#')?;
    let v = |a: &str| u8::from_str_radix(a, 16).ok();
    match h.len() {
        6 => Some([v(&h[0..2])?, v(&h[2..4])?, v(&h[4..6])?]),
        3 => {
            let d = |i: usize| v(&h[i..i + 1]).map(|x| x * 17);
            Some([d(0)?, d(1)?, d(2)?])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_accent_is_spelled_and_read_back() {
        assert_eq!(hex([0, 122, 255]), "#007aff");
        assert_eq!(parse_hex("#007aff"), Some([0, 122, 255]));
        assert_eq!(parse_hex("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("blue"), None);
    }
}
