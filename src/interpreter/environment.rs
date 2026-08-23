//! The interpreter's scope model: a persistent-map environment with cheap
//! snapshots (the basis of capture-by-value closures), plus the module
//! debugging configuration.

use super::InterpreterError;
use crate::ast::Value;
use im::HashMap as ImHashMap;
use std::collections::HashMap;
use std::sync::Arc;

/// Configuration for module resolution debugging
#[derive(Debug, Clone)]
pub struct ModuleDebugConfig {
    pub enable_resolution_tracing: bool,
    pub log_search_paths: bool,
    pub show_resolution_timing: bool,
    pub verbose_error_messages: bool,
}

impl Default for ModuleDebugConfig {
    fn default() -> Self {
        Self {
            enable_resolution_tracing: std::env::var("OLANG_DEBUG_MODULES").is_ok(),
            log_search_paths: true,
            show_resolution_timing: false,
            verbose_error_messages: true,
        }
    }
}

/// Environment with Arc-based immutable collections for O(1) cloning
/// This is the core optimization - previous version cloned entire HashMap on every scope entry
#[derive(Clone)]
pub struct Environment {
    pub(crate) variables: Arc<ImHashMap<String, Value>>,
    /// Frame bindings (parameters, the function's own name, and — in frame
    /// environments — every runtime binding), probed by string compare
    /// before the map. Hashed HAMT traffic was the dominant interpreter
    /// cost twice over: closure copies per call, then let/assignment
    /// inserts per loop iteration.
    pub(crate) locals: Vec<(String, Value)>,
    /// Whether this is a transient frame (function call, loop body, match
    /// arm, catch block). Frames store new bindings in `locals` — a push
    /// and in-place overwrites — instead of the persistent map. The root
    /// environment is not a frame: top-level definitions go to the map so
    /// they persist and are cheap to snapshot into closures.
    pub(crate) is_frame: bool,
    pub(crate) parent: Option<Arc<Environment>>,
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

impl Environment {
    pub fn new() -> Self {
        Self {
            variables: Arc::new(ImHashMap::new()),
            locals: Vec::new(),
            is_frame: false,
            parent: None,
        }
    }

    /// Create child environment with parent reference - O(1) now!
    pub fn with_parent(parent: Environment) -> Self {
        Self {
            variables: Arc::new(ImHashMap::new()),
            locals: Vec::new(),
            is_frame: true,
            parent: Some(Arc::new(parent)),
        }
    }

    /// Create child environment from Arc parent - even cheaper
    pub fn with_parent_arc(parent: Arc<Environment>) -> Self {
        Self {
            variables: Arc::new(ImHashMap::new()),
            locals: Vec::new(),
            is_frame: true,
            parent: Some(parent),
        }
    }

    /// Define a variable. In a frame, bindings live in the probed locals
    /// vector (in-place overwrite on rebind, e.g. a `let` re-executed each
    /// loop iteration); at the root they go to the persistent map.
    pub fn define(&mut self, name: String, value: Value) {
        // A later binding must shadow an existing frame local of the same name
        if let Some(slot) = self.locals.iter_mut().rev().find(|(n, _)| *n == name) {
            slot.1 = value;
            return;
        }
        if self.is_frame {
            self.locals.push((name, value));
            return;
        }
        Arc::make_mut(&mut self.variables).insert(name, value);
    }

    /// Define a call-frame binding (a parameter or the function's own name)
    /// without touching the shared persistent map.
    pub fn define_local(&mut self, name: String, value: Value) {
        self.locals.push((name, value));
    }

    /// Fetch a resolved slot: hop `depth` parents, verify the slot holds
    /// `name`, and fall back to a normal lookup on any mismatch — static
    /// resolution can be stale (e.g. a conditionally-executed `let` shifted
    /// later slots), and the fallback keeps that a performance event, not a
    /// correctness event.
    pub fn get_slot(&self, name: &str, depth: u16, slot: u16) -> Option<Value> {
        let mut env = self;
        for _ in 0..depth {
            match env.parent.as_deref() {
                Some(parent) => env = parent,
                // The chain is shorter than the resolved depth — a body
                // resolved in one context executing under another (a
                // closure rebuilt across the tier boundary). The
                // verify-and-fall-back contract still holds: resolve by
                // name. The old `?` here returned None instead, which
                // surfaced as a spurious "Undefined variable" the moment
                // the bridge gained a tier and re-ran such bodies.
                None => return self.get(name),
            }
        }
        if let Some((slot_name, value)) = env.locals.get(slot as usize)
            && slot_name == name
        {
            return Some(value.clone());
        }
        self.get(name)
    }

