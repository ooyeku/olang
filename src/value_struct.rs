//! The interpreter's struct parts (OL-431): an interned type name, and
//! fields that are either the interpreter's own map or a struct the VM
//! built, kept as it is.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ast::{Value, ValueMap};
use crate::ovm::value::StructObject;

/// A struct's type name: interned, so building or copying a struct
/// never allocates its name (it was a `String` a struct, 24 bytes inline
/// and an allocation each). Compares, hashes and prints as the string.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeName(Arc<str>);

/// Past this many names a thread stops interning (each new name is then
/// an allocation of its own, as it was): a program that makes type names
/// from data cannot grow the table without bound.
const INTERN_MAX: usize = 1 << 14;

thread_local! {
    static NAMES: std::cell::RefCell<rustc_hash::FxHashSet<Arc<str>>> =
        std::cell::RefCell::new(Default::default());
}

impl TypeName {
    pub fn new(name: &str) -> Self {
        let interned = NAMES.try_with(|names| {
            let mut names = names.borrow_mut();
            if let Some(hit) = names.get(name) {
                return hit.clone();
            }
            let fresh: Arc<str> = Arc::from(name);
            if names.len() < INTERN_MAX {
                names.insert(fresh.clone());
            }
            fresh
        });
        TypeName(interned.unwrap_or_else(|_| Arc::from(name)))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for TypeName {
    fn default() -> Self {
        TypeName::new("")
    }
}

impl std::ops::Deref for TypeName {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for TypeName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for TypeName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TypeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, f)
    }
}

impl fmt::Debug for TypeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl From<&str> for TypeName {
    fn from(s: &str) -> Self {
        TypeName::new(s)
    }
}

impl From<String> for TypeName {
    fn from(s: String) -> Self {
        TypeName::new(&s)
    }
}

impl From<&String> for TypeName {
    fn from(s: &String) -> Self {
        TypeName::new(s)
    }
}

impl From<&TypeName> for TypeName {
    fn from(s: &TypeName) -> Self {
        s.clone()
    }
}

impl From<TypeName> for String {
    fn from(t: TypeName) -> String {
        t.0.to_string()
    }
}

impl PartialEq<str> for TypeName {
    fn eq(&self, other: &str) -> bool {
        &*self.0 == other
    }
}

impl PartialEq<&str> for TypeName {
    fn eq(&self, other: &&str) -> bool {
        &*self.0 == *other
    }
}

impl PartialEq<String> for TypeName {
    fn eq(&self, other: &String) -> bool {
        *self.0 == **other
    }
}

impl PartialEq<TypeName> for str {
    fn eq(&self, other: &TypeName) -> bool {
        self == &*other.0
    }
}

impl PartialEq<TypeName> for &str {
    fn eq(&self, other: &TypeName) -> bool {
        *self == &*other.0
    }
}

impl PartialEq<TypeName> for String {
    fn eq(&self, other: &TypeName) -> bool {
        **self == *other.0
    }
}

impl Serialize for TypeName {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TypeName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(TypeName::new(&s))
    }
}

/// A struct's fields. The interpreter's own are a map; a struct the VM
/// built crosses into the interpreter as the VM's object (`Vm`), in O(1)
/// and without copying a field, and crosses back as the same object.
///
/// Every reader sees a `ValueMap` (this derefs to one): for `Vm` it is
/// the map the object builds the first time one is asked for and keeps
/// (see `ovm::struct_object`). `get_value` reads one field without
/// building it. A write (`make_mut`) turns `Vm` into a map of its own,
/// as `Arc::make_mut` copies a shared map.
#[derive(Clone)]
pub enum StructFields {
    Map(Arc<ValueMap>),
    Vm(StructObject),
}

impl StructFields {
    pub fn new(map: ValueMap) -> Self {
        StructFields::Map(Arc::new(map))
    }

    /// The fields as a map the caller may write: this value's own (copied
    /// when shared, built from the VM's object once).
    pub fn make_mut(&mut self) -> &mut ValueMap {
        if let StructFields::Vm(obj) = self {
            *self = StructFields::Map(obj.ast_map_arc());
        }
        match self {
            StructFields::Map(m) => Arc::make_mut(m),
            StructFields::Vm(_) => unreachable!(),
        }
    }

