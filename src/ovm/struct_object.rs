//! The VM's struct: one allocation holding a header, a word per field and
//! a kind byte per field (OL-431).
//!
//! A struct used to be an `Arc<StructObject>` (a 48-byte allocation: the
//! counts, the shape, a `Vec`) whose `Vec<OvmValue>` was a second
//! allocation of 16 bytes a field. Five Int fields cost 128 bytes in two
//! allocations. Here the same struct is one allocation of 24 + 9 bytes a
//! field (69 for five): a scalar field (Int, Float, Bool, `()`) is its
//! raw 8 bytes, and any other field is the one pointer its `OvmValue`
//! holds (an `Arc` turned raw), so no field kind costs an allocation of
//! its own. The kind byte says which.
//!
//! The handle is one pointer, so `OvmValue` stays 16 bytes. It counts
//! strong references only: nothing takes a weak reference to a struct
//! (it crosses into the interpreter wrapped, so the conversion cache that
//! held weak handles never needs one).
//!
//! The header also holds the struct's interpreter form, built the first
//! time an interpreter consumer needs a map of the fields
//! (`crate::ast::StructFields::Vm` derefs to it), and kept for the
//! struct's life. A struct is immutable while shared; the one in-place
//! write (freeing the fields of a sole owner, `clear_fields`) drops it.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::mem::ManuallyDrop;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering, fence};

use super::value::{AstRecordKind, OvmValue, StructShape, ValueData, intern_shape};
use crate::ast::{Value, ValueMap};

/// What a field's word holds.
pub mod kind {
    pub const INT: u8 = 0;
    pub const FLOAT: u8 = 1;
    pub const BOOL: u8 = 2;
    pub const UNIT: u8 = 3;
    pub const STRING: u8 = 4;
    pub const LIST: u8 = 5;
    pub const FLOAT_LIST: u8 = 6;
    pub const INT_LIST: u8 = 7;
    pub const AST_LIST: u8 = 8;
    pub const AST_MAP: u8 = 9;
    pub const AST_STRUCT_JSON: u8 = 10;
    pub const AST_STRUCT_OBJECT: u8 = 11;
    pub const TUPLE: u8 = 12;
    pub const FUNCTION: u8 = 13;
    pub const AST_FUNCTION: u8 = 14;
    pub const MAP: u8 = 15;
    pub const ENUM: u8 = 16;
    pub const CLOSURE: u8 = 17;
    pub const STRUCT: u8 = 18;
    pub const RANGE: u8 = 19;
    pub const BUILTIN: u8 = 20;
    pub const ERROR: u8 = 21;
    pub const RESULT: u8 = 22;
    pub const NATIVE: u8 = 23;

    /// Whether a field of this kind is its raw bits (owns nothing).
    #[inline]
    pub fn is_scalar(k: u8) -> bool {
        k <= UNIT
    }
}

/// The fixed part of a struct's allocation; the fields follow it.
#[repr(C)]
pub struct StructHeader {
    rc: AtomicU32,
    len: u32,
    pub shape: Arc<StructShape>,
    /// The interpreter's map of the fields (`Arc::into_raw`), or null
    /// until something needs it.
    ast: AtomicPtr<ValueMap>,
}

const HEADER: usize = std::mem::size_of::<StructHeader>();

/// A VM struct: a counted pointer to one allocation (see the module).
pub struct StructObject {
    ptr: NonNull<StructHeader>,
}

// The fields are `OvmValue`s in another form, and the cached map is
// `Value`s: the struct is as thread-safe as they are.
unsafe impl Send for StructObject {}
unsafe impl Sync for StructObject {}
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OvmValue>();
    assert_send_sync::<Value>();
    assert!(HEADER == 24);
};

#[inline]
fn layout(len: usize) -> Layout {
    Layout::from_size_align(HEADER + len * 9, 8).expect("struct layout")
}

