//! `weak` — weak references.
//!
//! `weak.new(v)` refers to `v` without keeping it alive; `weak.get(w)`
//! answers `v` while something else still holds it, and `()` once it has
//! been freed. A memo table keyed by weak references forgets entries whose
//! values nothing uses any more (OL-467).
//!
//! Only a value with an allocation behind it can be freed: a list, a
//! map, a struct, a tuple, a string, a closure, an enum with a payload,
//! a native value. A scalar (an Int, a Float, a Bool, `()`) is kept as
//! it is, and `weak.get` always answers it.
//!
//! The bytecode VM keeps its own values (`vm_call`): a list or a map the
//! VM built is referred to in the VM's form, so a weak reference made in
//! compiled code does not point at a converted copy that dies at once.

use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use crate::ovm::value::{OvmValue, ValueData};
use std::any::Any;
use std::sync::{Arc, Weak};

/// What a weak reference holds: a weak pointer to one of the allocations
/// a value is, in the interpreter's form or the VM's, or a scalar kept.
#[derive(Debug)]
enum Referent {
    Kept(Value),
    String(Weak<String>),
    List(Weak<Vec<Value>>),
    Tuple(Weak<Vec<Value>>),
    Map(Weak<crate::ast::ValueMap>),
    Struct(String, Weak<crate::ast::ValueMap>),
    Function(Weak<crate::ast::Function>),
    Enum(Weak<crate::ast::EnumValue>),
    Native(Weak<Box<dyn NativeObject>>),
    Vm(VmReferent),
}

/// The VM's data allocations (closures and functions are referred to in
/// the interpreter's form: their bytecode is one VM's own).
#[derive(Debug)]
enum VmReferent {
    String(Weak<String>),
    List(Weak<Vec<OvmValue>>),
    IntList(Weak<Vec<i64>>),
    FloatList(Weak<Vec<f64>>),
    Tuple(Weak<Vec<OvmValue>>),
    Map(Weak<crate::ovm::value::OvmMap>),
    AstList(Weak<Vec<Value>>),
    AstMap(Weak<crate::ast::ValueMap>),
    AstStruct(Weak<crate::ast::ValueMap>, crate::ovm::value::AstRecordKind),
}

#[derive(Debug)]
pub struct WeakObject(Referent);

