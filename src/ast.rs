use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use std::sync::Arc;

/// Represents a complete Olang program
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

/// Statement types in Olang
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Statement {
    Expression(Expr),
    LetDecl(LetDecl),
    FunctionDecl(FunctionDecl),
    AsyncFunctionDecl(AsyncFunctionDecl),
    TypeDecl(TypeDecl),
    ErrorTypeDecl(ErrorTypeDecl),
    ShareDecl(ShareDecl),
    UseDecl(UseDecl),
    TestDecl(TestDecl),
    TraitDecl(TraitDecl),
    ImplDecl(ImplDecl),
}

/// A trait declaration: a named set of methods, each optionally with a
/// default body. Dispatch is on the runtime type of the receiver (`self`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraitDecl {
    pub name: String,
    pub methods: Vec<TraitMethod>,
}

/// A method signature in a trait, with an optional default implementation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraitMethod {
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub default_body: Option<Expr>,
}

/// An `impl Trait for Type { ... }` block providing method bodies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImplDecl {
    pub trait_name: String,
    pub type_name: String,
    pub methods: Vec<FunctionDecl>,
}

/// Error type declaration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorTypeDecl {
    pub name: String,
    pub variants: Vec<ErrorVariant>,
}

/// One variant of an `error` declaration: a bare name (`NotFound`) or a name
/// with a payload (`Invalid: { msg: String }`), whose fields become the
/// constructor's positional parameters in declaration order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorVariant {
    pub name: String,
    pub fields: Vec<StructField>,
}

/// Variable declaration with optional type annotation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LetDecl {
    pub pattern: Pattern,
    pub type_annotation: Option<TypeAnnotation>,
    pub value: Option<Expr>,
    /// 1-based (line, column) of the binding's first name, when parsed
    /// from source. Desugared/synthetic declarations carry None.
    #[serde(default)]
    pub name_span: Option<(u32, u32)>,
}

/// Function declaration for named/recursive functions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionDecl {
    pub name: String,
    /// 1-based (line, column) of the function name in source.
    #[serde(default)]
    pub name_span: Option<(u32, u32)>,
    pub type_params: Vec<String>, // Type parameters for generic functions
    /// Trait bounds per type parameter: (param name, required trait names).
    /// From `<T: Show + Ord>`. Enforced at runtime against argument types.
    #[serde(default)]
    pub type_param_bounds: Vec<(String, Vec<String>)>,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub body: Expr,
}

/// Async function declaration for named/recursive async functions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AsyncFunctionDecl {
    pub name: String,
    pub type_params: Vec<String>, // Type parameters for generic functions
    pub parameters: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>, // Should be Promise<T, E>
    pub body: Expr,
}

/// Function parameter with optional type annotation and default value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub type_annotation: Option<TypeAnnotation>,
    pub default_value: Option<Expr>,
}

