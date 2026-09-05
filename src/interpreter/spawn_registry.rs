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
    started: std::time::Instant,
    /// Channels declared to die with this task (`task.watch`): closed
    /// when the task ends, however it ends.
    watched: Vec<Value>,
    /// Set by the task's own thread as its last act, under the lock, so a
    /// `watch` that arrives after it sees the task as gone and closes the
    /// channel itself — no window in which a channel is watched by
    /// nobody.
    finished: bool,
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
    let map = tasks().lock().unwrap();
    let mut out: Vec<(u64, &'static str, u64)> = map
        .iter()
        .map(|(id, e)| {
            let state = match &e.slot {
                TaskSlot::Running(h) if h.is_finished() => "done",
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
    let mut map = tasks().lock().unwrap();
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
    let mut map = tasks().lock().unwrap();
    match map.get_mut(&id) {
        Some(entry) => {
            entry.finished = true;
            std::mem::take(&mut entry.watched)
        }
        None => Vec::new(),
    }
}

pub(crate) fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn register(id: u64, handle: JoinHandle<Result<Value, String>>) {
    tasks().lock().unwrap().insert(
        id,
        Entry {
            slot: TaskSlot::Running(handle),
            started: std::time::Instant::now(),
            watched: Vec::new(),
            finished: false,
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
        map.lock().unwrap().remove(&id);
    }
}

/// Join the task if it finishes before `deadline`, otherwise report the
/// timeout. `Some(None)` means "still running" — the thread is left
/// alone and keeps working, because an OS thread cannot be cancelled;
/// a timeout here stops the *waiting*, not the work. `None` means the id
/// is unknown.
///
/// Polls rather than blocking, since `JoinHandle` has no timed join. The
/// finished handle is joined normally once `is_finished` reports it, so
/// the memoized result path is shared with `join`.
pub(crate) fn join_until(
    id: u64,
    deadline: std::time::Instant,
) -> Option<Option<Result<Value, String>>> {
    loop {
        let ready = {
            let map = tasks().lock().unwrap();
            match map.get(&id).map(|e| &e.slot) {
                None => return None,
                Some(TaskSlot::Done(r)) => return Some(Some(r.clone())),
                Some(TaskSlot::Running(h)) => h.is_finished(),
                // Another thread is mid-join; treat as not-yet-ready and
                // come back for its memoized result.
                Some(TaskSlot::Joining) => false,
            }
        };
        if ready {
            return Some(join(id));
        }
        if std::time::Instant::now() >= deadline {
            return Some(None);
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Join the task (or return its memoized result). Returns `None` for an
/// unknown id. The registry lock is never held across the join itself —
/// a spawned task may join other tasks, and holding the lock while
/// blocking would deadlock.
pub(crate) fn join(id: u64) -> Option<Result<Value, String>> {
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
) -> Option<Result<Value, String>> {
    loop {
        let handle = {
            let mut map = tasks().lock().unwrap();
            let started = map.get(&id).map(|e| e.started);
            match (map.get(&id).map(|e| &e.slot), started) {
                (None, _) => return None,
                (Some(TaskSlot::Done(r)), _) => return Some(r.clone()),
                (Some(TaskSlot::Joining), _) => None,
                (Some(TaskSlot::Running(h)), Some(started)) => {
                    if h.is_finished() {
                        match map.insert(
                            id,
                            // The thread is finished, so it has drained
                            // its watched channels already.
                            Entry {
                                slot: TaskSlot::Joining,
                                started,
                                watched: Vec::new(),
                                finished: true,
                            },
                        ) {
                            Some(Entry {
                                slot: TaskSlot::Running(h),
                                ..
                            }) => Some((h, started)),
                            _ => unreachable!("slot state checked under the same lock"),
                        }
                    } else {
                        None
                    }
                }
                (Some(TaskSlot::Running(_)), None) => unreachable!("entry has a start time"),
            }
        };
        match handle {
            Some((h, started)) => {
                let result = h
                    .join()
                    .unwrap_or_else(|_| Err("task thread panicked".to_string()));
                tasks().lock().unwrap().insert(
                    id,
                    Entry {
                        slot: TaskSlot::Done(result.clone()),
                        started,
                        watched: Vec::new(),
                        finished: true,
                    },
                );
                return Some(result);
            }
            // Still running, or someone else is mid-join: tick the
            // stall detector and look again.
            None => {
                if let Some(p) = park {
                    p.tick();
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
}