    /// Assign through a resolved slot, with the same verify-and-fall-back
    /// contract as get_slot.
    pub fn set_slot(
        &mut self,
        name: &str,
        depth: u16,
        slot: u16,
        value: Value,
    ) -> Result<(), InterpreterError> {
        // Walk mutably: hop through Arc parents with make_mut
        if depth == 0 {
            if let Some((slot_name, slot_value)) = self.locals.get_mut(slot as usize)
                && slot_name == name
            {
                *slot_value = value;
                return Ok(());
            }
            return self.set(name, value);
        }
        let Some(parent) = self.parent.as_mut() else {
            return self.set(name, value);
        };
        Arc::make_mut(parent).set_slot(name, depth - 1, slot, value)
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some((_, value)) = self.locals.iter().rev().find(|(n, _)| n == name) {
            Some(value.clone())
        } else if let Some(value) = self.variables.get(name) {
            Some(value.clone())
        } else if let Some(parent) = &self.parent {
            parent.get(name)
        } else {
            None
        }
    }

    pub fn set(&mut self, name: &str, value: Value) -> Result<(), InterpreterError> {
        if let Some(slot) = self.locals.iter_mut().rev().find(|(n, _)| n == name) {
            slot.1 = value;
            Ok(())
        } else if self.variables.contains_key(name) {
            Arc::make_mut(&mut self.variables).insert(name.to_string(), value);
            Ok(())
        } else if let Some(parent) = &mut self.parent {
            // Write through to the ancestor that owns the variable; shadowing
            // it locally would silently discard the assignment when this
            // scope is popped (e.g. `x = x + 1` inside a for-loop body)
            Arc::make_mut(parent).set(name, value)
        } else {
            Err(InterpreterError::UndefinedVariable {
                name: name.to_string(),
            })
        }
    }

    /// Extend the `List` bound to `name` in place with `items`, but only
    /// when this environment solely owns both the binding's slot and the
    /// list's `Arc` — the sole-owner fast path behind `xs = xs + [..]`
    /// that makes list accumulation O(n) instead of O(n²) on the
    /// interpreter. Returns `true` if it extended in place; `false` means
    /// the caller must fall back to the ordinary copying concat (the
    /// binding is aliased, not a list, or lives in a shared map). It
    /// never mutates on the `false` path, so any alias — a snapshot, a
    /// nested list, a captured closure — is always left untouched, which
    /// is what keeps this transparent against the interpreter oracle.
    pub fn try_extend_list(&mut self, name: &str, items: &[Value]) -> bool {
        // Frame-locals are a plain Vec: a clean mutable borrow.
        if let Some((_, val)) = self.locals.iter_mut().rev().find(|(n, _)| n == name) {
            return extend_if_sole(val, items);
        }
        // The persistent map: `im`'s `get_mut` path-copies any node another
        // snapshot still shares, and a copied node holds a second Arc to the
        // list — so `extend_if_sole`'s sole-ownership guard refuses exactly
        // when a closure or clone captured the map, and extends in place
        // exactly when nothing did. Top-level accumulation is O(n) too.
        if self.variables.contains_key(name) {
            if let Some(val) = Arc::make_mut(&mut self.variables).get_mut(name) {
                return extend_if_sole(val, items);
            }
            return false;
        }
        // Ancestor scope: recurse only when we solely own the parent Arc
        // (nothing else — a closure, another frame — captured it).
        if let Some(parent) = self.parent.as_mut()
            && let Some(p) = Arc::get_mut(parent)
        {
            return p.try_extend_list(name, items);
        }
        false
    }

    /// Get all variables in this environment (excluding parent environments)
    /// Returns a clone for compatibility with existing code
    pub fn get_all_variables(&self) -> HashMap<String, Value> {
        let mut all: HashMap<String, Value> = self
            .variables
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (name, value) in &self.locals {
            all.insert(name.clone(), value.clone());
        }
        all
    }

