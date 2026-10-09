//! Where an error went: the frames it unwound through, each with the
//! file and line that frame was at.
//!
//! A program that catches an error at a boundary (`attempt`, Loom's run
//! loop, a plugin host's `runtime.call_budget`) used to get the message
//! alone — "str.trim: argument 1 must be a string, got Unit" with no
//! idea where. Both tiers now note each frame an error leaves, innermost
//! first: the tree-walker at the first located statement of each frame
//! the error passes, the VM as each frame unwinds (its failing pc's
//! line). The boundary that turns the error into a value keeps the
//! trace here, per thread, and `runtime.last_error()` reads it back.
//!
//! Nothing is paid on the way in: the frames are noted only while an
//! error unwinds.

use std::cell::RefCell;

/// One frame an error left: its file (a full path when known), the line
/// it was at, and the function's name (`None` at a module's top level).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub file: Option<String>,
    pub line: u32,
    pub function: Option<String>,
}

thread_local! {
    static LAST: RefCell<Option<(String, Vec<Frame>)>> = const { RefCell::new(None) };
    /// The frames of the error unwinding now, innermost first.
    static PENDING: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
    /// Who noted the last frame: (interpreter, call depth); (0, 0) for
    /// the VM, whose frames are noted one each as they unwind.
    static MARK: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// The most frames a trace keeps (a runaway recursion's are all alike).
const MOST: usize = 64;

/// A frame the unwinding error is leaving. `fresh`: nothing has caught
/// this error's place yet, so an earlier error's frames are dropped.
/// `mark` names the frame (an interpreter and its depth): a frame that
/// already said where it was (its innermost statement) is not noted
/// again for an outer statement of its own.
pub fn note(fresh: bool, mark: (usize, usize), frame: Frame) {
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        if fresh {
            p.clear();
            MARK.with(|m| m.set((usize::MAX, usize::MAX)));
        }
        if mark != (0, 0) && MARK.with(|m| m.get()) == mark {
            return;
        }
        MARK.with(|m| m.set(mark));
        if p.len() < MOST {
            p.push(frame);
        }
    });
}

/// A boundary begins (an `attempt`): whatever an earlier error left is
/// dropped.
pub fn begin() {
    PENDING.with(|p| p.borrow_mut().clear());
    MARK.with(|m| m.set((usize::MAX, usize::MAX)));
}

/// The unwinding error's frames, taken (a boundary caught it).
pub fn take_pending() -> Vec<Frame> {
    MARK.with(|m| m.set((usize::MAX, usize::MAX)));
    dedup(PENDING.with(|p| std::mem::take(&mut *p.borrow_mut())))
}

/// The error caught at a boundary: its message and the frames it
/// unwound through kept for `runtime.last_error()`.
pub fn caught(message: &str) -> Vec<Frame> {
    let frames = take_pending();
    keep(message, frames.clone());
    frames
}

/// The error a boundary on this thread caught last, with its frames.
pub fn keep(message: &str, frames: Vec<Frame>) {
    LAST.with(|l| *l.borrow_mut() = Some((message.to_string(), frames)));
}

/// What `keep` kept last on this thread.
pub fn last() -> Option<(String, Vec<Frame>)> {
    LAST.with(|l| l.borrow().clone())
}

/// The frames as olang values: `[#{ file, line, fn }]`, innermost first.
pub fn frames_value(frames: &[Frame]) -> crate::ast::Value {
    use crate::ast::Value;
    use std::sync::Arc;
    let rows: Vec<Value> = frames
        .iter()
        .map(|f| {
            let mut m = crate::ast::ValueMap::default();
            m.insert(
                "file".to_string(),
                f.file
                    .as_ref()
                    .map(|s| Value::String(Arc::new(s.clone())))
                    .unwrap_or(Value::Unit),
            );
            m.insert("line".to_string(), Value::Integer(f.line as i64));
            m.insert(
                "fn".to_string(),
                f.function
                    .as_ref()
                    .map(|s| Value::String(Arc::new(s.clone())))
                    .unwrap_or(Value::Unit),
            );
            Value::Map(Arc::new(m))
        })
        .collect();
    Value::List(Arc::from(rows))
}

/// Drop a repeated frame: a tier boundary notes the frame it crossed on
/// both sides (the VM's frame, then the interpreter's call of it).
pub fn dedup(frames: Vec<Frame>) -> Vec<Frame> {
    let mut out: Vec<Frame> = Vec::with_capacity(frames.len());
    for f in frames {
        if out.last() != Some(&f) {
            out.push(f);
        }
    }
    out
}