/// A field as its kind and word, taking the value's ownership.
#[inline]
fn encode(v: OvmValue) -> (u8, u64) {
    use kind as K;
    match v.into_data() {
        ValueData::Integer(i) => (K::INT, i as u64),
        ValueData::Float(f) => (K::FLOAT, f.to_bits()),
        ValueData::Boolean(b) => (K::BOOL, b as u64),
        ValueData::Unit => (K::UNIT, 0),
        ValueData::String(a) => (K::STRING, Arc::into_raw(a) as u64),
        ValueData::List(a) => (K::LIST, Arc::into_raw(a) as u64),
        ValueData::FloatList(a) => (K::FLOAT_LIST, Arc::into_raw(a) as u64),
        ValueData::IntList(a) => (K::INT_LIST, Arc::into_raw(a) as u64),
        ValueData::AstList(a) => (K::AST_LIST, Arc::into_raw(a) as u64),
        ValueData::AstMap(a) => (K::AST_MAP, Arc::into_raw(a) as u64),
        ValueData::AstStruct(a, AstRecordKind::Json) => {
            (K::AST_STRUCT_JSON, Arc::into_raw(a) as u64)
        }
        ValueData::AstStruct(a, AstRecordKind::Object) => {
            (K::AST_STRUCT_OBJECT, Arc::into_raw(a) as u64)
        }
        ValueData::Tuple(a) => (K::TUPLE, Arc::into_raw(a) as u64),
        ValueData::Function(a) => (K::FUNCTION, Arc::into_raw(a) as u64),
        ValueData::AstFunction(a) => (K::AST_FUNCTION, Arc::into_raw(a) as u64),
        ValueData::Map(a) => (K::MAP, Arc::into_raw(a) as u64),
        ValueData::Enum(a) => (K::ENUM, Arc::into_raw(a) as u64),
        ValueData::Closure(a) => (K::CLOSURE, Arc::into_raw(a) as u64),
        ValueData::Struct(s) => (K::STRUCT, s.into_raw() as u64),
        ValueData::Range(a) => (K::RANGE, Arc::into_raw(a) as u64),
        ValueData::Builtin(a) => (K::BUILTIN, Arc::into_raw(a) as u64),
        ValueData::Error(a) => (K::ERROR, Arc::into_raw(a) as u64),
        ValueData::Result(a) => (K::RESULT, Arc::into_raw(a) as u64),
        ValueData::Native(h) => (K::NATIVE, Arc::into_raw(h.0) as u64),
    }
}

/// The value a field's kind and word encode, owning what the word owns.
///
/// # Safety
/// `(k, bits)` came from `encode` and its ownership is handed over here
/// exactly once (or the result is never dropped: `peek`).
#[inline]
unsafe fn decode(k: u8, bits: u64) -> OvmValue {
    use kind as K;
    let data = unsafe {
        match k {
            K::INT => ValueData::Integer(bits as i64),
            K::FLOAT => ValueData::Float(f64::from_bits(bits)),
            K::BOOL => ValueData::Boolean(bits != 0),
            K::UNIT => ValueData::Unit,
            K::STRING => ValueData::String(Arc::from_raw(bits as *const _)),
            K::LIST => ValueData::List(Arc::from_raw(bits as *const _)),
            K::FLOAT_LIST => ValueData::FloatList(Arc::from_raw(bits as *const _)),
            K::INT_LIST => ValueData::IntList(Arc::from_raw(bits as *const _)),
            K::AST_LIST => ValueData::AstList(Arc::from_raw(bits as *const _)),
            K::AST_MAP => ValueData::AstMap(Arc::from_raw(bits as *const _)),
            K::AST_STRUCT_JSON => {
                ValueData::AstStruct(Arc::from_raw(bits as *const _), AstRecordKind::Json)
            }
            K::AST_STRUCT_OBJECT => {
                ValueData::AstStruct(Arc::from_raw(bits as *const _), AstRecordKind::Object)
            }
            K::TUPLE => ValueData::Tuple(Arc::from_raw(bits as *const _)),
            K::FUNCTION => ValueData::Function(Arc::from_raw(bits as *const _)),
            K::AST_FUNCTION => ValueData::AstFunction(Arc::from_raw(bits as *const _)),
            K::MAP => ValueData::Map(Arc::from_raw(bits as *const _)),
            K::ENUM => ValueData::Enum(Arc::from_raw(bits as *const _)),
            K::CLOSURE => ValueData::Closure(Arc::from_raw(bits as *const _)),
            K::STRUCT => ValueData::Struct(StructObject::from_raw(bits as *const StructHeader)),
            K::RANGE => ValueData::Range(Arc::from_raw(bits as *const _)),
            K::BUILTIN => ValueData::Builtin(Arc::from_raw(bits as *const _)),
            K::ERROR => ValueData::Error(Arc::from_raw(bits as *const _)),
            K::RESULT => ValueData::Result(Arc::from_raw(bits as *const _)),
            K::NATIVE => {
                ValueData::Native(crate::native::NativeHandle(Arc::from_raw(bits as *const _)))
            }
            _ => unreachable!("struct field kind {k}"),
        }
    };
    OvmValue { data }
}

