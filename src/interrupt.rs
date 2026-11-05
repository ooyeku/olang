//! Stopping a running evaluation: from another thread, after a time
//! budget, or after a number of steps.
//!
//! Every thread that runs olang code has a **stop slot** of its own (a
//! [`Slot`], made the first time the thread asks). Whatever asks a thread
//! to stop writes to that thread's slot, never to a shared flag:
//!
//! - `task.interrupt(t, reason)` asks the task's thread to stop once:
//!   the next poll raises "interrupted: reason" there, and the request is
//!   used up, so an `attempt` that catches it carries on normally.
//! - `runtime.call_budget(f, args, ms)` gives the calling thread a
//!   deadline. One watchdog thread keeps every thread's deadlines in a
//!   min-heap and trips the slot of the thread whose deadline passed;
//!   the trip stands until that budget's call ends, so code inside the
//!   call cannot catch it and run on.
//! - `runtime.call_cancellable(f, args, token)` stops the thread once
//!   the token is set.
//! - The REPL server's interrupt (`request`) is the one request for every
//!   thread at once.
//! - A step budget (`meter_begin`, what `meta.eval`'s `max_steps` uses)
//!   counts loop iterations and calls on this thread, on every tier.
//!
//! **The polls.** Each tier polls where a program can spin: the
//! tree-walker at every statement and loop iteration, the bytecode VM at
//! every backward jump, self tail call and call entry, and native code at
//! its back edges and self tail calls. The poll's fast path is one
//! relaxed load of a process-wide byte, false while no thread anywhere is
//! asked anything. Only when it is set does the poll take the slow path
//! and ask whether the request is for this thread. Native code always
//! compiles its poll (it used to compile it only after the first budget,
//! so code compiled earlier never stopped).
//!
//! **Waits.** A thread blocked in a wait (`chan.recv`, `chan.ask`, a
//! bounded `chan.send`, `task.join`, `time.sleep`) cannot poll, so its
//! slot has a wake channel too. Asking the thread to stop sends one wake
//! on it, and every wait selects on it next to what it waits for, so the
//! wait ends at once and raises.
//!
//! The error is an ordinary runtime error, so the evaluation unwinds as
//! any failure does.

use crate::clock::Instant;
use crate::stdlib::cancel::Token;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::Duration;

/// The polls' byte: true while any thread may be asked to stop (an
/// interrupt, a budget's trip, a set token a cancellable call watches, a
/// step budget running). Only `pending`'s slow path asks whether it is
/// this thread. Written only under `FLAG_LOCK`, by `refresh`.
static REQUESTED: AtomicBool = AtomicBool::new(false);
/// The REPL's interrupt: every thread stops.
static EVERYONE: AtomicBool = AtomicBool::new(false);
/// How many slots have a request standing.
static STANDING: AtomicUsize = AtomicUsize::new(0);
/// How many threads run under a step budget.
static METERED: AtomicUsize = AtomicUsize::new(0);
static FLAG_LOCK: Mutex<()> = Mutex::new(());
static NEXT_THREAD: AtomicU64 = AtomicU64::new(1);
static NEXT_BUDGET: AtomicU64 = AtomicU64::new(1);

/// A slot's request bits.
const ASKED: u8 = 1;
const BUDGET: u8 = 2;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One budget running on a thread.
struct Budget {
    id: u64,
    tripped: bool,
    /// What the trip raises; `None` is the plain "interrupted".
    message: Option<String>,
}

/// One thread's stop slot. Shared (an `Arc`) between the thread and
/// whatever may ask it to stop: its task's registry entry, the watchdog,
/// a cancellable call's watch.
pub struct Slot {
    no: u64,
    bits: AtomicU8,
    /// The reason `task.interrupt` gave, for the error it raises.
    reason: Mutex<Option<String>>,
    /// This thread's budgets, outermost first.
    budgets: Mutex<Vec<Budget>>,
    #[cfg(not(target_arch = "wasm32"))]
    wake_tx: crossbeam_channel::Sender<()>,
    #[cfg(not(target_arch = "wasm32"))]
    wake_rx: crossbeam_channel::Receiver<()>,
}