/// Expression types supported by Olang
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Expr {
    // Literals
    Integer(i64),
    Float(f64),
    String(Arc<String>),
    Boolean(bool),
    List(Arc<[Expr]>),
    Tuple(Arc<Vec<Expr>>),

    // Variables and calls
    Identifier(String),
    /// An identifier resolved to a frame slot at function-declaration time.
    /// `depth` counts environment frames outward from the innermost; the
    /// name is kept so the runtime can verify the slot holds the expected
    /// binding and fall back to a name lookup when it doesn't — resolution
    /// can therefore never change semantics, only speed.
    LocalRef {
        name: String,
        depth: u16,
        slot: u16,
    },
    /// An assignment target resolved to a frame slot, same contract as
    /// LocalRef.
    LocalAssign {
        name: String,
        depth: u16,
        slot: u16,
        value: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        arguments: Vec<Argument>,
    },

    // Functions
    Lambda {
        parameters: Vec<Parameter>,
        body: Box<Expr>,
        return_type: Option<TypeAnnotation>,
    },

    // Pipeline operator
    Pipeline {
        left: Box<Expr>,
        right: Box<Expr>,
    },

    // Pattern matching
    Match {
        value: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    // Control flow
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },

    // Range expressions
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
    },

    // Blocks
    Block(Vec<Statement>),

    // Binary operations
    BinaryOp {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },

    // Unary operations
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
    },

    // Custom types
    StructLiteral(StructLiteral),
    AnonymousObject {
        fields: Vec<FieldValue>,
    },
    MapLiteral {
        entries: Vec<MapEntry>,
    },
    FieldAccess {
        object: Box<Expr>,
        field: String,
    },

    // Error handling
    ResultOk(Box<Expr>),
    ResultErr(Box<Expr>),
    Try(Box<Expr>),
    TryCatch {
        try_block: Box<Expr>,
        catch_var: String,
        catch_block: Box<Expr>,
    },

    // Loop constructs
    ForLoop {
        variable: String,
        iterable: Box<Expr>,
        body: Box<Expr>,
    },
    /// `par for x in xs { ... }` — iterations fan across worker threads
    /// with spawn-style snapshot semantics and a barrier at the end.
    /// Interpreter-only: the bytecode compiler refuses it (fail-closed).
    ParForLoop {
        variable: String,
        iterable: Box<Expr>,
        body: Box<Expr>,
    },
    WhileLoop {
        condition: Box<Expr>,
        body: Box<Expr>,
    },
    Loop {
        body: Box<Expr>,
    },
    /// `break` / `break value` — exits the nearest loop; with a value, the
    /// loop expression evaluates to it.
    Break(Option<Box<Expr>>),
    Continue,
    /// `return` / `return value` — exits the nearest function with the value
    /// (Unit when bare).
    Return(Option<Box<Expr>>),

    Assignment {
        target: String,
        value: Box<Expr>,
    },

    // Array/List indexing
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },

    // Async expressions
    Async {
        parameters: Vec<Parameter>,
        body: Box<Expr>,
        return_type: Option<TypeAnnotation>,
    },
    Await {
        expression: Box<Expr>,
    },
    Promise {
        promise_type: PromiseType,
        value: Box<Expr>,
        delay: Option<Box<Expr>>, // For Promise.delay(ms, value)
    },

    // Concurrent operations
    All(Box<Expr>),   // Promise.all(list-expr)
    Race(Box<Expr>),  // Promise.race(list-expr)
    Spawn(Box<Expr>), // spawn async_expr

    // New string literal variants
    RawString(Arc<String>),
    TemplateString {
        parts: Vec<TemplatePart>,
    },

    // Bitwise operations
    BitwiseOp {
        left: Box<Expr>,
        op: BitwiseOp,
        right: Box<Expr>,
    },

    // Spread/rest (for future use)
    Spread(Box<Expr>),
    Rest(Box<Expr>),

    // Test assertions
    AssertEq {
        actual: Box<Expr>,
        expected: Box<Expr>,
        message: Option<String>,
    },
    AssertNe {
        actual: Box<Expr>,
        expected: Box<Expr>,
        message: Option<String>,
    },
    Assert {
        condition: Box<Expr>,
        message: Option<String>,
    },
    AssertTrue {
        expression: Box<Expr>,
        message: Option<String>,
    },
    AssertFalse {
        expression: Box<Expr>,
        message: Option<String>,
    },
}

/// Argument types for function calls
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Argument {
    Positional(Expr),
    Named { name: String, value: Expr },
}

/// Promise types for Promise expressions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PromiseType {
    Resolve, // Promise.resolve(value)
    Reject,  // Promise.reject(error)
    Delay,   // Promise.delay(ms, value)
}

/// Binary operators
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Equal,
    NotEqual,
    LessThan,
    LessThanEqual,
    GreaterThan,
    GreaterThanEqual,
    And,
    Or,
}

/// Unary operators
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UnaryOp {
    Negate,
    Not,
}

/// Pattern matching arm
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Box<Expr>>,
    pub expression: Expr,
}