/// A field read without taking a reference count: derefs to the
/// `OvmValue`, and is never dropped as one (the struct still owns it).
pub struct FieldRef<'a> {
    value: ManuallyDrop<OvmValue>,
    _owner: std::marker::PhantomData<&'a StructObject>,
}

impl std::ops::Deref for FieldRef<'_> {
    type Target = OvmValue;
    #[inline]
    fn deref(&self) -> &OvmValue {
        &self.value
    }
}

impl std::fmt::Debug for FieldRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&*self.value, f)
    }
}

impl StructObject {
    #[inline]
    fn header(&self) -> &StructHeader {
        unsafe { self.ptr.as_ref() }
    }

    #[inline]
    fn words(&self) -> *mut u64 {
        unsafe { (self.ptr.as_ptr() as *mut u8).add(HEADER) as *mut u64 }
    }

    #[inline]
    fn kinds(&self) -> *mut u8 {
        unsafe { (self.ptr.as_ptr() as *mut u8).add(HEADER + self.len() * 8) }
    }

    /// Allocate a struct of `shape` with `len` fields, each written by
    /// `fill(i)` as a kind and word that the struct then owns.
    fn build(
        shape: Arc<StructShape>,
        len: usize,
        mut fill: impl FnMut(usize) -> (u8, u64),
    ) -> Self {
        assert!(len <= u32::MAX as usize, "a struct of {len} fields");
        let layout = layout(len);
        let raw = unsafe { alloc(layout) };
        let Some(ptr) = NonNull::new(raw as *mut StructHeader) else {
            handle_alloc_error(layout)
        };
        unsafe {
            ptr.as_ptr().write(StructHeader {
                rc: AtomicU32::new(1),
                len: len as u32,
                shape,
                ast: AtomicPtr::new(std::ptr::null_mut()),
            });
        }
        let obj = StructObject { ptr };
        let (words, kinds) = (obj.words(), obj.kinds());
        for i in 0..len {
            // Each slot is written before the next fill runs, and a fill
            // that panics leaves the rest unwritten: set them to `()` so
            // the drop that unwinds through `obj` sees valid fields.
            unsafe {
                *kinds.add(i) = kind::UNIT;
            }
        }
        for i in 0..len {
            let (k, bits) = fill(i);
            unsafe {
                *words.add(i) = bits;
                *kinds.add(i) = k;
            }
        }
        obj
    }

    /// A struct of `shape` whose fields are `values`, in the shape's
    /// order (missing ones `()`).
    pub fn new(shape: Arc<StructShape>, values: Vec<OvmValue>) -> Self {
        let len = shape.field_names.len().max(values.len());
        let mut values = values.into_iter();
        Self::build(shape, len, |_| {
            values.next().map(encode).unwrap_or((kind::UNIT, 0))
        })
    }

    /// A struct of `shape` with `len` fields, field `i` being `value(i)`.
    pub fn from_fn(
        shape: Arc<StructShape>,
        len: usize,
        mut value: impl FnMut(usize) -> OvmValue,
    ) -> Self {
        Self::build(shape, len, |i| encode(value(i)))
    }