/// Every slot made, so the REPL's interrupt can wake every wait.
static ALL: Mutex<Vec<Weak<Slot>>> = Mutex::new(Vec::new());

fn made(slot: Arc<Slot>) -> Arc<Slot> {
    let mut all = lock(&ALL);
    all.retain(|w| w.strong_count() > 0);
    all.push(Arc::downgrade(&slot));
    slot
}

impl Slot {
    fn new() -> Slot {
        #[cfg(not(target_arch = "wasm32"))]
        let (wake_tx, wake_rx) = crossbeam_channel::bounded(1);
        Slot {
            no: NEXT_THREAD.fetch_add(1, Ordering::Relaxed),
            bits: AtomicU8::new(0),
            reason: Mutex::new(None),
            budgets: Mutex::new(Vec::new()),
            #[cfg(not(target_arch = "wasm32"))]
            wake_tx,
            #[cfg(not(target_arch = "wasm32"))]
            wake_rx,
        }
    }

    fn set_bit(&self, bit: u8) {
        let _g = lock(&FLAG_LOCK);
        let before = self.bits.fetch_or(bit, Ordering::SeqCst);
        if before == 0 {
            STANDING.fetch_add(1, Ordering::SeqCst);
        }
        store_requested();
        drop(_g);
        self.wake();
    }

    fn clear_bit(&self, bit: u8) {
        let _g = lock(&FLAG_LOCK);
        let before = self.bits.fetch_and(!bit, Ordering::SeqCst);
        if before != 0 && before & !bit == 0 {
            STANDING.fetch_sub(1, Ordering::SeqCst);
        }
        store_requested();
    }

    /// End this thread's wait, if it is in one (a stale wake only makes
    /// a wait look again).
    fn wake(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.wake_tx.try_send(());
    }

    fn asked_here(&self) -> bool {
        self.bits.load(Ordering::SeqCst) != 0
    }
}

/// A handle on a thread's stop slot: what `task.interrupt` holds for the
/// task it may stop.
#[derive(Clone)]
pub struct Handle(Arc<Slot>);

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<stop slot {}>", self.0.no)
    }
}

impl Handle {
    /// A slot for a thread about to start: the spawning side keeps this
    /// handle, and the new thread `adopt`s it first thing.
    pub fn fresh() -> Handle {
        Handle(made(Arc::new(Slot::new())))
    }

    /// Ask the thread to stop at its next poll, once: it raises
    /// "interrupted" (with `reason` after a colon, when given). A wait
    /// the thread is in ends at once.
    pub fn interrupt(&self, reason: Option<String>) {
        used();
        *lock(&self.0.reason) = reason;
        self.0.set_bit(ASKED);
    }

    /// Whether a deadline runs on the thread (a budget, a `meta.eval`
    /// timeout): its wait will be ended by the watchdog, from outside
    /// the program's threads.
    pub fn has_deadline(&self) -> bool {
        !lock(&self.0.budgets).is_empty()
    }

    /// Whether an interrupt was asked and not yet raised there (a thread
    /// in a system call raises once the call returns).
    pub fn interrupt_pending(&self) -> bool {
        self.0.bits.load(Ordering::SeqCst) & ASKED != 0
    }
}

thread_local! {
    static SLOT: std::cell::RefCell<Option<Arc<Slot>>> = const { std::cell::RefCell::new(None) };
    /// The tokens this thread's cancellable calls watch, outermost first.
    static WATCHING: std::cell::RefCell<Vec<Arc<Token>>> = const { std::cell::RefCell::new(Vec::new()) };
    /// This thread's step budgets: steps left, outermost first.
    static METER: std::cell::RefCell<Vec<(u64, u64)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn slot() -> Arc<Slot> {
    SLOT.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| made(Arc::new(Slot::new())))
            .clone()
    })
}

