use crate::ast::{PromiseState, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Unique identifier for tasks
pub type TaskId = u64;
/// Unique identifier for promises
pub type PromiseId = u64;

/// Task represents a unit of async work
#[derive(Debug)]
pub struct Task {
    pub id: TaskId,
    pub task_type: TaskType,
    pub promise_id: Option<PromiseId>,
    pub created_at: Instant,
}

/// Type of async task to be executed
#[derive(Debug, Clone)]
pub enum TaskType {
    Delay { ms: u64, value: Value },
    Resolve { value: Value },
    Reject { error: Value },
    Custom { name: String },
}

/// The async runtime manages task execution and promise resolution
pub struct AsyncRuntime {
    /// Queue of tasks waiting to be executed
    task_queue: VecDeque<Task>,
    /// Registry of active promises
    promise_registry: HashMap<PromiseId, Arc<Mutex<Value>>>,
    /// Next available task ID
    next_task_id: TaskId,
    /// Next available promise ID
    next_promise_id: PromiseId,
    /// Whether the runtime is currently running
    running: bool,
}

impl Default for AsyncRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncRuntime {
    /// Create a new async runtime
    pub fn new() -> Self {
        Self {
            task_queue: VecDeque::new(),
            promise_registry: HashMap::new(),
            next_task_id: 1,
            next_promise_id: 1,
            running: false,
        }
    }

    /// Create a new promise with pending state
    pub fn create_promise(&mut self) -> (PromiseId, Value) {
        let id = self.next_promise_id;
        self.next_promise_id += 1;

        let promise = Value::Promise {
            state: PromiseState::Pending,
            value: None,
            error: None,
        };

        let promise_arc = Arc::new(Mutex::new(promise.clone()));
        self.promise_registry.insert(id, promise_arc);

        (id, promise)
    }

    /// Resolve a promise with a value
    pub fn resolve_promise(&mut self, promise_id: PromiseId, value: Value) -> Result<(), String> {
        if let Some(promise_arc) = self.promise_registry.get(&promise_id) {
            let mut promise = promise_arc
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;

            if let Value::Promise {
                state,
                value: promise_value,
                ..
            } = &mut *promise
            {
                match state {
                    PromiseState::Pending => {
                        *state = PromiseState::Resolved;
                        *promise_value = Some(Box::new(value));
                        Ok(())
                    }
                    _ => Err("Promise already resolved or rejected".to_string()),
                }
            } else {
                Err("Invalid promise value".to_string())
            }
        } else {
            Err("Promise not found".to_string())
        }
    }

    /// Reject a promise with an error
    pub fn reject_promise(&mut self, promise_id: PromiseId, error: Value) -> Result<(), String> {
        if let Some(promise_arc) = self.promise_registry.get(&promise_id) {
            let mut promise = promise_arc
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;

            if let Value::Promise {
                state,
                error: promise_error,
                ..
            } = &mut *promise
            {
                match state {
                    PromiseState::Pending => {
                        *state = PromiseState::Rejected;
                        *promise_error = Some(Box::new(error));
                        Ok(())
                    }
                    _ => Err("Promise already resolved or rejected".to_string()),
                }
            } else {
                Err("Invalid promise value".to_string())
            }
        } else {
            Err("Promise not found".to_string())
        }
    }

    /// Schedule a task for execution
    pub fn schedule_task(&mut self, task_type: TaskType, promise_id: Option<PromiseId>) -> TaskId {
        let task_id = self.next_task_id;
        self.next_task_id += 1;

        let task = Task {
            id: task_id,
            task_type,
            promise_id,
            created_at: Instant::now(),
        };

        self.task_queue.push_back(task);
        task_id
    }

    /// Get the next task from the queue
    pub fn next_task(&mut self) -> Option<Task> {
        self.task_queue.pop_front()
    }

    /// Check if there are pending tasks
    pub fn has_pending_tasks(&self) -> bool {
        !self.task_queue.is_empty()
    }

    /// Get promise by ID
    pub fn get_promise(&self, promise_id: PromiseId) -> Option<Arc<Mutex<Value>>> {
        self.promise_registry.get(&promise_id).cloned()
    }

    /// Remove a completed promise from the registry
    pub fn cleanup_promise(&mut self, promise_id: PromiseId) {
        self.promise_registry.remove(&promise_id);
    }

    /// Start the async runtime
    pub fn start(&mut self) {
        self.running = true;
    }

    /// Stop the async runtime
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Check if the runtime is running
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Create a delayed promise (Promise.delay)
    pub fn create_delayed_promise(&mut self, delay_ms: u64, value: Value) -> (PromiseId, Value) {
        let (promise_id, promise) = self.create_promise();

        // Create a delay task
        let delay_task = TaskType::Delay {
            ms: delay_ms,
            value,
        };
        self.schedule_task(delay_task, Some(promise_id));

        (promise_id, promise)
    }

    /// Handle Promise.all - wait for all promises to resolve
    pub fn handle_promise_all(&mut self, _promises: Vec<PromiseId>) -> (PromiseId, Value) {
        let (result_promise_id, result_promise) = self.create_promise();

        // For now, create a simple custom task
        let all_task = TaskType::Custom {
            name: "all".to_string(),
        };
        self.schedule_task(all_task, Some(result_promise_id));

        (result_promise_id, result_promise)
    }

    /// Handle Promise.race - wait for first promise to resolve
    pub fn handle_promise_race(&mut self, _promises: Vec<PromiseId>) -> (PromiseId, Value) {
        let (result_promise_id, result_promise) = self.create_promise();

        // For now, create a simple custom task
        let race_task = TaskType::Custom {
            name: "race".to_string(),
        };
        self.schedule_task(race_task, Some(result_promise_id));

        (result_promise_id, result_promise)
    }

    /// Get runtime statistics
    pub fn stats(&self) -> RuntimeStats {
        RuntimeStats {
            pending_tasks: self.task_queue.len(),
            active_promises: self.promise_registry.len(),
            running: self.running,
        }
    }
}

/// Runtime statistics for monitoring
#[derive(Debug, Clone)]
pub struct RuntimeStats {
    pub pending_tasks: usize,
    pub active_promises: usize,
    pub running: bool,
}

/// Utility functions for promise operations
impl AsyncRuntime {
    /// Create Promise.resolve(value)
    pub fn promise_resolve(&mut self, value: Value) -> Value {
        Value::Promise {
            state: PromiseState::Resolved,
            value: Some(Box::new(value)),
            error: None,
        }
    }

    /// Create Promise.reject(error)
    pub fn promise_reject(&mut self, error: Value) -> Value {
        Value::Promise {
            state: PromiseState::Rejected,
            value: None,
            error: Some(Box::new(error)),
        }
    }

    /// Check if a value is a resolved promise
    pub fn is_resolved_promise(value: &Value) -> bool {
        matches!(
            value,
            Value::Promise {
                state: PromiseState::Resolved,
                ..
            }
        )
    }

    /// Check if a value is a rejected promise
    pub fn is_rejected_promise(value: &Value) -> bool {
        matches!(
            value,
            Value::Promise {
                state: PromiseState::Rejected,
                ..
            }
        )
    }

    /// Check if a value is a pending promise
    pub fn is_pending_promise(value: &Value) -> bool {
        matches!(
            value,
            Value::Promise {
                state: PromiseState::Pending,
                ..
            }
        )
    }

    /// Extract value from resolved promise
    pub fn extract_promise_value(value: &Value) -> Option<&Value> {
        match value {
            Value::Promise {
                state: PromiseState::Resolved,
                value: Some(val),
                ..
            } => Some(val),
            _ => None,
        }
    }

    /// Extract error from rejected promise
    pub fn extract_promise_error(value: &Value) -> Option<&Value> {
        match value {
            Value::Promise {
                state: PromiseState::Rejected,
                error: Some(err),
                ..
            } => Some(err),
            _ => None,
        }
    }
}