    /// A struct from raw scalar bits: `kind_of(i)` is a scalar kind code
    /// (`kind::INT`, `kind::FLOAT`, `kind::BOOL`). For the JIT's
    /// MakeStruct, which builds the fields without boxing them.
    pub fn from_scalar_bits(
        shape: Arc<StructShape>,
        bits: &[i64],
        kind_of: impl Fn(usize) -> u8,
    ) -> Self {
        Self::build(shape, bits.len(), |i| {
            let k = kind_of(i);
            debug_assert!(kind::is_scalar(k));
            let b = bits[i] as u64;
            (k, if k == kind::BOOL { (b != 0) as u64 } else { b })
        })
    }

    /// Build from unordered (name, value) pairs, interning the shape.
    pub fn from_pairs(type_name: &str, pairs: Vec<(String, OvmValue)>) -> Self {
        let shape = intern_shape(type_name, pairs.iter().map(|(n, _)| n.clone()).collect());
        let mut values: Vec<Option<OvmValue>> =
            (0..shape.field_names.len()).map(|_| None).collect();
        for (name, value) in pairs {
            if let Some(i) = shape.field_index(&name) {
                values[i as usize] = Some(value);
            }
        }
        let mut values = values.into_iter();
        let len = shape.field_names.len();
        Self::build(shape, len, |_| {
            values
                .next()
                .flatten()
                .map(encode)
                .unwrap_or((kind::UNIT, 0))
        })
    }

    /// The same fields under another shape of as many fields (a struct
    /// renamed: the interpreter set another type name on its fields).
    pub fn with_shape(&self, shape: Arc<StructShape>) -> Self {
        debug_assert_eq!(shape.field_names.len(), self.len());
        Self::build(shape, self.len(), |i| encode(self.get(i)))
    }

    /// How many fields.
    #[inline]
    pub fn len(&self) -> usize {
        self.header().len as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn type_name(&self) -> &str {
        &self.shape.type_name
    }

    /// Field `i`'s kind code and raw word. The word of a non-scalar kind
    /// is a pointer the struct owns: borrow it, never free it.
    #[inline]
    pub fn raw(&self, i: usize) -> (u8, u64) {
        assert!(i < self.len());
        unsafe { (*self.kinds().add(i), *self.words().add(i)) }
    }

    /// Field `i`, borrowed.
    #[inline]
    pub fn peek(&self, i: usize) -> FieldRef<'_> {
        let (k, bits) = self.raw(i);
        FieldRef {
            value: ManuallyDrop::new(unsafe { decode(k, bits) }),
            _owner: std::marker::PhantomData,
        }
    }

    /// Field `i`, as a value of its own (a reference count, no copy).
    #[inline]
    pub fn get(&self, i: usize) -> OvmValue {
        (*self.peek(i)).clone()
    }

    /// The field named `name`, as a value of its own.
    #[inline]
    pub fn field(&self, name: &str) -> Option<OvmValue> {
        self.shape
            .field_index(name)
            .filter(|&i| (i as usize) < self.len())
            .map(|i| self.get(i as usize))
    }