fn with_slot<R>(f: impl FnOnce(&Slot) -> R) -> R {
    SLOT.with(|s| {
        let mut s = s.borrow_mut();
        f(s.get_or_insert_with(|| made(Arc::new(Slot::new()))))
    })
}

/// Take `handle` as this thread's slot (a spawned task's first act, so
/// the task's handle can stop it).
pub fn adopt(handle: &Handle) {
    SLOT.with(|s| *s.borrow_mut() = Some(handle.0.clone()));
}

/// This thread's slot, for something that may stop it later.
pub fn current() -> Handle {
    Handle(slot())
}

/// Every cancellable call running: (its thread's slot, its token).
static WATCHES: Mutex<Vec<(Arc<Slot>, Arc<Token>)>> = Mutex::new(Vec::new());

/// Write the polls' byte from what stands. Call under `FLAG_LOCK`.
fn store_requested() {
    let cancelled = lock(&WATCHES).iter().any(|(_, t)| t.is_set());
    REQUESTED.store(
        EVERYONE.load(Ordering::SeqCst)
            || STANDING.load(Ordering::SeqCst) > 0
            || METERED.load(Ordering::SeqCst) > 0
            || cancelled,
        Ordering::SeqCst,
    );
}

fn refresh() {
    let _g = lock(&FLAG_LOCK);
    store_requested();
}

/// This thread's number (never 0).
pub fn thread_no() -> u64 {
    with_slot(|s| s.no)
}

/// The message every tier raises.
pub const MESSAGE: &str = "interrupted";

/// The long builtins take their stoppable paths from now on (the REPL
/// server arms the process before it evaluates anything). Native code
/// polls whether or not the process is armed.
pub fn arm() {
    used();
}

/// Whether a long builtin should take its stoppable path (a bounded
/// `os.exec`, a chunked http body): true once the process has used
/// anything that can stop a thread.
#[inline]
pub fn armed() -> bool {
    USED.load(Ordering::Relaxed)
}

/// Something that can stop a thread was used (a budget, a token, an
/// interrupt): the long builtins take their stoppable paths from now on.
static USED: AtomicBool = AtomicBool::new(false);

fn used() {
    USED.store(true, Ordering::Relaxed);
}

/// Ask the running evaluation to stop (every thread's): the REPL server's
/// interrupt.
pub fn request() {
    used();
    {
        let _g = lock(&FLAG_LOCK);
        EVERYONE.store(true, Ordering::SeqCst);
        store_requested();
    }
    for s in lock(&ALL).iter().filter_map(|w| w.upgrade()) {
        s.wake();
    }
}

/// Forget the REPL's request and an interrupt asked of this thread (at
/// the start of an evaluation). A budget or a cancellable call still
/// running keeps its say.
pub fn clear() {
    {
        let _g = lock(&FLAG_LOCK);
        EVERYONE.store(false, Ordering::SeqCst);
        store_requested();
    }
    with_slot(|s| s.clear_bit(ASKED));
}

/// Whether a stop was asked for this thread: one relaxed load, a
/// predicted-false branch at each poll; only a request standing asks
/// whom it is for.
#[inline(always)]
pub fn pending() -> bool {
    REQUESTED.load(Ordering::Relaxed) && aimed_here()
}

/// A poll at a step: a loop's back edge or a call (the VM's polls and
/// native code's). As `pending`, and under a step budget it also counts
/// the step.
#[inline(always)]
pub fn pending_step() -> bool {
    REQUESTED.load(Ordering::Relaxed) && step_slow()
}

#[cold]
#[inline(never)]
fn step_slow() -> bool {
    charge_step() || aimed_here()
}