/// Pattern for matching
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Pattern {
    Literal(Value),
    Identifier(String),
    Wildcard,
    List {
        patterns: Vec<Pattern>,
        rest: Option<String>, // For ...rest patterns
    },
    Tuple(Vec<Pattern>),
    // Result patterns for error handling
    Ok(Box<Pattern>),
    Err(Box<Pattern>),
    // Enum variant patterns
    EnumVariant {
        variant_name: String,
        patterns: Vec<Pattern>,
    },
    // Struct patterns
    Struct {
        type_name: String,
        field_patterns: Vec<(String, Pattern)>,
    },
    // Anonymous struct patterns
    AnonymousStruct {
        field_patterns: Vec<(String, Pattern)>,
    },
    // New pattern variants
    Range {
        start: Box<Pattern>,
        end: Box<Pattern>,
        inclusive: bool,
    },
    Or {
        alternatives: Vec<Pattern>,
    },
    Guarded {
        pattern: Box<Pattern>,
        guard: Box<Expr>,
    },
    // Rest pattern for capturing remaining elements
    Rest(String),
}

/// State of a Promise value
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PromiseState {
    Pending,
    Resolved,
    Rejected,
}

/// Enum variant data for proper enum value representation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EnumVariantData {
    Unit,                           // Simple variant: Red
    Tuple(Vec<Value>),              // Tuple variant: Point(x, y)
    Struct(HashMap<String, Value>), // Struct variant: Person { name, age }
}

/// Runtime values
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Value {
    Integer(i64),
    Float(f64),
    String(Arc<String>),
    Boolean(bool),
    List(Arc<[Value]>),
    Map(Arc<HashMap<String, Value>>),
    Tuple(Arc<Vec<Value>>),
    Function(Function),
    Builtin(BuiltinFunction),
    Struct {
        type_name: String,
        fields: HashMap<String, Value>,
    },
    // Range values for range expressions like 1..50
    Range {
        start: i64,
        end: i64,
        inclusive: bool,
    },
    // Result values for error handling
    Ok(Box<Value>),
    Err(Box<Value>),
    Unit,

    // Enum values with proper variant representation
    Enum {
        type_name: String,
        variant_name: String,
        variant_data: EnumVariantData,
    },

    // A tuple-variant constructor, e.g. `Circle` in `Circle(radius)`. Unit
    // variants are `Enum` values directly; payload variants are callables
    // that build an `Enum` when applied. Defined by `eval_type_decl`.
    EnumConstructor {
        type_name: String,
        variant_name: String,
        arity: usize,
    },

    // Promise values for async operations
    Promise {
        state: PromiseState,
        value: Option<Box<Value>>,
        error: Option<Box<Value>>,
        /// For `promise delay`: epoch millis when the value becomes ready.
        /// `await` sleeps out the remainder. This is what makes a delayed
        /// promise awaitable at all in a synchronous interpreter — there is
        /// no scheduler to resolve it in the background.
        #[serde(default)]
        resolve_at_epoch_ms: Option<u64>,
        /// For `spawn`: the id of a real background thread in the spawn
        /// registry. `await` joins it (memoized, so a cloned promise can be
        /// awaited more than once).
        #[serde(default)]
        task_id: Option<u64>,
    },

    // Type information for exported types
    TypeInfo {
        name: String,
        definition: TypeDefinition,
    },

    // A value owned by an OVM module (e.g. an ods array). The handle is
    // one Arc shared verbatim with the bytecode tier, so crossing the
    // boundary is a refcount bump, never a conversion.
    Native(crate::native::NativeHandle),
}

// Value is Send + Sync automatically now that Expr uses Arc throughout;
// assert it here so a non-thread-safe field can't sneak back in.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Value>()
};

/// Function value
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Function {
    pub name: Option<String>,
    pub parameters: Vec<Parameter>,
    // Arc so cloning a function value (every call/env lookup) is cheap
    // instead of deep-copying the body AST and captured environment
    pub body: Arc<Expr>,
    // Persistent map so a call can adopt the whole closure as its
    // environment in O(1) instead of copying every entry per call —
    // with the prelude captured, that was ~200 inserts on every call
    pub closure: Arc<im::HashMap<String, Value>>,
    /// Trait bounds resolved to parameter positions at declaration:
    /// (parameter index, required trait names). Checked at call time so a
    /// value not implementing the bound fails at the boundary with a clear
    /// message. Empty for lambdas and unbounded functions.
    #[serde(default)]
    pub param_bounds: Vec<(usize, Vec<String>)>,
}