    /// This environment's own bindings as a persistent map: the flat map
    /// with call-frame locals overlaid. O(1) when there are no locals.
    pub(crate) fn flat_snapshot(&self) -> ImHashMap<String, Value> {
        let mut snapshot = (*self.variables).clone();
        for (name, value) in &self.locals {
            snapshot.insert(name.clone(), value.clone());
        }
        snapshot
    }

    /// Reserve capacity - no-op for ImHashMap (it grows automatically)
    pub fn reserve(&mut self, _additional: usize) {
        // ImHashMap handles capacity automatically
    }

    /// Remove a variable from this environment (for scoped cleanup)
    pub fn remove_variable(&mut self, name: &str) {
        self.locals.retain(|(n, _)| n != name);
        Arc::make_mut(&mut self.variables).remove(name);
    }

    /// Get ownership of all variables in this environment (for scoped operations)
    pub fn into_variables(self) -> HashMap<String, Value> {
        Arc::try_unwrap(self.variables)
            .unwrap_or_else(|arc| (*arc).clone())
            .into_iter()
            .collect()
    }
}

/// Extend `val` in place iff it is a sole-owner `List`; otherwise leave it
/// untouched and report failure so the caller copies. `Arc::get_mut` is the
/// aliasing guard — it yields `Some` only when this is the single reference
/// to the underlying `Vec`, exactly the discipline the string/list AddAssign
/// fusion uses in the bytecode tier.
impl Environment {
    /// Run `f` over the sole-owned Vec behind `name`'s list binding, in
    /// place. `None` means the binding is missing, is not a list, or is
    /// aliased — the caller then takes the ordinary copy path, so
    /// aliasing degrades to a copy, never to a wrong answer. The
    /// traversal is `try_extend_list`'s; the sole-ownership discipline
    /// is `extend_if_sole`'s. This is the primitive behind the
    /// `xs = col.set(xs, i, v)` fusion, which is what lets the
    /// olang-source collections (heap, table, dsu, ...) write single
    /// elements at O(1).
    /// Take `name`'s value out of its slot, leaving Unit — the caller
    /// side of the move-call fusion (`x = mod.f(x, ...)` passes `x` by
    /// move so the callee owns it solely). `None` when the binding is
    /// missing or its scope is shared (an ancestor Arc another frame
    /// holds): the caller then passes a copy, exactly as before. The
    /// slot is written again by the assignment that follows; a raised
    /// error aborts the program before anything can observe the Unit.
    pub fn take_for_move(&mut self, name: &str) -> Option<Value> {
        if let Some((_, val)) = self.locals.iter_mut().rev().find(|(n, _)| n == name) {
            return Some(std::mem::replace(val, Value::Unit));
        }
        if self.variables.contains_key(name) {
            return Arc::make_mut(&mut self.variables)
                .get_mut(name)
                .map(|val| std::mem::replace(val, Value::Unit));
        }
        if let Some(parent) = self.parent.as_mut()
            && let Some(p) = Arc::get_mut(parent)
        {
            return p.take_for_move(name);
        }
        None
    }

    pub fn try_list_update<R>(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut Vec<Value>) -> R,
    ) -> Option<R> {
        fn sole_vec(val: &mut Value) -> Option<&mut Vec<Value>> {
            if let Value::List(arc) = val {
                return Arc::get_mut(arc);
            }
            None
        }
        if let Some((_, val)) = self.locals.iter_mut().rev().find(|(n, _)| n == name) {
            return sole_vec(val).map(f);
        }
        if self.variables.contains_key(name) {
            if let Some(val) = Arc::make_mut(&mut self.variables).get_mut(name) {
                return sole_vec(val).map(f);
            }
            return None;
        }
        if let Some(parent) = self.parent.as_mut()
            && let Some(p) = Arc::get_mut(parent)
        {
            return p.try_list_update(name, f);
        }
        None
    }
}

fn extend_if_sole(val: &mut Value, items: &[Value]) -> bool {
    if let Value::List(arc) = val
        && let Some(v) = Arc::get_mut(arc)
    {
        v.extend_from_slice(items);
        return true;
    }
    false
}
