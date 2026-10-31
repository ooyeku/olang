//! Gamepads, as Toad's input reads them: buttons are held, pressed and
//! released like keys (`"pad:a"`, `"pad:start"`, `"pad:up"`…, gui's key
//! record), sticks and triggers are numbers. Every connected pad counts
//! as one (the first to move a stick is read first), which is what a
//! single-player game wants; per-pad reads come with local multiplayer.
//! Polled once a step on the olang thread (gilrs is not shared).

use std::cell::RefCell;

thread_local! {
    static PADS: RefCell<Option<Option<gilrs::Gilrs>>> = const { RefCell::new(None) };
}

fn name(b: gilrs::Button) -> Option<&'static str> {
    use gilrs::Button::*;
    Some(match b {
        South => "pad:a",
        East => "pad:b",
        West => "pad:x",
        North => "pad:y",
        LeftTrigger => "pad:lb",
        RightTrigger => "pad:rb",
        LeftTrigger2 => "pad:lt",
        RightTrigger2 => "pad:rt",
        Select => "pad:select",
        Start => "pad:start",
        Mode => "pad:home",
        LeftThumb => "pad:ls",
        RightThumb => "pad:rs",
        DPadUp => "pad:up",
        DPadDown => "pad:down",
        DPadLeft => "pad:left",
        DPadRight => "pad:right",
        _ => return None,
    })
}

/// A stick's reading with its dead zone taken out (radial, rescaled so
/// just past the dead zone is just past zero).
fn dead(x: f32, y: f32) -> (f32, f32) {
    const DEAD: f32 = 0.18;
    let m = (x * x + y * y).sqrt();
    if m < DEAD {
        return (0.0, 0.0);
    }
    let k = ((m - DEAD) / (1.0 - DEAD)).min(1.0) / m;
    (x * k, y * k)
}

/// Poll the pads: their buttons into the key record, and the sticks and
/// triggers now (`left`, `right` pairs; `lt`, `rt` 0 to 1) and how many
/// pads are connected.
pub fn poll() -> ((f32, f32), (f32, f32), f32, f32, usize) {
    PADS.with(|p| {
        let mut p = p.borrow_mut();
        let g = p.get_or_insert_with(|| gilrs::Gilrs::new().ok());
        let Some(g) = g.as_mut() else {
            return ((0.0, 0.0), (0.0, 0.0), 0.0, 0.0, 0);
        };
        while let Some(ev) = g.next_event() {
            match ev.event {
                gilrs::EventType::ButtonPressed(b, _) => {
                    if let Some(n) = name(b) {
                        crate::stdlib::gui::keys::device_press(n);
                    }
                }
                gilrs::EventType::ButtonReleased(b, _) => {
                    if let Some(n) = name(b) {
                        crate::stdlib::gui::keys::device_release(n);
                    }
                }
                gilrs::EventType::Disconnected => crate::stdlib::gui::keys::device_release_all(),
                _ => {}
            }
        }
        let mut best = ((0.0, 0.0), (0.0, 0.0), 0.0f32, 0.0f32);
        let mut n = 0;
        for (_, pad) in g.gamepads() {
            n += 1;
            let ax = |a| pad.axis_data(a).map(|d| d.value()).unwrap_or(0.0);
            let bt = |b| pad.button_data(b).map(|d| d.value()).unwrap_or(0.0);
            let l = dead(ax(gilrs::Axis::LeftStickX), ax(gilrs::Axis::LeftStickY));
            let r = dead(ax(gilrs::Axis::RightStickX), ax(gilrs::Axis::RightStickY));
            let lt = bt(gilrs::Button::LeftTrigger2);
            let rt = bt(gilrs::Button::RightTrigger2);
            let mag = |v: (f32, f32)| v.0.abs() + v.1.abs();
            if mag(l) + mag(r) + lt + rt > mag(best.0) + mag(best.1) + best.2 + best.3 {
                best = (l, r, lt, rt);
            }
        }
        (best.0, best.1, best.2, best.3, n)
    })
}