/// Built-in function
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuiltinFunction {
    pub name: String,
    pub arity: usize,
}

/// Function parameter documentation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionParameter {
    pub name: String,
    pub param_type: String,
    pub description: String,
    pub optional: bool,
}

/// Basic type expressions for optional type annotations
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeAnnotation {
    Int,
    Float,
    String,
    Bool,
    Unit, // () type
    List(Box<TypeAnnotation>),
    Map {
        key_type: Box<TypeAnnotation>,
        value_type: Box<TypeAnnotation>,
    },
    Tuple(Vec<TypeAnnotation>),
    Function {
        params: Vec<TypeAnnotation>,
        return_type: Box<TypeAnnotation>,
    },
    // Custom types
    Custom(String), // User-defined type by name
    // Generic types
    Generic {
        base_type: String,              // Base type name (e.g., "List", "Tree")
        type_args: Vec<TypeAnnotation>, // Type arguments (e.g., <Int>, <T>)
    },
    TypeVariable(String), // Type variable (e.g., T, U)
    // Type constraints
    Constrained {
        // Type with constraints
        type_var: Box<TypeAnnotation>,    // The type variable
        constraints: Vec<TypeConstraint>, // Constraints on the type
    },
    // Result type for error handling
    Result {
        ok_type: Box<TypeAnnotation>,
        err_type: Box<TypeAnnotation>,
    },
    // Promise type for async operations
    Promise {
        value_type: Box<TypeAnnotation>,
        error_type: Option<Box<TypeAnnotation>>, // Optional error type
    },
    // Type inference placeholders
    Inferred(String), // For type variables during inference
    Unknown,          // For unresolved types
    // New type variants
    Union {
        types: Vec<TypeAnnotation>,
    },
    Intersection {
        types: Vec<TypeAnnotation>,
    },
    Literal {
        value: Box<Value>,
    },
    Range {
        start: Box<TypeAnnotation>,
        end: Box<TypeAnnotation>,
        inclusive: bool,
    },
}

/// A struct field's declared type reduced to the subset the runtime can
/// reliably enforce when a struct is constructed. Only these annotations are
/// checked; every other annotation (a generic type parameter, a list/map,
/// a function type, a union, an absent/unknown type) reduces to `None` and
/// leaves the field unchecked — construction never rejects what it cannot
/// reliably verify.
///
/// The check itself is a single string comparison: the declared type's name
/// against the value's runtime type name. `Value::type_name` and
/// `OvmValue::type_name` spell primitives identically (`Int`, `Float`,
/// `Bool`, `String`) and report a struct/enum's declared name, so the same
/// comparison enforces the same rule byte-for-byte on every tier. An `Int`
/// value does NOT satisfy a `Float` field: the match is exact, no numeric
/// widening.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldTypeCheck {
    Int,
    Float,
    Bool,
    String,
    /// A declared struct or enum type, matched by name against the value's
    /// runtime type name.
    Named(String),
}

impl FieldTypeCheck {
    /// Reduce a declared field annotation to a checkable type, or `None` to
    /// leave the field unchecked. `type_params` are the enclosing type's
    /// generic parameters, which are erased at runtime and never checked.
    pub fn from_annotation(ann: &TypeAnnotation, type_params: &[String]) -> Option<Self> {
        match ann {
            TypeAnnotation::Int => Some(Self::Int),
            TypeAnnotation::Float => Some(Self::Float),
            TypeAnnotation::Bool => Some(Self::Bool),
            TypeAnnotation::String => Some(Self::String),
            // A bare custom name is a declared struct or enum — unless it is
            // one of this type's generic parameters, which has no runtime
            // identity to check against.
            TypeAnnotation::Custom(name) if !type_params.iter().any(|p| p == name) => {
                Some(Self::Named(name.clone()))
            }
            _ => None,
        }
    }

    /// The declared type's name, as it must appear as a value's runtime type
    /// name to satisfy the field.
    pub fn expected_name(&self) -> &str {
        match self {
            Self::Int => "Int",
            Self::Float => "Float",
            Self::Bool => "Bool",
            Self::String => "String",
            Self::Named(name) => name,
        }
    }

