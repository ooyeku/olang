//! The cycle collector: frees loops that reference counting cannot (OL-570).
//!
//! Values are reference-counted and freed the moment the last reference
//! goes. Values never change and closures copy what they capture, so
//! plain data cannot form a loop; the mutable places can. A cell holding
//! a list that holds the cell, or a closure stored in the cell it reads,
//! keeps itself alive after the program has let go of it. Every such
//! loop passes through a mutable native (a cell, a shared reference), so
//! those are the only places a search for one has to start.
//!
//! ## How
//!
//! Trial deletion, as CPython's collector does it. From every live cell
//! and shared reference, walk what each holds: every allocation reached
//! becomes a node, with its reference count read once. Each edge found
//! between two nodes is one reference the walk can account for. A node
//! with more references than edges into it is held from outside the walk
//! (a variable, a frame, a task), and so is everything it reaches. What
//! is left is held only by itself: each cell or shared reference there
//! is emptied, and ordinary reference counting frees the rest.
//!
//! Counting is exact only if every edge is a distinct reference. Each
//! node is visited once, so its own slots are distinct. A closure's
//! captured scope is the exception: an `im::HashMap` that shares tree
//! nodes with the scope it was made in, so a value's count counts nodes,
//! not maps. The walk goes through the tree's nodes and their own counts
//! (the vendored im's `trace_tree`, vendor/im/OLANG.md): a node a live
//! scope shares has a reference the walk cannot account for, and keeps
//! what it holds.
//!
//! Anything the walk does not understand is taken as held from outside:
//! a native that does not `trace`, a cell whose lock is busy. That costs
//! a loop through it staying alive, never a live value freed.
//!
//! ## Other threads
//!
//! Tasks run while the walk does, so counts are read at different
//! moments. A thread can only make a new reference to something it
//! already reaches, and that reference was counted when it was read; the
//! walk reads a parent's count before its children's, so taking a child
//! out and dropping the parent mid-walk is still seen as held. To leave
//! no window at all, a loop is emptied only when a second walk, from the
//! loop alone, finds it unreachable again.
//!
//! ## When
//!
//! After `THRESHOLD_MIN` new cells and shared references, or more when
//! the last walk found many still alive or visited many nodes (so a
//! program with a large store pays a bounded share per new cell); also
//! when the heap has doubled since the last walk; and on
//! `runtime.collect()`.

use crate::ast::{EnumVariantData, EnumValue, Function, Value, ValueMap};
use crate::native::{NativeHandle, NativeObject, Traced};
use crate::ovm::struct_object::StructObject;
use crate::ovm::value::{
    ClosureObject, EnumData, EnumObject, FunctionObject, OvmMap, OvmValue, ResultObject, ValueData,
};
use crate::value_struct::StructFields;
use std::cell::RefCell;
use rustc_hash::FxHashMap as HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

type Scope = im::HashMap<String, Value>;
type Root = Weak<Box<dyn NativeObject>>;

/// New cells and shared references between two automatic walks, at least.
const THRESHOLD_MIN: usize = 10_000;
/// How often (in new roots) `track` looks at the heap's size.
const HEAP_CHECK_EVERY: usize = 256;
/// A heap smaller than this never triggers a walk by its growth.
const HEAP_FLOOR: u64 = 64 << 20;

static ROOTS: Mutex<Vec<Root>> = Mutex::new(Vec::new());
static SINCE: AtomicUsize = AtomicUsize::new(0);
static THRESHOLD: AtomicUsize = AtomicUsize::new(THRESHOLD_MIN);
static HEAP_AFTER: AtomicU64 = AtomicU64::new(0);
static COLLECTING: AtomicBool = AtomicBool::new(false);
static TOTALS: Mutex<Totals> = Mutex::new(Totals {
    walks: 0,
    freed: 0,
});

struct Totals {
    walks: u64,
    freed: u64,
}

/// What one collection did.
#[derive(Debug, Clone, Copy, Default)]
pub struct Collection {
    /// Cells and shared references emptied: the loops freed.
    pub freed: usize,
    /// Allocations the first walk visited.
    pub scanned: usize,
    /// Cells and shared references alive when it began.
    pub roots: usize,
    pub millis: f64,
}