#[cold]
#[inline(never)]
fn aimed_here() -> bool {
    if EVERYONE.load(Ordering::SeqCst) {
        return true;
    }
    if with_slot(|s| s.asked_here()) {
        return true;
    }
    if METER.with(|m| m.borrow().iter().any(|(_, left)| *left == 0)) {
        return true;
    }
    WATCHING.with(|w| w.borrow().iter().any(|t| t.is_set()))
}

/// What native code calls when the polls' byte is set: 1 when the stop
/// is for this thread (the body deopts), 0 to run on.
pub extern "C" fn native_poll() -> u64 {
    step_slow() as u64
}

/// The error a stop raises here, and the request used up when it is a
/// once-only one (`task.interrupt`'s). Call when `pending` said true.
pub fn stop_message() -> String {
    if METER.with(|m| m.borrow().iter().any(|(_, left)| *left == 0)) {
        return STEPS_MESSAGE.to_string();
    }
    let s = slot();
    let bits = s.bits.load(Ordering::SeqCst);
    if bits & BUDGET != 0 {
        let budgets = lock(&s.budgets);
        if let Some(b) = budgets.iter().rev().find(|b| b.tripped) {
            return b.message.clone().unwrap_or_else(|| MESSAGE.to_string());
        }
    }
    if bits & ASKED != 0 && !EVERYONE.load(Ordering::SeqCst) {
        let reason = lock(&s.reason).take();
        s.clear_bit(ASKED);
        return match reason {
            Some(r) if !r.is_empty() => format!("{MESSAGE}: {r}"),
            _ => MESSAGE.to_string(),
        };
    }
    MESSAGE.to_string()
}

// ── waits ───────────────────────────────────────────────────────────
/// This thread's wake channel: a blocking wait selects on it next to
/// what it waits for, and checks `pending` whenever it fires.
#[cfg(not(target_arch = "wasm32"))]
pub fn wake_receiver() -> crossbeam_channel::Receiver<()> {
    with_slot(|s| s.wake_rx.clone())
}

/// Sleep `d`, or less when this thread is asked to stop: `Err` with the
/// stop's message then.
#[cfg(not(target_arch = "wasm32"))]
pub fn sleep(d: Duration) -> Result<(), String> {
    let wake = wake_receiver();
    let deadline = Instant::now() + d;
    loop {
        if pending() {
            return Err(stop_message());
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(());
        }
        let _ = wake.recv_timeout(deadline - now);
    }
}

// ── cancellation ────────────────────────────────────────────────────
/// The message a cancelled call's error carries once caught.
pub const CANCEL_MESSAGE: &str = "cancelled";

/// A cancellable call begun on this thread: once `token` is set, this
/// thread's evaluation is stopped as an interrupt is, until
/// `cancel_end`.
pub fn cancel_begin(token: Arc<Token>) {
    used();
    WATCHING.with(|w| w.borrow_mut().push(token.clone()));
    lock(&WATCHES).push((slot(), token));
    refresh();
}

/// The cancellable call begun last on this thread ended: whether its
/// token was set.
pub fn cancel_end() -> bool {
    let me = slot();
    let token = WATCHING.with(|w| w.borrow_mut().pop());
    if let Some(token) = &token {
        let mut watches = lock(&WATCHES);
        if let Some(i) = watches
            .iter()
            .rposition(|(s, k)| Arc::ptr_eq(s, &me) && Arc::ptr_eq(k, token))
        {
            watches.remove(i);
        }
    }
    refresh();
    token.is_some_and(|t| t.is_set())
}

/// `cancel.set(token)` just set it: the threads watching it stop at their
/// next poll, and a wait they are in ends.
pub fn cancel_set(token: &Arc<Token>) {
    refresh();
    for (s, t) in lock(&WATCHES).iter() {
        if Arc::ptr_eq(t, token) {
            s.wake();
        }
    }
}

/// Whether this thread runs inside a cancellable call whose token is
/// set, or was asked to stop otherwise: what a long builtin checks
/// between its steps (the same answer the evaluators' polls get).
pub fn should_stop() -> bool {
    pending()
}