    /// The field named `name`, borrowed.
    #[inline]
    pub fn field_ref(&self, name: &str) -> Option<FieldRef<'_>> {
        self.shape
            .field_index(name)
            .filter(|&i| (i as usize) < self.len())
            .map(|i| self.peek(i as usize))
    }

    /// The fields in order, borrowed.
    pub fn values(&self) -> impl ExactSizeIterator<Item = FieldRef<'_>> + '_ {
        (0..self.len()).map(move |i| self.peek(i))
    }

    /// (name, field) in order, the fields borrowed.
    pub fn iter(&self) -> impl Iterator<Item = (&String, FieldRef<'_>)> + '_ {
        self.shape.field_names.iter().zip(self.values())
    }

    /// Whether any field owns something (is not a scalar).
    pub fn has_nested(&self) -> bool {
        (0..self.len()).any(|i| !kind::is_scalar(unsafe { *self.kinds().add(i) }))
    }

    /// The fields with `key` set to `value`: in its place when the record
    /// has it (as a map keeps a key set again where it was), else last.
    pub fn set_pairs(&self, key: String, value: OvmValue) -> Vec<(String, OvmValue)> {
        let mut value = Some(value);
        let mut pairs: Vec<(String, OvmValue)> = self
            .iter()
            .map(|(name, v)| {
                if *name == key {
                    (name.clone(), value.take().unwrap_or_else(|| (*v).clone()))
                } else {
                    (name.clone(), (*v).clone())
                }
            })
            .collect();
        if let Some(v) = value {
            pairs.push((key, v));
        }
        pairs
    }

    /// Whether `a` and `b` are one struct.
    #[inline]
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        a.ptr == b.ptr
    }

    /// The allocation's address: what the JIT passes as a borrowed struct
    /// (`borrow_raw` reads it back).
    #[inline]
    pub fn as_ptr(&self) -> *const StructHeader {
        self.ptr.as_ptr()
    }

    #[inline]
    pub fn strong_count(&self) -> usize {
        self.header().rc.load(Ordering::Acquire) as usize
    }

    /// Whether this handle is the only one.
    #[inline]
    pub fn is_unique(&self) -> bool {
        self.strong_count() == 1
    }

    /// The handle as a raw pointer that owns its count (`from_raw` takes
    /// it back).
    #[inline]
    pub fn into_raw(self) -> *const StructHeader {
        let p = self.ptr.as_ptr();
        std::mem::forget(self);
        p
    }

    /// # Safety
    /// `p` came from `into_raw`, and its count is handed over here once.
    #[inline]
    pub unsafe fn from_raw(p: *const StructHeader) -> Self {
        StructObject {
            ptr: unsafe { NonNull::new_unchecked(p as *mut StructHeader) },
        }
    }

    /// A struct borrowed from its address, without a count.
    ///
    /// # Safety
    /// `p` is the address of a live struct (`as_ptr`) that outlives the
    /// borrow.
    #[inline]
    pub unsafe fn borrow_raw(p: *const StructHeader) -> ManuallyDrop<StructObject> {
        ManuallyDrop::new(unsafe { Self::from_raw(p) })
    }

    /// Field `idx` of the struct at `p`, as its kind and word (the JIT's
    /// field read), or None out of range.
    ///
    /// # Safety
    /// `p` is the address of a live struct.
    #[inline]
    pub unsafe fn raw_at(p: *const StructHeader, idx: usize) -> Option<(u8, u64)> {
        unsafe {
            let len = (*p).len as usize;
            if idx >= len {
                return None;
            }
            let base = p as *const u8;
            let bits = *(base.add(HEADER) as *const u64).add(idx);
            let k = *base.add(HEADER + len * 8 + idx);
            Some((k, bits))
        }
    }

    /// Frees the fields of a sole owner in place (each through its own
    /// drop), leaving `()`s: the bounded free's step for a struct.
    pub fn clear_fields(&mut self) {
        if !self.is_unique() {
            return;
        }
        self.drop_ast();
        let (words, kinds) = (self.words(), self.kinds());
        for i in 0..self.len() {
            unsafe {
                let k = *kinds.add(i);
                if !kind::is_scalar(k) {
                    *kinds.add(i) = kind::UNIT;
                    drop(decode(k, *words.add(i)));
                }
            }
        }
    }

    fn drop_ast(&mut self) {
        let p = self
            .header()
            .ast
            .swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !p.is_null() {
            drop(unsafe { Arc::from_raw(p as *const ValueMap) });
        }
    }

    /// Whether the interpreter's map of the fields is built.
    #[inline]
    pub fn has_ast_map(&self) -> bool {
        !self.header().ast.load(Ordering::Acquire).is_null()
    }

    /// The interpreter's map of the fields: built on the first call (each
    /// field converted by `to_ast`), the same map after.
    pub fn ast_map(&self) -> &ValueMap {
        let p = self.ast_map_ptr();
        unsafe { &*p }
    }

    /// The interpreter's map of the fields, as an `Arc` of its own.
    pub fn ast_map_arc(&self) -> Arc<ValueMap> {
        let p = self.ast_map_ptr();
        unsafe {
            Arc::increment_strong_count(p);
            Arc::from_raw(p)
        }
    }

    fn ast_map_ptr(&self) -> *const ValueMap {
        let slot = &self.header().ast;
        let p = slot.load(Ordering::Acquire);
        if !p.is_null() {
            return p;
        }
        let mut map = ValueMap::default();
        map.reserve(self.len());
        for (name, v) in self.iter() {
            map.insert(name.clone(), v.to_ast().unwrap_or(Value::Unit));
        }
        let fresh = Arc::into_raw(Arc::new(map)) as *mut ValueMap;
        match slot.compare_exchange(
            std::ptr::null_mut(),
            fresh,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => fresh,
            Err(won) => {
                drop(unsafe { Arc::from_raw(fresh as *const ValueMap) });
                won
            }
        }
    }

    #[cold]
    unsafe fn drop_slow(&mut self) {
        self.drop_ast();
        let len = self.len();
        let (words, kinds) = (self.words(), self.kinds());
        for i in 0..len {
            unsafe {
                let k = *kinds.add(i);
                if !kind::is_scalar(k) {
                    drop(decode(k, *words.add(i)));
                }
            }
        }
        unsafe {
            std::ptr::drop_in_place(&mut (*self.ptr.as_ptr()).shape);
            dealloc(self.ptr.as_ptr() as *mut u8, layout(len));
        }
    }
}