    /// Does a value whose runtime type name is `actual` satisfy this field?
    pub fn accepts(&self, actual: &str) -> bool {
        self.expected_name() == actual
    }
}

/// Generic type definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenericTypeDefinition {
    pub name: String,
    pub type_params: Vec<String>, // Names of type parameters (e.g., "T", "U")
    pub definition: TypeDefinition, // The actual type definition
}

/// Type checking context and state
#[derive(Debug, Clone)]
pub struct TypeContext {
    pub variables: HashMap<String, TypeAnnotation>,
    pub functions: HashMap<String, TypeAnnotation>,
    pub type_vars: HashMap<String, TypeAnnotation>,
    pub next_type_var: usize,

    // New fields for generics
    pub generic_types: HashMap<String, GenericTypeDefinition>,
    pub type_parameters: Vec<String>, // Currently in-scope type parameters
}

/// Type constraint for generic types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeConstraint {
    pub constraint_type: ConstraintType,
    pub type_class: String, // e.g., "Numeric", "Comparable"
}

/// Type constraint types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConstraintType {
    Implements, // Type must implement an interface
    Extends,    // Type must extend another type
    Equals,     // Type must equal another type
}

/// Type checking errors
#[derive(Debug, Clone, PartialEq)]
pub enum TypeError {
    TypeMismatch {
        expected: TypeAnnotation,
        found: TypeAnnotation,
        location: String,
    },
    UnknownVariable {
        name: String,
    },
    UnknownFunction {
        name: String,
    },
    ArityMismatch {
        expected: usize,
        found: usize,
        function: String,
    },
    InvalidOperation {
        op: String,
        left_type: TypeAnnotation,
        right_type: Option<TypeAnnotation>,
    },
    CannotInfer {
        expression: String,
    },
    /// Enhanced error for constraint violations
    ConstraintViolation {
        type_name: String,
        constraint: String,
        location: String,
    },
    /// Enhanced error for union type mismatches
    UnionMismatch {
        expected_types: Vec<TypeAnnotation>,
        found: TypeAnnotation,
        location: String,
    },
    /// Enhanced error for intersection type issues
    IntersectionMismatch {
        required_types: Vec<TypeAnnotation>,
        found: TypeAnnotation,
        location: String,
    },
}

/// Module import statement
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportDecl {
    pub module_path: String,
    pub items: Option<Vec<String>>, // None for wildcard import
}

/// Module export statement  
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportDecl {
    pub name: String,
    pub value: Expr,
}

/// Custom type declaration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeDecl {
    pub name: String,
    /// 1-based (line, column) of the type name in source.
    #[serde(default)]
    pub name_span: Option<(u32, u32)>,
    pub type_params: Vec<String>, // Type parameters for generic types
    pub definition: TypeDefinition,
}

/// Type definition variants
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeDefinition {
    Struct { fields: Vec<StructField> },
    Enum { variants: Vec<EnumVariant> },
    Union { types: Vec<TypeAnnotation> },
}

/// Struct field definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructField {
    pub name: String,
    pub field_type: TypeAnnotation,
}

/// Enum variant definition  
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub data: Option<Vec<TypeAnnotation>>, // None for unit variants, Some for tuple variants
}

/// Struct literal expression
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructLiteral {
    pub type_name: String,
    pub fields: Vec<FieldValue>,
}

/// Field value in struct literal
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldValue {
    pub name: String,
    pub value: Expr,
}

/// Map entry for map literals
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MapEntry {
    pub key: Expr,
    pub value: Expr,
}

impl Default for Program {
    fn default() -> Self {
        Self::new()
    }
}

impl Program {
    pub fn new() -> Self {
        Self {
            statements: Vec::new(),
        }
    }