/// This thread's stop question, for helper threads working on its behalf
/// (a file walk's workers): it asks what `should_stop` would ask here.
#[derive(Clone)]
pub struct StopProbe {
    slot: Arc<Slot>,
    tokens: Vec<Arc<Token>>,
}

impl StopProbe {
    pub fn here() -> StopProbe {
        StopProbe {
            slot: slot(),
            tokens: WATCHING.with(|w| w.borrow().clone()),
        }
    }

    #[inline]
    pub fn should_stop(&self) -> bool {
        if !REQUESTED.load(Ordering::Relaxed) {
            return false;
        }
        EVERYONE.load(Ordering::SeqCst)
            || self.slot.asked_here()
            || self.tokens.iter().any(|t| t.is_set())
    }
}

// ── budgets ─────────────────────────────────────────────────────────
/// The message a call over its budget raises (as `interrupted` does).
pub const BUDGET_MESSAGE: &str = "interrupted: over its time budget";

/// The watchdog's view: every budget running on any thread, by deadline.
struct Dog {
    heap: BinaryHeap<Reverse<(Instant, u64)>>,
    live: HashMap<u64, Weak<Slot>>,
}

fn dog() -> &'static (Mutex<Dog>, Condvar) {
    static W: OnceLock<(Mutex<Dog>, Condvar)> = OnceLock::new();
    W.get_or_init(|| {
        std::thread::Builder::new()
            .name("olang-budget".into())
            .spawn(watchdog)
            .expect("the budget watchdog starts");
        (
            Mutex::new(Dog {
                heap: BinaryHeap::new(),
                live: HashMap::new(),
            }),
            Condvar::new(),
        )
    })
}

