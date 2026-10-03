//! The system's settings a window answers without the program branching
//! on them (Loom's SPEC §11): the dark appearance, increased contrast,
//! and reduced motion. Read where the platform says; elsewhere the
//! window's own theme answers dark, and the other two are false.

use super::values::*;
use crate::ast::Value;

/// What the system asks for now: `(dark, contrast, reduce_motion)`, with
/// `dark` `None` when only a window can tell.
pub fn system() -> (Option<bool>, bool, bool) {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSUserDefaults, ns_string};
        let ws = NSWorkspace::sharedWorkspace();
        let contrast = ws.accessibilityDisplayShouldIncreaseContrast();
        let motion = ws.accessibilityDisplayShouldReduceMotion();
        let style =
            NSUserDefaults::standardUserDefaults().stringForKey(ns_string!("AppleInterfaceStyle"));
        let dark = style
            .map(|s| s.to_string().eq_ignore_ascii_case("dark"))
            .unwrap_or(false);
        (Some(dark), contrast, motion)
    }
    #[cfg(not(target_os = "macos"))]
    {
        (None, false, false)
    }
}

/// The settings as a map: `#{ dark, contrast, reduce_motion }`, `dark`
/// from `window_dark` when the system cannot say.
pub fn as_value(window_dark: Option<bool>) -> Value {
    let (dark, contrast, motion) = system();
    map(vec![
        (
            "dark",
            Value::Boolean(dark.or(window_dark).unwrap_or(false)),
        ),
        ("contrast", Value::Boolean(contrast)),
        ("reduce_motion", Value::Boolean(motion)),
    ])
}

/// The `appearance` event for window `id`: the settings now.
pub fn event_for(id: u64, window_dark: Option<bool>) -> Value {
    let (dark, contrast, motion) = system();
    event(
        "appearance",
        vec![
            ("window", Value::Integer(id as i64)),
            (
                "dark",
                Value::Boolean(dark.or(window_dark).unwrap_or(false)),
            ),
            ("contrast", Value::Boolean(contrast)),
            ("reduce_motion", Value::Boolean(motion)),
        ],
    )
}