impl std::ops::Deref for StructObject {
    type Target = StructHeader;
    #[inline]
    fn deref(&self) -> &StructHeader {
        self.header()
    }
}

impl Clone for StructObject {
    #[inline]
    fn clone(&self) -> Self {
        let old = self.header().rc.fetch_add(1, Ordering::Relaxed);
        if old > (u32::MAX >> 1) {
            std::process::abort();
        }
        StructObject { ptr: self.ptr }
    }
}

impl Drop for StructObject {
    #[inline]
    fn drop(&mut self) {
        if self.header().rc.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        unsafe { self.drop_slow() }
    }
}

impl std::fmt::Debug for StructObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("StructObject");
        s.field("type", &self.type_name());
        for (name, v) in self.iter() {
            s.field(name, &*v);
        }
        s.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_scalar_fields_are_one_allocation_of_69_bytes() {
        assert_eq!(layout(5).size(), 69);
        assert_eq!(std::mem::size_of::<StructObject>(), 8);
    }

    #[test]
    fn fields_round_trip_and_free() {
        let s = Arc::new("hi".to_string());
        let obj = StructObject::from_pairs(
            "T",
            vec![
                ("a".into(), OvmValue::new_integer(-3)),
                ("b".into(), OvmValue::new_float(1.5)),
                ("c".into(), OvmValue::new_boolean(true)),
                ("d".into(), OvmValue::new_unit()),
                (
                    "e".into(),
                    OvmValue {
                        data: ValueData::String(s.clone()),
                    },
                ),
            ],
        );
        assert_eq!(Arc::strong_count(&s), 2);
        assert!(matches!(
            obj.field("a").unwrap().data,
            ValueData::Integer(-3)
        ));
        assert!(matches!(obj.field("b").unwrap().data, ValueData::Float(f) if f == 1.5));
        assert!(matches!(
            obj.field("c").unwrap().data,
            ValueData::Boolean(true)
        ));
        assert!(matches!(obj.field("d").unwrap().data, ValueData::Unit));
        let e = obj.field("e").unwrap();
        assert_eq!(Arc::strong_count(&s), 3);
        drop(e);
        let copy = obj.clone();
        assert_eq!(obj.strong_count(), 2);
        let m = copy.ast_map_arc();
        assert_eq!(m.len(), 5);
        drop(copy);
        drop(obj);
        // the cached map still holds one
        assert_eq!(Arc::strong_count(&s), 2);
        drop(m);
        assert_eq!(Arc::strong_count(&s), 1);
    }
}