fn watchdog() {
    let (m, cv) = dog();
    let mut d = lock(m);
    loop {
        let Some(Reverse((deadline, id))) = d.heap.peek().copied() else {
            d = cv.wait(d).unwrap_or_else(|e| e.into_inner());
            continue;
        };
        if !d.live.contains_key(&id) {
            d.heap.pop();
            continue;
        }
        let now = Instant::now();
        if now < deadline {
            d = cv
                .wait_timeout(d, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
            continue;
        }
        d.heap.pop();
        if let Some(slot) = d.live.remove(&id).and_then(|w| w.upgrade()) {
            // Under the watchdog's lock: a `budget_end` that comes after
            // finds the budget gone from `live` and its trip recorded.
            let mut budgets = lock(&slot.budgets);
            if let Some(b) = budgets.iter_mut().find(|b| b.id == id) {
                b.tripped = true;
                drop(budgets);
                slot.set_bit(BUDGET);
            }
        }
    }
}

/// A running budget, as `budget_begin` answers it: end it with
/// `budget_end`.
#[must_use]
pub struct BudgetId(u64);

/// A budget begun: the calling thread's evaluation is stopped (as an
/// interrupt) once `ms` pass, unless `budget_end` comes first. Each
/// thread's budgets are its own: one thread's deadline never stops
/// another.
pub fn budget_begin(ms: u64) -> BudgetId {
    budget_begin_with(Duration::from_millis(ms.max(1)), None)
}

/// `budget_begin`, the trip raising `message` (the meta.eval timeout's
/// words) instead of "interrupted".
pub fn budget_begin_with(limit: Duration, message: Option<String>) -> BudgetId {
    used();
    let id = NEXT_BUDGET.fetch_add(1, Ordering::Relaxed);
    let s = slot();
    lock(&s.budgets).push(Budget {
        id,
        tripped: false,
        message,
    });
    let (m, cv) = dog();
    let mut d = lock(m);
    let deadline = Instant::now() + limit;
    let earliest = d.heap.peek().is_none_or(|Reverse((t, _))| deadline < *t);
    d.heap.push(Reverse((deadline, id)));
    d.live.insert(id, Arc::downgrade(&s));
    drop(d);
    if earliest {
        cv.notify_all();
    }
    BudgetId(id)
}

/// A budget ended: whether it ran out. Its trip is forgotten, so the code
/// after the call runs on (an outer budget that ran out still stands).
pub fn budget_end(budget: BudgetId) -> bool {
    {
        let (m, _) = dog();
        lock(m).live.remove(&budget.0);
    }
    let s = slot();
    let mut budgets = lock(&s.budgets);
    let tripped = match budgets.iter().rposition(|b| b.id == budget.0) {
        Some(i) => budgets.remove(i).tripped,
        None => false,
    };
    let still = budgets.iter().any(|b| b.tripped);
    drop(budgets);
    if !still {
        s.clear_bit(BUDGET);
    }
    tripped
}

// ── step budgets ────────────────────────────────────────────────────
/// The message a run out of steps raises.
pub const STEPS_MESSAGE: &str = "budget exceeded: the evaluation used up its max_steps";

static NEXT_METER: AtomicU64 = AtomicU64::new(1);

/// A step budget running on this thread; dropping it ends it.
pub struct Meter {
    id: u64,
}

/// Begin a step budget of `steps` on this thread: every loop iteration
/// and call from now on counts, on every tier, and the poll after the
/// last one raises `STEPS_MESSAGE`. Nested budgets each count.
pub fn meter_begin(steps: u64) -> Meter {
    let id = NEXT_METER.fetch_add(1, Ordering::Relaxed);
    let first = METER.with(|m| {
        let mut m = m.borrow_mut();
        m.push((id, steps));
        m.len() == 1
    });
    if first {
        let _g = lock(&FLAG_LOCK);
        METERED.fetch_add(1, Ordering::SeqCst);
        store_requested();
    }
    Meter { id }
}

impl Drop for Meter {
    fn drop(&mut self) {
        let last = METER.with(|m| {
            let mut m = m.borrow_mut();
            let before = m.len();
            m.retain(|(id, _)| *id != self.id);
            before > 0 && m.is_empty()
        });
        if last {
            let _g = lock(&FLAG_LOCK);
            METERED.fetch_sub(1, Ordering::SeqCst);
            store_requested();
        }
    }
}

/// A sandboxed run's bounds kept by its thread: a step budget, a
/// deadline raising its own words, or both. Dropping it ends them.
pub struct Bound {
    _meter: Option<Meter>,
    budget: Option<BudgetId>,
}

impl Bound {
    pub fn begin(steps: Option<u64>, deadline: Option<(Duration, String)>) -> Bound {
        Bound {
            _meter: steps.map(meter_begin),
            budget: deadline.map(|(d, m)| budget_begin_with(d, Some(m))),
        }
    }
}

impl Drop for Bound {
    fn drop(&mut self) {
        if let Some(b) = self.budget.take() {
            let _ = budget_end(b);
        }
    }
}

/// Count one step against this thread's step budgets: true when one has
/// none left (the step is refused). Free when none runs.
#[inline]
pub fn charge_step() -> bool {
    METER.with(|m| {
        let mut m = m.borrow_mut();
        if m.is_empty() {
            return false;
        }
        if m.iter().any(|(_, left)| *left == 0) {
            return true;
        }
        for (_, left) in m.iter_mut() {
            *left -= 1;
        }
        false
    })
}

/// Hand back the step `charge_step` took last (a call the interpreter
/// counted, then gave to the VM, which counts it again).
pub fn refund_step() {
    METER.with(|m| {
        for (_, left) in m.borrow_mut().iter_mut() {
            *left += 1;
        }
    })
}

/// Whether this thread runs under a step budget.
pub fn metered() -> bool {
    METER.with(|m| !m.borrow().is_empty())
}

/// Where the polls' byte lives, for native code to read (0 or 1).
pub fn flag_address() -> usize {
    REQUESTED.as_ptr() as usize
}

/// The stop as the tree-walker's error, built out of line: the polls
/// stay a load and a branch where they sit.
#[cold]
#[inline(never)]
pub fn interpreter_error() -> crate::interpreter::InterpreterError {
    crate::interpreter::InterpreterError::RuntimeError {
        message: stop_message(),
    }
}

/// The stop as the VM's error, built out of line.
#[cold]
#[inline(never)]
pub fn vm_error() -> crate::ovm::bytecode::BytecodeError {
    crate::ovm::bytecode::BytecodeError::RuntimeError(stop_message())
}

/// Whether an error message is a stop's (an interrupt, a budget's trip,
/// a cancel).
pub fn is_interrupt(message: &str) -> bool {
    message == MESSAGE
        || message.starts_with("interrupted: ")
        || message.ends_with(": interrupted")
        || message.contains(": interrupted: ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_trips_only_its_own_thread() {
        let b = budget_begin(5);
        let other = std::thread::spawn(|| {
            crate::clock::sleep(Duration::from_millis(30));
            pending()
        });
        let start = Instant::now();
        while !pending() && start.elapsed() < Duration::from_secs(2) {
            std::hint::spin_loop();
        }
        assert!(pending(), "the budget's thread sees the trip");
        assert!(!other.join().unwrap(), "another thread polls past it");
        assert!(budget_end(b), "the budget says it ran out");
        assert!(!pending(), "the trip is forgotten once the budget ends");
        let b = budget_begin(10_000);
        assert!(!budget_end(b), "a budget that did not run out says so");
    }

    #[test]
    fn two_threads_budgets_trip_each_on_its_own_deadline() {
        // A long budget on one thread must not hold back a short one on
        // another (one shared stack watched only its top).
        let long = std::thread::spawn(|| {
            let b = budget_begin(1_500);
            crate::clock::sleep(Duration::from_millis(300));
            let p = pending();
            let _ = budget_end(b);
            p
        });
        crate::clock::sleep(Duration::from_millis(20));
        let b = budget_begin(50);
        let start = Instant::now();
        while !pending() && start.elapsed() < Duration::from_secs(2) {
            std::hint::spin_loop();
        }
        let took = start.elapsed();
        assert!(budget_end(b));
        assert!(took < Duration::from_millis(400), "took {took:?}");
        assert!(!long.join().unwrap(), "the long budget had not run out");
    }

    #[test]
    fn an_interrupt_is_used_up_by_the_error_it_raises() {
        let h = Handle::fresh();
        let h2 = h.clone();
        let t = std::thread::spawn(move || {
            adopt(&h2);
            let start = Instant::now();
            while !pending() && start.elapsed() < Duration::from_secs(2) {
                std::hint::spin_loop();
            }
            let msg = stop_message();
            (msg, pending())
        });
        crate::clock::sleep(Duration::from_millis(10));
        h.interrupt(Some("restart".into()));
        let (msg, after) = t.join().unwrap();
        assert_eq!(msg, "interrupted: restart");
        assert!(!after, "the interrupt was used up");
    }

    #[test]
    fn a_sleep_ends_when_its_thread_is_interrupted() {
        let h = Handle::fresh();
        let h2 = h.clone();
        let t = std::thread::spawn(move || {
            adopt(&h2);
            let start = Instant::now();
            let r = sleep(Duration::from_secs(5));
            (r, start.elapsed())
        });
        crate::clock::sleep(Duration::from_millis(20));
        h.interrupt(None);
        let (r, took) = t.join().unwrap();
        assert_eq!(r, Err("interrupted".to_string()));
        assert!(took < Duration::from_millis(500), "took {took:?}");
    }

    #[test]
    fn a_step_budget_runs_out() {
        let m = meter_begin(3);
        assert!(!charge_step() && !charge_step() && !charge_step());
        assert!(charge_step(), "the fourth step is refused");
        assert!(pending_step());
        assert_eq!(stop_message(), STEPS_MESSAGE);
        drop(m);
        assert!(!pending_step());
    }
}