    /// The map, when this value is its only owner (`Arc::get_mut`); never
    /// for the VM's object.
    pub fn get_mut(&mut self) -> Option<&mut ValueMap> {
        match self {
            StructFields::Map(m) => Arc::get_mut(m),
            StructFields::Vm(_) => None,
        }
    }

    pub fn strong_count(&self) -> usize {
        match self {
            StructFields::Map(m) => Arc::strong_count(m),
            StructFields::Vm(obj) => obj.strong_count(),
        }
    }

    /// Whether the two are one set of fields in memory.
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        match (a, b) {
            (StructFields::Map(x), StructFields::Map(y)) => Arc::ptr_eq(x, y),
            (StructFields::Vm(x), StructFields::Vm(y)) => StructObject::ptr_eq(x, y),
            _ => false,
        }
    }

    /// The fields as a shared map (for a map made of a struct's fields).
    pub fn to_arc(&self) -> Arc<ValueMap> {
        match self {
            StructFields::Map(m) => m.clone(),
            StructFields::Vm(obj) => obj.ast_map_arc(),
        }
    }

    /// The fields as a map of the caller's own: moved out when this value
    /// is the only owner, else copied.
    pub fn into_map(self) -> ValueMap {
        match self {
            StructFields::Map(m) => Arc::unwrap_or_clone(m),
            StructFields::Vm(obj) => (*obj.ast_map()).clone(),
        }
    }

    /// The VM's object, when these are its fields.
    pub fn vm(&self) -> Option<&StructObject> {
        match self {
            StructFields::Vm(obj) => Some(obj),
            StructFields::Map(_) => None,
        }
    }

    /// Field `name`, as a value of its own. On the VM's object a scalar
    /// or string field is read where it is, without building the map; a
    /// collection field builds it once (converting the collection per
    /// read would cost its size each time).
    pub fn get_value(&self, name: &str) -> Option<Value> {
        match self {
            StructFields::Map(m) => m.get(name).cloned(),
            StructFields::Vm(obj) => {
                if !obj.has_ast_map() {
                    let field = obj.field_ref(name)?;
                    use crate::ovm::value::ValueData as D;
                    match &field.data {
                        D::Integer(i) => return Some(Value::Integer(*i)),
                        D::Float(f) => return Some(Value::Float(*f)),
                        D::Boolean(b) => return Some(Value::Boolean(*b)),
                        D::Unit => return Some(Value::Unit),
                        D::String(s) => return Some(Value::String(s.clone())),
                        _ => {}
                    }
                }
                obj.ast_map().get(name).cloned()
            }
        }
    }

    /// Whether a field named `name` exists (without building the map).
    pub fn has(&self, name: &str) -> bool {
        match self {
            StructFields::Map(m) => m.contains_key(name),
            StructFields::Vm(obj) => obj.shape.field_index(name).is_some(),
        }
    }
}

impl std::ops::Deref for StructFields {
    type Target = ValueMap;
    #[inline]
    fn deref(&self) -> &ValueMap {
        match self {
            StructFields::Map(m) => m,
            StructFields::Vm(obj) => obj.ast_map(),
        }
    }
}

impl AsRef<ValueMap> for StructFields {
    fn as_ref(&self) -> &ValueMap {
        self
    }
}

impl From<ValueMap> for StructFields {
    fn from(map: ValueMap) -> Self {
        StructFields::new(map)
    }
}

impl From<Arc<ValueMap>> for StructFields {
    fn from(map: Arc<ValueMap>) -> Self {
        StructFields::Map(map)
    }
}

impl PartialEq for StructFields {
    fn eq(&self, other: &Self) -> bool {
        StructFields::ptr_eq(self, other) || **self == **other
    }
}

impl fmt::Debug for StructFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl Serialize for StructFields {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (**self).serialize(s)
    }
}

impl<'de> Deserialize<'de> for StructFields {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(StructFields::new(ValueMap::deserialize(d)?))
    }
}