    pub fn add_statement(&mut self, statement: Statement) {
        self.statements.push(statement);
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Integer(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::Float(value)
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::String(Arc::new(value))
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Boolean(value)
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Value::List(Arc::from(value))
    }
}

/// Render a float for human display so a `Float` is always distinguishable
/// from an `Int` and never expands into hundreds of fake-precision digits.
///
/// Rust's default `f64` Display drops the fractional part of whole values
/// (`2.0` prints as `2`) and writes very large or very small magnitudes as
/// full decimal (`1e301` becomes a 302-digit integer). Both violate the
/// "shows a value's shape" contract of `to_string`/`show`, so:
/// - whole finite values keep a trailing `.0` (`2.0`, not `2`);
/// - magnitudes at or beyond 1e16, or nonzero below 1e-4, use exponent form;
/// - NaN and infinities render as `NaN` / `inf` / `-inf`.
///
/// The output still round-trips (the underlying formats are shortest-form),
/// and it is used by every human-facing path so all three tiers agree.
pub fn format_float(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf" } else { "inf" }.to_string();
    }
    let a = x.abs();
    if a != 0.0 && !(1e-4..1e16).contains(&a) {
        // Shortest exponent form, e.g. "1e301", "1.5e-5".
        return format!("{:e}", x);
    }
    let s = format!("{}", x);
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{}.0", s)
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Integer(n) => write!(f, "{}", n),
            Value::Float(x) => write!(f, "{}", format_float(*x)),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Boolean(b) => write!(f, "{}", b),
            Value::List(items_rc) => {
                let items = items_rc.as_ref();
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Map(map_rc) => {
                let map = map_rc.as_ref();
                write!(f, "#{{")?;
                for (i, (key, value)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\": {}", key, value)?;
                }
                write!(f, "}}")
            }
            Value::Tuple(items_rc) => {
                let items = items_rc.as_ref();
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, ")")
            }
            Value::Function(func) => {
                if let Some(name) = &func.name {
                    write!(f, "<function: {}>", name)
                } else {
                    write!(f, "<function>")
                }
            }
            Value::Builtin(builtin) => write!(f, "<builtin: {}>", builtin.name),
            Value::Struct { type_name, fields } => {
                write!(f, "<struct: {}>", type_name)?;
                write!(f, "{{")?;
                for (i, (name, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, value)?;
                }
                write!(f, "}}")
            }
            Value::Ok(value) => write!(f, "Ok({})", value),
            Value::Err(value) => write!(f, "Err({})", value),
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                if *inclusive {
                    write!(f, "{}..={}", start, end)
                } else {
                    write!(f, "{}..{}", start, end)
                }
            }
            Value::Unit => write!(f, "()"),
            Value::Enum {
                type_name,
                variant_name,
                variant_data,
            } => match variant_data {
                EnumVariantData::Unit => write!(f, "{}.{}", type_name, variant_name),
                EnumVariantData::Tuple(values) => {
                    write!(f, "{}.{}(", type_name, variant_name)?;
                    for (i, value) in values.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", value)?;
                    }
                    write!(f, ")")
                }
                EnumVariantData::Struct(fields) => {
                    write!(f, "{}.{} {{ ", type_name, variant_name)?;
                    for (i, (name, value)) in fields.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}: {}", name, value)?;
                    }
                    write!(f, " }}")
                }
            },
            Value::EnumConstructor {
                type_name,
                variant_name,
                ..
            } => write!(f, "{}.{}", type_name, variant_name),
            Value::Promise {
                state,
                value,
                error,
                ..
            } => match state {
                PromiseState::Pending => write!(f, "Promise<Pending>"),
                PromiseState::Resolved => {
                    if let Some(val) = value {
                        write!(f, "Promise<Resolved: {}>", val)
                    } else {
                        write!(f, "Promise<Resolved>")
                    }
                }
                PromiseState::Rejected => {
                    if let Some(err) = error {
                        write!(f, "Promise<Rejected: {}>", err)
                    } else {
                        write!(f, "Promise<Rejected>")
                    }
                }
            },
            Value::TypeInfo { name, .. } => write!(f, "<type: {}>", name),
            Value::Native(handle) => write!(f, "{}", handle.0.display()),
        }
    }
}

