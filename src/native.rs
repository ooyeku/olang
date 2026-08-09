//! Native extension values and OVM modules.
//!
//! An OVM module is a Rust component that plugs native value types and
//! native functions into *both* execution tiers at once. The contract is
//! deliberately small — a name, some stdlib-style namespaces, a function
//! dispatcher, and first refusal on binary operators that touch the
//! module's values. See `docs/design/ods.md` for the full design.
//!
//! The load-bearing property is [`NativeHandle`]: one `Arc` that the
//! interpreter's `Value::Native` and the OVM's `ValueData::Native` hold
//! in common. Crossing the tier boundary is a refcount bump, never a
//! conversion, so the lossy-round-trip failure mode that once kept maps
//! and enums off the bytecode tier is unrepresentable for native values.

use crate::ast::{BinaryOp, Value};
use once_cell::sync::Lazy;
use std::any::Any;
use std::fmt;
use std::sync::Arc;

/// A value type owned by an OVM module (e.g. an ods array).
///
/// Implementations carry the obligations `Value` already imposes on every
/// variant: a `typeof` name, a display form, structural equality, and a
/// serialized form. `as_any` gives the owning module's kernels their
/// downcast back to the concrete type.
pub trait NativeObject: fmt::Debug + Send + Sync {
    /// The owning module's registered name (e.g. "ods").
    fn module(&self) -> &'static str;

    /// What `typeof` returns for this value.
    fn type_name(&self) -> &'static str;

    /// What `to_string`, `println`, and template interpolation show.
    fn display(&self) -> String;

    /// Structural equality against another native value. Callers pass any
    /// native object; implementations should downcast and return false on
    /// a type mismatch.
    fn native_eq(&self, other: &dyn NativeObject) -> bool;

    /// Downcast support for the owning module's kernels.
    fn as_any(&self) -> &dyn Any;
}

/// Shared handle to a native value: the single allocation both tiers hold.
#[derive(Clone)]
pub struct NativeHandle(pub Arc<dyn NativeObject>);

impl NativeHandle {
    pub fn new(obj: impl NativeObject + 'static) -> Self {
        Self(Arc::new(obj))
    }

    /// Identity, not structural equality — true only for the same allocation.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for NativeHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NativeHandle({:?})", self.0)
    }
}

impl PartialEq for NativeHandle {
    fn eq(&self, other: &Self) -> bool {
        self.0.native_eq(other.0.as_ref())
    }
}

// A native value serializes as an opaque tagged record — the payload lives
// in Rust, not in data, so deserialization is refused rather than silently
// producing a different value.
impl serde::Serialize for NativeHandle {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = serializer.serialize_struct("Native", 3)?;
        st.serialize_field("__native__", self.0.module())?;
        st.serialize_field("type", self.0.type_name())?;
        st.serialize_field("repr", &self.0.display())?;
        st.end()
    }
}

impl<'de> serde::Deserialize<'de> for NativeHandle {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "native values cannot be deserialized",
        ))
    }
}

/// An OVM module: native types plus native functions, plugged into both
/// tiers through one registry.
pub trait OvmModule: Send + Sync {
    /// Stable name; doubles as the namespace prefix (`ods.foo`).
    fn name(&self) -> &'static str;

    /// Stdlib-style namespaces to register — the same shape
    /// `stdlib::get_stdlib` entries have (a "Module" struct of builtins).
    fn namespaces(&self) -> Vec<(String, Value)>;

    /// Called by both tiers' builtin dispatch for `<name>.<func>` calls.
    fn dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, String>;

    /// First refusal on a binary operator with a Native operand of this
    /// module. `None` means "not mine" and falls through to the normal
    /// type-error path — identically in both tiers.
    fn binary_op(&self, op: &BinaryOp, lhs: &Value, rhs: &Value) -> Option<Result<Value, String>>;
}

/// Every compiled-in module. Cargo features decide the set; registration
/// is static so both tiers observe the identical registry with no locks.
#[allow(clippy::vec_init_then_push)] // push sites are cfg-gated per module
pub fn registered_modules() -> &'static [Arc<dyn OvmModule>] {
    static MODULES: Lazy<Vec<Arc<dyn OvmModule>>> = Lazy::new(|| {
        #[allow(unused_mut)]
        let mut modules: Vec<Arc<dyn OvmModule>> = Vec::new();
        #[cfg(feature = "ods")]
        modules.push(Arc::new(crate::ods::OdsModule));
        modules
    });
    &MODULES
}

pub fn module_named(name: &str) -> Option<&'static Arc<dyn OvmModule>> {
    registered_modules().iter().find(|m| m.name() == name)
}

/// Offer a binary operation to the module owning a Native operand.
///
/// Both tiers call this — the interpreter from `eval_binary_op`, the VM
/// from `execute_binary_op` — on the same Arc-shared values, so operator
/// behavior cannot diverge across the tier boundary. Returns `None` when
/// neither operand is native, or when the owning module declines; equality
/// between two natives then falls back to structural `native_eq` so `==`
/// works even for types whose module defines no operators.
pub fn binary_op_hook(op: &BinaryOp, lhs: &Value, rhs: &Value) -> Option<Result<Value, String>> {
    let owner = match (lhs, rhs) {
        (Value::Native(h), _) => h.0.module(),
        (_, Value::Native(h)) => h.0.module(),
        _ => return None,
    };
    if let Some(module) = module_named(owner) {
        if let Some(result) = module.binary_op(op, lhs, rhs) {
            return Some(result);
        }
    }
    if let (Value::Native(a), Value::Native(b)) = (lhs, rhs) {
        match op {
            BinaryOp::Equal => return Some(Ok(Value::Boolean(a == b))),
            BinaryOp::NotEqual => return Some(Ok(Value::Boolean(a != b))),
            _ => {}
        }
    }
    None
}
