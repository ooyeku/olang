//! The `tty` module on Windows, over crossterm: console raw mode, mouse
//! capture, and the console's input records (keys, mouse, resize, focus)
//! mapped onto the same events the Unix decoder produces. Output is VT
//! sequences (Windows Terminal and ConPTY consoles; the legacy console
//! is out of scope, as in Heddle's spec).
//!
//! Compiled and reasoned about, not yet run: the development machine for
//! H0 is macOS. Known differences from Unix: `tty.query` answers `Err`
//! (the console delivers records, not the reply bytes), bracketed paste
//! arrives as keys, `kitty_keys` has no effect (the console reports keys
//! unambiguously already), and there is no `tty.suspend` (no job control).

use super::decode::{Input, Key, Mouse};
use super::guard::Snapshot;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use std::io::Write;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub(crate) fn is_tty_in() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

pub(crate) fn is_tty_out() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

pub(crate) fn size() -> Result<(u16, u16), String> {
    if !is_tty_out() {
        return Err("tty.size: stdout is not a terminal".to_string());
    }
    crossterm::terminal::size().map_err(|e| format!("tty.size: {e}"))
}

fn write_out(bytes: &[u8]) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}

/// Turn on what `snap` describes.
pub(crate) fn enter(snap: &Snapshot) {
    // Enables VT output processing on the console, once.
    let _ = crossterm::ansi_support::supports_ansi();
    if snap.raw {
        let _ = crossterm::terminal::enable_raw_mode();
    }
    if snap.mouse {
        let _ = crossterm::execute!(std::io::stdout(), event::EnableMouseCapture);
    }
    write_out(&snap.enter);
}

/// Undo what `enter` turned on.
pub(crate) fn leave(snap: &Snapshot) {
    write_out(&snap.leave);
    if snap.mouse {
        let _ = crossterm::execute!(std::io::stdout(), event::DisableMouseCapture);
    }
    if snap.raw {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn key_of(code: KeyCode, mods: KeyModifiers) -> Option<Key> {
    let (name, text) = match code {
        KeyCode::Char(' ') => ("space".to_string(), " ".to_string()),
        KeyCode::Char(c) => (c.to_string(), c.to_string()),
        KeyCode::Enter => ("enter".into(), String::new()),
        KeyCode::Tab => ("tab".into(), String::new()),
        KeyCode::BackTab => ("tab".into(), String::new()),
        KeyCode::Backspace => ("backspace".into(), String::new()),
        KeyCode::Esc => ("esc".into(), String::new()),
        KeyCode::Up => ("up".into(), String::new()),
        KeyCode::Down => ("down".into(), String::new()),
        KeyCode::Left => ("left".into(), String::new()),
        KeyCode::Right => ("right".into(), String::new()),
        KeyCode::Home => ("home".into(), String::new()),
        KeyCode::End => ("end".into(), String::new()),
        KeyCode::PageUp => ("pgup".into(), String::new()),
        KeyCode::PageDown => ("pgdown".into(), String::new()),
        KeyCode::Insert => ("insert".into(), String::new()),
        KeyCode::Delete => ("delete".into(), String::new()),
        KeyCode::F(n) => (format!("f{n}"), String::new()),
        KeyCode::Null => ("space".into(), String::new()),
        _ => return None,
    };
    let ctrl = mods.contains(KeyModifiers::CONTROL) || code == KeyCode::Null;
    let alt = mods.contains(KeyModifiers::ALT);
    let super_ = mods.contains(KeyModifiers::SUPER);
    let shift = mods.contains(KeyModifiers::SHIFT) || code == KeyCode::BackTab;
    let text = if ctrl || alt || super_ {
        String::new()
    } else {
        text
    };
    Some(Key {
        key: name,
        text,
        ctrl,
        alt,
        shift,
        super_,
    })
}

fn input_of(ev: Event) -> Option<Input> {
    Some(match ev {
        Event::Key(k) => {
            if k.kind == KeyEventKind::Release {
                return None;
            }
            Input::Key(key_of(k.code, k.modifiers)?)
        }
        Event::Paste(text) => Input::Paste(text),
        Event::FocusGained => Input::Focus(true),
        Event::FocusLost => Input::Focus(false),
        Event::Mouse(m) => {
            let button = |b: MouseButton| match b {
                MouseButton::Left => "left",
                MouseButton::Right => "right",
                MouseButton::Middle => "middle",
            };
            let (action, button) = match m.kind {
                MouseEventKind::Down(b) => ("down", button(b)),
                MouseEventKind::Up(b) => ("up", button(b)),
                MouseEventKind::Drag(b) => ("drag", button(b)),
                MouseEventKind::Moved => ("move", "none"),
                MouseEventKind::ScrollUp => ("wheel", "up"),
                MouseEventKind::ScrollDown => ("wheel", "down"),
                MouseEventKind::ScrollLeft => ("wheel", "left"),
                MouseEventKind::ScrollRight => ("wheel", "right"),
            };
            Input::Mouse(Mouse {
                action,
                button,
                x: m.column as u32,
                y: m.row as u32,
                ctrl: m.modifiers.contains(KeyModifiers::CONTROL),
                alt: m.modifiers.contains(KeyModifiers::ALT),
                shift: m.modifiers.contains(KeyModifiers::SHIFT),
            })
        }
        Event::Resize(..) => return None, // handled by the caller
    })
}

/// Read console input until told to stop.
pub(crate) fn reader(ctx: super::ReaderCtx) {
    let _live = crate::stdlib::chan::live_guard();
    loop {
        if ctx.stop.load(Ordering::SeqCst) {
            break;
        }
        match event::poll(Duration::from_millis(100)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => {
                ctx.eof();
                break;
            }
        }
        if ctx.stop.load(Ordering::SeqCst) {
            break;
        }
        match event::read() {
            Ok(Event::Resize(cols, rows)) => ctx.resize(cols, rows),
            Ok(ev) => {
                if let Some(input) = input_of(ev) {
                    ctx.deliver(vec![input]);
                }
            }
            Err(_) => {
                ctx.eof();
                break;
            }
        }
    }
}