impl Value {
    /// Returns the type name of this value as a string
    pub fn type_name(&self) -> String {
        match self {
            Value::Integer(_) => "Int".to_string(),
            Value::Float(_) => "Float".to_string(),
            Value::String(_) => "String".to_string(),
            Value::Boolean(_) => "Bool".to_string(),
            Value::List(_) => "List".to_string(),
            Value::Map(_) => "Map".to_string(),
            Value::Tuple(_) => "Tuple".to_string(),
            Value::Function(_) => "Function".to_string(),
            Value::Builtin(_) => "Builtin".to_string(),
            Value::Struct { type_name, .. } => type_name.clone(),
            Value::Range { .. } => "Range".to_string(),
            Value::Ok(_) => "Result".to_string(),
            Value::Err(_) => "Result".to_string(),
            Value::Unit => "Unit".to_string(),
            Value::Enum { type_name, .. } => type_name.clone(),
            Value::EnumConstructor { type_name, .. } => type_name.clone(),
            Value::Promise { .. } => "Promise".to_string(),
            Value::TypeInfo { .. } => "Type".to_string(),
            Value::Native(handle) => handle.0.type_name().to_string(),
        }
    }

    /// Compare values for sorting (returns None if not comparable)
    pub fn compare_for_sort(&self, other: &Value) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => Some(a.cmp(b)),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
            (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
            (Value::Boolean(a), Value::Boolean(b)) => Some(a.cmp(b)),
            // For mixed numeric types, convert to float
            (Value::Integer(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
            (Value::Float(a), Value::Integer(b)) => a.partial_cmp(&(*b as f64)),
            // Result values are not comparable
            (Value::Ok(_), _) | (Value::Err(_), _) | (_, Value::Ok(_)) | (_, Value::Err(_)) => None,
            _ => None, // Other types are not comparable for sorting
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TemplatePart {
    Literal(String),
    Interpolation(Box<Expr>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BitwiseOp {
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

// New ShareDecl enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ShareDecl {
    Function(FunctionDecl),
    Let(LetDecl),
    Type(TypeDecl),
    Use(UseDecl),     // Transitive sharing: share use module { items }
    Trait(TraitDecl), // `share trait` — accepted; traits register globally
    Impl(ImplDecl),   // `share impl`  — accepted; impls register globally
}

// Use item enum for specific imports or wildcard
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum UseItem {
    Specific(String),
    Wildcard,
}

// New UseDecl struct
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UseDecl {
    pub path: Vec<String>,
    pub items: Vec<UseItem>,
}

// Test declaration struct
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestDecl {
    pub name: String,
    pub body: Vec<Statement>,
}

#[cfg(test)]
mod float_format_tests {
    use super::format_float;

    #[test]
    fn whole_values_keep_a_decimal() {
        assert_eq!(format_float(2.0), "2.0");
        assert_eq!(format_float(-3.0), "-3.0");
        assert_eq!(format_float(0.0), "0.0");
        assert_eq!(format_float(1234567890.0), "1234567890.0");
    }

    #[test]
    fn fractional_values_are_shortest() {
        assert_eq!(format_float(3.5), "3.5");
        assert_eq!(format_float(0.1), "0.1");
        assert_eq!(format_float(-2.25), "-2.25");
    }

    #[test]
    fn extremes_use_exponent_not_hundreds_of_digits() {
        assert_eq!(format_float(1e301), "1e301");
        assert_eq!(format_float(1e-5), "1e-5");
        assert!(format_float(1e301).len() < 10);
        // The 1e16 boundary: just below stays decimal, at/above goes exponent.
        assert_eq!(format_float(9.9e15), "9900000000000000.0");
        assert_eq!(format_float(1e16), "1e16");
    }

    #[test]
    fn non_finite_values_render_plainly() {
        assert_eq!(format_float(f64::NAN), "NaN");
        assert_eq!(format_float(f64::INFINITY), "inf");
        assert_eq!(format_float(f64::NEG_INFINITY), "-inf");
    }

    #[test]
    fn every_output_round_trips_back_to_the_same_float() {
        for x in [2.0, 3.5, -0.0, 1234567890.0, 1e301, 1e-5, 0.1, 9.9e15, 1e16] {
            let s = format_float(x);
            let back: f64 = s.parse().unwrap();
            assert_eq!(back.to_bits(), x.to_bits(), "round-trip failed for {s}");
        }
    }
}