/// Every collection so far: (walks, cells and shared references freed).
pub fn totals() -> (u64, u64) {
    let t = TOTALS.lock().unwrap_or_else(|e| e.into_inner());
    (t.walks, t.freed)
}

/// Register a new cell or shared reference: a place a loop can pass
/// through. May run a collection.
pub fn track(handle: &NativeHandle) {
    ROOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Arc::downgrade(&handle.0));
    let since = SINCE.fetch_add(1, Ordering::Relaxed) + 1;
    let due = since >= THRESHOLD.load(Ordering::Relaxed)
        || (since % HEAP_CHECK_EVERY == 0 && heap_doubled());
    if due {
        collect();
    }
}

fn heap_doubled() -> bool {
    let now = crate::memory::heap_bytes();
    now >= HEAP_FLOOR && now >= HEAP_AFTER.load(Ordering::Relaxed).saturating_mul(2)
}

/// Find the loops nothing reaches and free them. Answers `None` when
/// another thread is already collecting.
pub fn collect() -> Option<Collection> {
    if COLLECTING.swap(true, Ordering::Acquire) {
        return None;
    }
    /// Lets the next collection run however this one ends.
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            COLLECTING.store(false, Ordering::Release);
        }
    }
    let _done = Done;
    let start = crate::clock::Instant::now();
    let roots: Vec<Root> = {
        let mut all = ROOTS.lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|w| w.strong_count() > 0);
        all.clone()
    };
    let first = Walk::from_weak(&roots);
    let scanned = first.nodes.len();
    let unreached = first.unreached();
    drop(first);
    let freed = if unreached.is_empty() {
        Vec::new()
    } else {
        // Again, from the loop alone, with nothing of the first walk held.
        let again = Walk::from_handles(unreached);
        let confirmed = again.unreached();
        drop(again);
        confirmed
    };
    for h in &freed {
        h.0.break_cycle();
    }
    let n = freed.len();
    drop(freed);

    let live = roots.iter().filter(|w| w.strong_count() > 0).count();
    drop(roots);
    THRESHOLD.store(
        THRESHOLD_MIN.max(live.saturating_mul(2)).max(scanned / 4),
        Ordering::Relaxed,
    );
    SINCE.store(0, Ordering::Relaxed);
    HEAP_AFTER.store(crate::memory::heap_bytes(), Ordering::Relaxed);
    {
        let mut t = TOTALS.lock().unwrap_or_else(|e| e.into_inner());
        t.walks += 1;
        t.freed += n as u64;
    }
    Some(Collection {
        freed: n,
        scanned,
        roots: live,
        millis: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// An allocation the walk keeps alive while it runs, so no address it
/// has named is reused.
enum Held {
    List(Arc<Vec<Value>>),
    OvmList(Arc<Vec<OvmValue>>),
    Map(Arc<ValueMap>),
    OvmMap(Arc<OvmMap>),
    Function(Arc<Function>),
    OvmFunction(Arc<FunctionObject>),
    Scope(Arc<Scope>),
    Enum(Arc<EnumValue>),
    OvmEnum(Arc<EnumObject>),
    Closure(Arc<ClosureObject>),
    Struct(StructObject),
    Result(Arc<ResultObject>),
    Native(NativeHandle),
    /// A node of a scope's tree, kept alive by the scope; its edges are
    /// recorded when the scope is walked.
    Tree,
}

struct Node {
    held: Held,
    /// References the allocation had when the walk reached it.
    count: usize,
    /// References the walk found from other nodes.
    inside: usize,
    children: Vec<u32>,
    /// Held from outside, whatever the counts say.
    outside: bool,
}

struct Walk {
    index: HashMap<usize, u32>,
    nodes: Vec<Node>,
    todo: Vec<u32>,
}

fn addr<T: ?Sized>(p: *const T) -> usize {
    p as *const u8 as usize
}

impl Walk {
    fn new() -> Walk {
        Walk {
            index: HashMap::default(),
            nodes: Vec::new(),
            todo: Vec::new(),
        }
    }

    fn from_weak(roots: &[Root]) -> Walk {
        let mut w = Walk::new();
        for root in roots {
            if w.index.contains_key(&addr(root.as_ptr())) {
                continue;
            }
            if let Some(strong) = root.upgrade() {
                w.root(NativeHandle(strong));
            }
        }
        w.run();
        w
    }

    fn from_handles(roots: Vec<NativeHandle>) -> Walk {
        let mut w = Walk::new();
        for h in roots {
            if !w.index.contains_key(&addr(Arc::as_ptr(&h.0))) {
                w.root(h);
            }
        }
        w.run();
        w
    }

    /// A root the walk holds already: its own reference is not counted.
    fn root(&mut self, h: NativeHandle) {
        let key = addr(Arc::as_ptr(&h.0));
        let count = Arc::strong_count(&h.0) - 1;
        self.add(key, count, Held::Native(h));
    }

    fn add(&mut self, key: usize, count: usize, held: Held) -> u32 {
        let id = self.nodes.len() as u32;
        let walk = !matches!(held, Held::Tree);
        self.nodes.push(Node {
            held,
            count,
            inside: 0,
            children: Vec::new(),
            outside: false,
        });
        self.index.insert(key, id);
        if walk {
            self.todo.push(id);
        }
        id
    }

    fn link(&mut self, parent: u32, child: u32) {
        self.nodes[parent as usize].children.push(child);
        self.nodes[child as usize].inside += 1;
    }

    /// An edge to an allocation: the count is read before the walk takes
    /// its own reference.
    fn reach(&mut self, parent: u32, key: usize, count: usize, hold: impl FnOnce() -> Held) {
        let child = match self.index.get(&key) {
            Some(&id) => id,
            None => self.add(key, count, hold()),
        };
        self.link(parent, child);
    }

    fn arc<T>(&mut self, parent: u32, a: &Arc<T>, wrap: fn(Arc<T>) -> Held) {
        self.reach(parent, addr(Arc::as_ptr(a)), Arc::strong_count(a), || {
            wrap(a.clone())
        });
    }

    fn value(&mut self, parent: u32, mut v: &Value) {
        loop {
            match v {
                Value::List(a) | Value::Tuple(a) => self.arc(parent, a, Held::List),
                Value::Map(a) => self.arc(parent, a, Held::Map),
                Value::Function(f) => self.arc(parent, f, Held::Function),
                Value::Struct { fields, .. } => match fields {
                    StructFields::Map(a) => self.arc(parent, a, Held::Map),
                    StructFields::Vm(s) => self.structure(parent, s),
                },
                Value::Enum(e) => self.arc(parent, e, Held::Enum),
                Value::Native(h) => self.arc(parent, &h.0, |a| Held::Native(NativeHandle(a))),
                // The box is the value's own: walk on through it.
                Value::Ok(inner) | Value::Err(inner) => {
                    v = &**inner;
                    continue;
                }
                _ => {}
            }
            return;
        }
    }

    fn ovm(&mut self, parent: u32, v: &OvmValue) {
        match &v.data {
            ValueData::List(a) | ValueData::Tuple(a) => self.arc(parent, a, Held::OvmList),
            ValueData::AstList(a) => self.arc(parent, a, Held::List),
            ValueData::AstMap(a) | ValueData::AstStruct(a, _) => self.arc(parent, a, Held::Map),
            ValueData::Function(f) => self.arc(parent, f, Held::OvmFunction),
            ValueData::AstFunction(f) => self.arc(parent, f, Held::Function),
            ValueData::Map(a) => self.arc(parent, a, Held::OvmMap),
            ValueData::Enum(e) => self.arc(parent, e, Held::OvmEnum),
            ValueData::Closure(c) => self.arc(parent, c, Held::Closure),
            ValueData::Struct(s) => self.structure(parent, s),
            ValueData::Result(r) => self.arc(parent, r, Held::Result),
            ValueData::Native(h) => self.arc(parent, &h.0, |a| Held::Native(NativeHandle(a))),
            _ => {}
        }
    }

    fn structure(&mut self, parent: u32, s: &StructObject) {
        self.reach(parent, addr(s.as_ptr()), s.strong_count(), || {
            Held::Struct(s.clone())
        });
    }

    /// A captured scope, through its tree's own nodes and counts.
    fn scope(&mut self, parent: u32, scope: &Arc<Scope>) {
        self.arc(parent, scope, Held::Scope);
    }

    fn run(&mut self) {
        while let Some(id) = self.todo.pop() {
            let held = std::mem::replace(&mut self.nodes[id as usize].held, Held::Tree);
            self.children(id, &held);
            self.nodes[id as usize].held = held;
        }
    }

    fn children(&mut self, id: u32, held: &Held) {
        match held {
            Held::List(a) => a.iter().for_each(|v| self.value(id, v)),
            Held::OvmList(a) => a.iter().for_each(|v| self.ovm(id, v)),
            Held::Map(a) => a.values().for_each(|v| self.value(id, v)),
            Held::OvmMap(a) => a.values().for_each(|v| self.ovm(id, v)),
            Held::Function(f) => self.scope(id, &f.closure),
            Held::OvmFunction(f) => f.closure.values().for_each(|v| self.ovm(id, v)),
            Held::Scope(s) => self.tree(id, s),
            Held::Enum(e) => match &e.variant_data {
                EnumVariantData::Unit => {}
                EnumVariantData::Tuple(vs) => vs.iter().for_each(|v| self.value(id, v)),
                EnumVariantData::Struct(m) => m.values().for_each(|v| self.value(id, v)),
            },
            Held::OvmEnum(e) => match &e.data {
                EnumData::Unit => {}
                EnumData::Tuple(vs) => vs.iter().for_each(|v| self.ovm(id, v)),
                EnumData::Struct(fs) => fs.iter().for_each(|(_, v)| self.ovm(id, v)),
            },
            Held::Closure(c) => {
                c.captured.iter().for_each(|v| self.ovm(id, v));
                self.scope(id, &c.template.closure);
                if let Some(s) = c.ast_closure.get() {
                    self.scope(id, s);
                }
            }
            Held::Struct(s) => {
                for f in s.values() {
                    self.ovm(id, &f);
                }
                if s.has_ast_map() {
                    // The struct holds one reference to its cached map;
                    // `ast_map_arc` adds the walk's before the count is read.
                    let m = s.ast_map_arc();
                    let key = addr(Arc::as_ptr(&m));
                    let count = Arc::strong_count(&m) - 1;
                    self.reach(id, key, count, || Held::Map(m));
                }
            }
            Held::Result(r) => {
                if let Some(v) = &r.ok {
                    self.ovm(id, v);
                }
                if let Some(v) = &r.err {
                    self.ovm(id, v);
                }
            }
            Held::Native(h) => {
                let traced = h.0.trace(&mut |t| match t {
                    Traced::Value(v) => self.value(id, v),
                    Traced::Ovm(v) => self.ovm(id, v),
                });
                if !traced {
                    self.nodes[id as usize].outside = true;
                }
            }
            Held::Tree => {}
        }
    }

    /// A scope's tree: each node of it is a node of the walk, so one a
    /// live scope shares shows a reference from outside.
    fn tree(&mut self, id: u32, scope: &Scope) {
        let walk = RefCell::new(self);
        scope.trace_tree(
            &mut |parent, child, count| {
                let mut w = walk.borrow_mut();
                let from = if parent == 0 { id } else { w.index[&parent] };
                match w.index.get(&child) {
                    Some(&c) => {
                        w.link(from, c);
                        false
                    }
                    None => {
                        let c = w.add(child, count, Held::Tree);
                        w.link(from, c);
                        true
                    }
                }
            },
            &mut |node, v| {
                let mut w = walk.borrow_mut();
                let from = w.index[&node];
                w.value(from, v);
            },
        );
    }

    /// The roots nothing outside the walk reaches.
    fn unreached(&self) -> Vec<NativeHandle> {
        let mut reached = vec![false; self.nodes.len()];
        let mut stack: Vec<u32> = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if n.outside || n.count > n.inside {
                reached[i] = true;
                stack.push(i as u32);
            }
        }
        while let Some(i) = stack.pop() {
            for &c in &self.nodes[i as usize].children {
                if !reached[c as usize] {
                    reached[c as usize] = true;
                    stack.push(c);
                }
            }
        }
        self.nodes
            .iter()
            .zip(reached)
            .filter_map(|(n, r)| match (&n.held, r) {
                (Held::Native(h), false) => Some(h.clone()),
                _ => None,
            })
            .collect()
    }
}
