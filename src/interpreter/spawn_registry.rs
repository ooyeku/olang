/// The registry of `spawn`ed background threads. `task.join` collects
/// through here; results are memoized so a cloned task handle can be
/// joined more than once.
use crate::ast::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;

enum TaskSlot {
    Running(JoinHandle<Result<Value, String>>),
    /// Another thread is currently joining this task.
    Joining,
    Done(Result<Value, String>),
}

/// One task's registry entry: its slot plus when it was spawned, so
/// `task.list()` can report elapsed time.
struct Entry {
    slot: TaskSlot,
    started: crate::clock::Instant,
    /// Channels declared to die with this task (`task.watch`): closed
    /// when the task ends, however it ends.
    watched: Vec<Value>,
    /// Set by the task's own thread as its last act, under the lock, so a
    /// `watch` that arrives after it sees the task as gone and closes the
    /// channel itself — no window in which a channel is watched by
    /// nobody.
    finished: bool,
    /// The task's thread's stop slot: what `task.interrupt` asks.
    stop: crate::interrupt::Handle,
    /// Disconnected once the task's thread ends (it holds the only
    /// sender): what a join waits on.
    done: crossbeam_channel::Receiver<()>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static TASKS: OnceLock<Mutex<HashMap<u64, Entry>>> = OnceLock::new();

fn tasks() -> &'static Mutex<HashMap<u64, Entry>> {
    TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A snapshot of every task a live handle still watches: (id, state,
/// elapsed ms since spawn), for `task.list()`. A `Running` slot whose
/// thread has already finished reports "done" — the memoized join just
/// has not collected it yet, and "running" would be a lie.
pub(crate) fn list() -> Vec<(u64, &'static str, u64)> {
    let map = tasks().lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<(u64, &'static str, u64)> = map
        .iter()
        .map(|(id, e)| {
            let state = match &e.slot {
                TaskSlot::Running(h) if h.is_finished() => "done",
                // asked to stop, and not there yet: in a system call,
                // say, it raises once the call returns
                TaskSlot::Running(_) if e.stop.interrupt_pending() => "interrupting",
                TaskSlot::Running(_) => "running",
                TaskSlot::Joining => "joining",
                TaskSlot::Done(_) => "done",
            };
            (*id, state, e.started.elapsed().as_millis() as u64)
        })
        .collect();
    out.sort();
    out
}

/// Declare that `channel` dies with task `id`. True when the task is
/// still running and now holds the channel; false when the task is gone
/// (or unknown), in which case the caller closes the channel itself.
pub(crate) fn watch(id: u64, channel: Value) -> bool {
    let mut map = tasks().lock().unwrap_or_else(|e| e.into_inner());
    match map.get_mut(&id) {
        Some(entry) if !entry.finished => {
            entry.watched.push(channel);
            true
        }
        _ => false,
    }
}

/// The task's own thread, ending: mark it finished and take the channels
/// it was watching, in one lock acquisition.
pub(crate) fn finish(id: u64) -> Vec<Value> {
    let mut map = tasks().lock().unwrap_or_else(|e| e.into_inner());
    match map.get_mut(&id) {
        Some(entry) => {
            entry.finished = true;
            std::mem::take(&mut entry.watched)
        }
        None => Vec::new(),
    }
}

/// `task.interrupt`: ask task `id` to stop at its next poll. `Some(true)`
/// when it was asked, `Some(false)` when it had already ended, `None`
/// for an unknown id.
pub(crate) fn interrupt(id: u64, reason: Option<String>) -> Option<bool> {
    let stop = {
        let map = lock_tasks();
        let e = map.get(&id)?;
        let running = !e.finished && matches!(&e.slot, TaskSlot::Running(h) if !h.is_finished());
        if !running {
            return Some(false);
        }
        e.stop.clone()
    };
    stop.interrupt(reason);
    Some(true)
}

/// Task `id`'s stop slot, for asking things of its thread.
pub(crate) fn stop_handle(id: u64) -> Option<crate::interrupt::Handle> {
    lock_tasks().get(&id).map(|e| e.stop.clone())
}

fn lock_tasks() -> std::sync::MutexGuard<'static, HashMap<u64, Entry>> {
    tasks().lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn register(
    id: u64,
    handle: JoinHandle<Result<Value, String>>,
    stop: crate::interrupt::Handle,
    done: crossbeam_channel::Receiver<()>,
) {
    tasks().lock().unwrap_or_else(|e| e.into_inner()).insert(
        id,
        Entry {
            slot: TaskSlot::Running(handle),
            started: crate::clock::Instant::now(),
            watched: Vec::new(),
            finished: false,
            stop,
            done,
        },
    );
}

/// Held by a `Task` handle (behind an `Arc`, so all clones of the handle
/// share one). When the last clone is dropped — the last place that could
/// still join this task is gone — its `Drop` removes
/// the registry entry. That makes reclamation *exact*: a completed task's
/// memoized result lives exactly as long as a handle can still ask for
/// it, and no longer, so a program that spawns a worker pool every tick
/// does not accumulate `Done` entries forever (the per-tick leak this
/// fixes).
#[derive(Debug)]
pub struct SpawnGuard {
    id: u64,
}

impl SpawnGuard {
    pub fn new(id: u64) -> Self {
        SpawnGuard { id }
    }
}

// Two guards are equal iff they watch the same task — but each spawn makes
// exactly one guard (shared by Arc), so this only ever compares a task
// to a clone of itself. Keeps `Value`'s derived `PartialEq` honest.
impl PartialEq for SpawnGuard {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Drop for SpawnGuard {
    fn drop(&mut self) {
        forget(self.id);
    }
}

/// Drop a task's registry entry. A `Running` handle is dropped, which
/// detaches its thread (it finishes on its own and the OS reaps it — a
/// dropped-before-join spawn is fire-and-forget); a `Done` result is
/// freed. Only ever called from `SpawnGuard::drop`, when no handle can
/// still join the task, so this never races an in-flight join.
fn forget(id: u64) {
    if let Some(map) = TASKS.get() {
        map.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    }
}

/// What a look at a task found.
enum Look {
    /// Gone from the registry.
    Unknown,
    /// Finished and collected: its memoized result.
    Done(Result<Value, String>),
    /// Finished, and this caller now owns the join.
    Collect(JoinHandle<Result<Value, String>>),
    /// Still running (wait on its done channel), or another thread is
    /// mid-join (`None`: look again shortly).
    Wait(Option<crossbeam_channel::Receiver<()>>),
}

fn look(id: u64) -> Look {
    let mut map = lock_tasks();
    let Some(entry) = map.get_mut(&id) else {
        return Look::Unknown;
    };
    match &entry.slot {
        TaskSlot::Done(r) => Look::Done(r.clone()),
        TaskSlot::Joining => Look::Wait(None),
        TaskSlot::Running(h) => {
            let ended = h.is_finished()
                || matches!(
                    entry.done.try_recv(),
                    Err(crossbeam_channel::TryRecvError::Disconnected)
                );
            if !ended {
                return Look::Wait(Some(entry.done.clone()));
            }
            // The thread is finished, so it has drained its watched
            // channels already.
            entry.finished = true;
            match std::mem::replace(&mut entry.slot, TaskSlot::Joining) {
                TaskSlot::Running(h) => Look::Collect(h),
                _ => unreachable!("slot state checked under the same lock"),
            }
        }
    }
}

/// Join a finished thread and memoize its result.
fn collect(id: u64, h: JoinHandle<Result<Value, String>>) -> Result<Value, String> {
    let result = h
        .join()
        .unwrap_or_else(|_| Err("task thread panicked".to_string()));
    if let Some(entry) = lock_tasks().get_mut(&id) {
        entry.slot = TaskSlot::Done(result.clone());
    }
    result
}

/// Wait on a task's done channel for up to `wait`, ending at once when the
/// task ends (its thread drops the channel's sender) or when this thread
/// is asked to stop. `None` (another thread is mid-join) waits briefly.
fn wait_done(done: Option<&crossbeam_channel::Receiver<()>>, wait: std::time::Duration) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(done) = done {
        let wake = crate::interrupt::wake_receiver();
        let mut sel = crossbeam_channel::Select::new();
        let at_done = sel.recv(done);
        sel.recv(&wake);
        if let Ok(op) = sel.select_timeout(wait) {
            if op.index() == at_done {
                let _ = op.recv(done);
            } else {
                let _ = op.recv(&wake);
            }
        }
        return;
    }
    let _ = done;
    crate::clock::sleep(wait.min(std::time::Duration::from_micros(50)));
}

/// Join the task if it finishes before `deadline`, otherwise report the
/// timeout. `Some(None)` means "still running" — the thread is left
/// alone and keeps working; a timeout here stops the *waiting*, not the
/// work. `None` means the id is unknown. `Err` is the joiner's own stop.
pub(crate) fn join_until(
    id: u64,
    deadline: crate::clock::Instant,
) -> Result<Option<Option<Result<Value, String>>>, String> {
    loop {
        let done = match look(id) {
            Look::Unknown => return Ok(None),
            Look::Done(r) => return Ok(Some(Some(r))),
            Look::Collect(h) => return Ok(Some(Some(collect(id, h)))),
            Look::Wait(done) => done,
        };
        if crate::interrupt::pending() {
            return Err(crate::interrupt::stop_message());
        }
        let now = crate::clock::Instant::now();
        if now >= deadline {
            return Ok(Some(None));
        }
        wait_done(done.as_ref(), deadline - now);
    }
}

/// Join the task (or return its memoized result). Returns `None` for an
/// unknown id. The registry lock is never held across the join itself —
/// a spawned task may join other tasks, and holding the lock while
/// blocking would deadlock. The wait wakes when the task's thread ends,
/// through a channel that thread holds, so a join costs no polling.
/// `Err` is the joiner's own stop (an interrupt, a budget's trip, a
/// cancel), which ends the wait; the task runs on.
pub(crate) fn join(id: u64) -> Result<Option<Result<Value, String>>, String> {
    // A task already done answers without parking.
    match look(id) {
        Look::Unknown => return Ok(None),
        Look::Done(r) => return Ok(Some(r)),
        Look::Collect(h) => return Ok(Some(collect(id, h))),
        Look::Wait(_) => {}
    }
    // An unbounded wait: park it so the stall detector counts this
    // thread among the blocked. Sound to include — a join wakes when
    // its target finishes, which requires the target to be running, so
    // "every thread parked" still proves nothing can progress.
    let token = crate::stdlib::chan::parked_token(format!("task.join on task {}", id));
    let result = join_inner(id, token.as_ref());
    drop(token);
    result
}

fn join_inner(
    id: u64,
    park: Option<&crate::stdlib::chan::ParkToken>,
) -> Result<Option<Result<Value, String>>, String> {
    let tick = std::time::Duration::from_millis(crate::stdlib::chan::STALL_TICK_MS);
    loop {
        let done = match look(id) {
            Look::Unknown => return Ok(None),
            Look::Done(r) => return Ok(Some(r)),
            Look::Collect(h) => return Ok(Some(collect(id, h))),
            Look::Wait(done) => done,
        };
        if crate::interrupt::pending() {
            return Err(crate::interrupt::stop_message());
        }
        if let Some(p) = park {
            p.tick();
        }
        wait_done(done.as_ref(), tick);
    }
}
