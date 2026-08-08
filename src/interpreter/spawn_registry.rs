/// The registry of `spawn`ed background threads. `await` joins through here;
/// results are memoized so a cloned promise value can be awaited repeatedly.
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

pub(super) fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn register(id: u64, handle: JoinHandle<Result<Value, String>>) {
    tasks()
        .lock()
        .unwrap()
        .insert(id, TaskSlot::Running(handle));
}

/// Join the task (or return its memoized result). Returns `None` for an
/// unknown id. The registry lock is never held across the join itself —
/// a spawned task may await other tasks, and holding the lock while
/// blocking would deadlock.
pub(super) fn join(id: u64) -> Option<Result<Value, String>> {
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