impl NativeObject for WeakObject {
    fn module(&self) -> &'static str {
        "weak"
    }
    fn type_name(&self) -> &'static str {
        "Weak"
    }
    fn display(&self) -> String {
        if self.alive() {
            "<weak>".to_string()
        } else {
            "<weak: freed>".to_string()
        }
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<WeakObject>()
            .is_some_and(|o| std::ptr::eq(self, o))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl WeakObject {
    fn alive(&self) -> bool {
        match &self.0 {
            Referent::Kept(_) => true,
            Referent::String(w) => w.strong_count() > 0,
            Referent::List(w) | Referent::Tuple(w) => w.strong_count() > 0,
            Referent::Map(w) | Referent::Struct(_, w) => w.strong_count() > 0,
            Referent::Function(w) => w.strong_count() > 0,
            Referent::Enum(w) => w.strong_count() > 0,
            Referent::Native(w) => w.strong_count() > 0,
            Referent::Vm(v) => match v {
                VmReferent::String(w) => w.strong_count() > 0,
                VmReferent::List(w) | VmReferent::Tuple(w) => w.strong_count() > 0,
                VmReferent::IntList(w) => w.strong_count() > 0,
                VmReferent::FloatList(w) => w.strong_count() > 0,
                VmReferent::Map(w) => w.strong_count() > 0,
                VmReferent::AstList(w) => w.strong_count() > 0,
                VmReferent::AstMap(w) | VmReferent::AstStruct(w, _) => w.strong_count() > 0,
            },
        }
    }

    /// The value in the interpreter's form, or `()` once freed.
    fn get_ast(&self) -> Result<Value, String> {
        Ok(match &self.0 {
            Referent::Kept(v) => v.clone(),
            Referent::String(w) => w.upgrade().map(Value::String).unwrap_or(Value::Unit),
            Referent::List(w) => w.upgrade().map(Value::List).unwrap_or(Value::Unit),
            Referent::Tuple(w) => w.upgrade().map(Value::Tuple).unwrap_or(Value::Unit),
            Referent::Map(w) => w.upgrade().map(Value::Map).unwrap_or(Value::Unit),
            Referent::Struct(t, w) => w
                .upgrade()
                .map(|fields| Value::Struct {
                    type_name: t.clone(),
                    fields,
                })
                .unwrap_or(Value::Unit),
            Referent::Function(w) => w.upgrade().map(Value::Function).unwrap_or(Value::Unit),
            Referent::Enum(w) => w.upgrade().map(Value::Enum).unwrap_or(Value::Unit),
            Referent::Native(w) => w
                .upgrade()
                .map(|h| Value::Native(NativeHandle(h)))
                .unwrap_or(Value::Unit),
            Referent::Vm(_) => match self.get_vm() {
                Some(v) => v.to_ast().map_err(|e| format!("weak.get: {e}"))?,
                None => Value::Unit,
            },
        })
    }

    /// The VM's own value, when it was made from one and is alive.
    fn get_vm(&self) -> Option<OvmValue> {
        let Referent::Vm(v) = &self.0 else {
            return None;
        };
        let data = match v {
            VmReferent::String(w) => ValueData::String(w.upgrade()?),
            VmReferent::List(w) => ValueData::List(w.upgrade()?),
            VmReferent::IntList(w) => ValueData::IntList(w.upgrade()?),
            VmReferent::FloatList(w) => ValueData::FloatList(w.upgrade()?),
            VmReferent::Tuple(w) => ValueData::Tuple(w.upgrade()?),
            VmReferent::Map(w) => ValueData::Map(w.upgrade()?),
            VmReferent::AstList(w) => ValueData::AstList(w.upgrade()?),
            VmReferent::AstMap(w) => ValueData::AstMap(w.upgrade()?),
            VmReferent::AstStruct(w, k) => ValueData::AstStruct(w.upgrade()?, *k),
        };
        Some(OvmValue { data })
    }
}

fn referent_of(v: &Value) -> Referent {
    match v {
        Value::String(a) => Referent::String(Arc::downgrade(a)),
        Value::List(a) => Referent::List(Arc::downgrade(a)),
        Value::Tuple(a) => Referent::Tuple(Arc::downgrade(a)),
        Value::Map(a) => Referent::Map(Arc::downgrade(a)),
        Value::Struct { type_name, fields } => {
            Referent::Struct(type_name.clone(), Arc::downgrade(fields))
        }
        Value::Function(a) => Referent::Function(Arc::downgrade(a)),
        // A unit variant is a constant, as a scalar is.
        Value::Enum(a) if !matches!(a.variant_data, crate::ast::EnumVariantData::Unit) => {
            Referent::Enum(Arc::downgrade(a))
        }
        Value::Native(h) => Referent::Native(Arc::downgrade(&h.0)),
        other => Referent::Kept(other.clone()),
    }
}

fn vm_referent_of(v: &OvmValue) -> Option<VmReferent> {
    Some(match &v.data {
        ValueData::String(a) => VmReferent::String(Arc::downgrade(a)),
        ValueData::List(a) => VmReferent::List(Arc::downgrade(a)),
        ValueData::IntList(a) => VmReferent::IntList(Arc::downgrade(a)),
        ValueData::FloatList(a) => VmReferent::FloatList(Arc::downgrade(a)),
        ValueData::Tuple(a) => VmReferent::Tuple(Arc::downgrade(a)),
        ValueData::Map(a) => VmReferent::Map(Arc::downgrade(a)),
        ValueData::AstList(a) => VmReferent::AstList(Arc::downgrade(a)),
        ValueData::AstMap(a) => VmReferent::AstMap(Arc::downgrade(a)),
        ValueData::AstStruct(a, k) => VmReferent::AstStruct(Arc::downgrade(a), *k),
        _ => return None,
    })
}

fn weak_of<'a>(v: &'a Value, who: &str) -> Result<&'a WeakObject, String> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<WeakObject>().ok_or_else(|| {
            format!(
                "{who}: expected a weak reference (from weak.new), got a {}",
                h.0.type_name()
            )
        }),
        other => Err(format!(
            "{who}: expected a weak reference (from weak.new), got {}",
            other.type_name()
        )),
    }
}

pub fn create_weak_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in [("new", 1), ("get", 1), ("alive", 1)] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("weak.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

pub fn call_weak_function(name: &str, args: Vec<Value>) -> Result<Value, String> {
    match (name, args.as_slice()) {
        ("new", [v]) => Ok(Value::Native(NativeHandle::new(WeakObject(referent_of(v))))),
        ("get", [w]) => weak_of(w, "weak.get")?.get_ast(),
        ("alive", [w]) => Ok(Value::Boolean(weak_of(w, "weak.alive")?.alive())),
        ("new" | "get" | "alive", _) => Err(format!(
            "weak.{name} expects one argument, got {}",
            args.len()
        )),
        _ => Err(format!("Unknown weak function: {name}")),
    }
}

/// `weak.new` and `weak.get` on the VM's own values: a value the VM built
/// is referred to as it is, and read back as it is. `None` for anything
/// else; the bridge then runs the call.
pub fn vm_call(op: &str, args: &[OvmValue]) -> Option<Result<OvmValue, String>> {
    match (op, args) {
        ("new", [v]) => {
            let r = vm_referent_of(v)?;
            Some(Ok(OvmValue {
                data: ValueData::Native(NativeHandle::new(WeakObject(Referent::Vm(r)))),
            }))
        }
        (
            "get",
            [
                OvmValue {
                    data: ValueData::Native(h),
                },
            ],
        ) => {
            let w = h.0.as_any().downcast_ref::<WeakObject>()?;
            if !matches!(w.0, Referent::Vm(_)) {
                return None;
            }
            Some(Ok(w.get_vm().unwrap_or_else(OvmValue::new_unit)))
        }
        _ => None,
    }
}
