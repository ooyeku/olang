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

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static TASKS: OnceLock<Mutex<HashMap<u64, TaskSlot>>> = OnceLock::new();

fn tasks() -> &'static Mutex<HashMap<u64, TaskSlot>> {
    TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn register(id: u64, handle: JoinHandle<Result<Value, String>>) {
    tasks()
        .lock()
        .unwrap()
        .insert(id, TaskSlot::Running(handle));
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
            match map.get(&id) {
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
    loop {
        let handle = {
            let mut map = tasks().lock().unwrap();
            match map.get(&id) {
                None => return None,
                Some(TaskSlot::Done(r)) => return Some(r.clone()),
                Some(TaskSlot::Joining) => None,
                Some(TaskSlot::Running(_)) => match map.insert(id, TaskSlot::Joining) {
                    Some(TaskSlot::Running(h)) => Some(h),
                    _ => unreachable!("slot state checked under the same lock"),
                },
            }
        };
        match handle {
            Some(h) => {
                let result = h
                    .join()
                    .unwrap_or_else(|_| Err("task thread panicked".to_string()));
                tasks()
                    .lock()
                    .unwrap()
                    .insert(id, TaskSlot::Done(result.clone()));
                return Some(result);
            }
            // Someone else is joining; wait for their result.
            None => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
}
