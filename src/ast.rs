use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use std::sync::Arc;

/// Represents a complete Olang program
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

/// Where a runtime error surfaced: the innermost located statement being
/// evaluated when the error first appeared, plus the interpreter call
/// stack at that moment (function names, outermost first). Attached as a
/// side channel — error Display strings never change — and rendered only
/// by top-level reporters (file runner, REPL).
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorLocation {
    pub line: u32,
    pub column: u32,
    pub call_stack: Vec<String>,
    /// A one-line remediation hint (e.g. did-you-mean candidates),
    /// computed where the error was raised.
    pub hint: Option<String>,
}

/// Statement types in Olang
///
/// Equality is span-insensitive: `Located` wrappers are stripped from both
/// sides before comparing, so two parses of the same program compare equal
/// even when formatting has moved statements to different lines/columns.
/// Position is metadata, not identity — `olang fmt`'s AST-verification gate
/// depends on this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Statement {
    /// A statement wrapped with its source position (1-based line/column
    /// of its first token). The parser wraps every statement it builds;
    /// desugared or synthetic statements may appear bare. Walkers that
    /// don't care about position delegate straight through to `stmt`.
    Located {
        line: u32,
        column: u32,
        stmt: Box<Statement>,
    },
    Expression(Expr),
    LetDecl(LetDecl),
    FunctionDecl(FunctionDecl),
    TypeDecl(TypeDecl),
    ErrorTypeDecl(ErrorTypeDecl),
    ShareDecl(ShareDecl),
    UseDecl(UseDecl),
    TestDecl(TestDecl),
    TraitDecl(TraitDecl),
    ImplDecl(ImplDecl),
    /// A `meta fn` declaration — a function that exists only at expansion
    /// time (docs/macros.md). Never evaluated at runtime: the expander
    /// strips it from the program source, and `span` (byte offsets into
    /// the source) is what lets it do that with line count preserved.
    MetaFnDecl {
        decl: FunctionDecl,
        span: (usize, usize),
    },
    /// One or more `@name` decorators stacked above a `type`, `fn`, or
    /// `let` declaration.
    /// The declaration rides as source text: the expander hands it to each
    /// macro innermost-first and replaces the whole span with the result.
    DecoratedDecl {
        decorators: Vec<Decorator>,
        decl_src: String,
        span: (usize, usize),
        line: u32,
    },
}

/// One `@name` or `@name(args)` above a declaration. Arguments are the
/// source text of each expression, validated by the parse that built this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Decorator {
    pub name: String,
    pub args_src: Vec<String>,
    pub line: u32,
}

impl PartialEq for Statement {
    fn eq(&self, other: &Self) -> bool {
        match (self.unwrapped(), other.unwrapped()) {
            (Statement::Expression(a), Statement::Expression(b)) => a == b,
            (Statement::LetDecl(a), Statement::LetDecl(b)) => a == b,
            (Statement::FunctionDecl(a), Statement::FunctionDecl(b)) => a == b,
            (Statement::TypeDecl(a), Statement::TypeDecl(b)) => a == b,
            (Statement::ErrorTypeDecl(a), Statement::ErrorTypeDecl(b)) => a == b,
            (Statement::ShareDecl(a), Statement::ShareDecl(b)) => a == b,
            (Statement::UseDecl(a), Statement::UseDecl(b)) => a == b,
            (Statement::TestDecl(a), Statement::TestDecl(b)) => a == b,
            (Statement::TraitDecl(a), Statement::TraitDecl(b)) => a == b,
            (Statement::ImplDecl(a), Statement::ImplDecl(b)) => a == b,
            (Statement::MetaFnDecl { decl: a, .. }, Statement::MetaFnDecl { decl: b, .. }) => {
                a == b
            }
            (
                Statement::DecoratedDecl {
                    decorators: da,
                    decl_src: sa,
                    ..
                },
                Statement::DecoratedDecl {
                    decorators: db,
                    decl_src: sb,
                    ..
                },
            ) => da == db && sa == sb,
            _ => false,
        }
    }
}

impl Statement {
    /// The statement itself, stripped of any Located wrapper — for
    /// consumers that match on statement kinds and don't care where the
    /// statement came from.
    pub fn unwrapped(&self) -> &Statement {
        let mut s = self;
        while let Statement::Located { stmt, .. } = s {
            s = stmt;
        }
        s
    }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetDecl {
    pub pattern: Pattern,
    pub type_annotation: Option<TypeAnnotation>,
    pub value: Option<Expr>,
    /// 1-based (line, column) of the binding's first name, when parsed
    /// from source. Desugared/synthetic declarations carry None.
    #[serde(default)]
    pub name_span: Option<(u32, u32)>,
    /// `let mut x = ...` — the binding may be reassigned. Plain `let`
    /// bindings are immutable: assigning to one is an error (a fresh
    /// `let` of the same name still shadows, which is the idiomatic way
    /// to thread a value through a pipeline). Defaults to false so ASTs
    /// serialized before 0.61 still deserialize.
    #[serde(default)]
    pub mutable: bool,
}

/// Function declaration for named/recursive functions
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// `@name(args)` — a macro invocation (docs/macros.md), gone before
    /// the interpreter or any tier sees the program. Arguments are the
    /// source text of each argument expression; `span` is the call's byte
    /// range in the source, which is what the expander splices over.
    MacroCall {
        name: String,
        args_src: Vec<String>,
        span: (usize, usize),
        line: u32,
    },
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

    // Background execution: `spawn f(x)` runs the call on its own OS
    // thread and evaluates to a task handle, which `task.join` collects.
    Spawn(Box<Expr>),

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

impl BinaryOp {
    /// The operator as written in source — for error messages shared
    /// verbatim by the interpreter and the bytecode tier.
    pub fn symbol(&self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::Modulo => "%",
            BinaryOp::Equal => "==",
            BinaryOp::NotEqual => "!=",
            BinaryOp::LessThan => "<",
            BinaryOp::LessThanEqual => "<=",
            BinaryOp::GreaterThan => ">",
            BinaryOp::GreaterThanEqual => ">=",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
        }
    }
}

/// Unary operators
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UnaryOp {
    Negate,
    Not,
}

impl UnaryOp {
    /// The operator as written in source — shared by both tiers' errors.
    pub fn symbol(&self) -> &'static str {
        match self {
            UnaryOp::Negate => "-",
            UnaryOp::Not => "!",
        }
    }
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
    // `Arc<Vec<_>>`, not `Arc<[_]>`, so a sole-owner list can grow in
    // place (`Arc::get_mut` + `Vec::extend`) — the interpreter half of
    // the in-place-append fusion. Reads are unaffected: `Vec` derefs to
    // the same slice.
    List(Arc<Vec<Value>>),
    Map(Arc<HashMap<String, Value>>),
    Tuple(Arc<Vec<Value>>),
    Function(Function),
    Builtin(BuiltinFunction),
    Struct {
        type_name: String,
        /// `Arc`, like the `Map` two lines up. Without it a struct passed
        /// to a function copied every field, so the language punished its
        /// own type system: a recursive structure written the readable
        /// way — `type Node = struct { k, v, l, r }` — copied the whole
        /// tree on every descent, and bare lists were the only way to get
        /// a tree that behaved like one. Reads are unaffected; `Arc`
        /// derefs to the same map.
        fields: Arc<HashMap<String, Value>>,
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
    /// Per-parameter runtime type checks, computed once at declaration
    /// from the annotations (generic parameters erase to None). Position-
    /// aligned with `parameters`; empty means fully dynamic.
    #[serde(default)]
    pub param_checks: Vec<Option<FieldTypeCheck>>,
    /// Runtime check for the declared return type, if checkable.
    #[serde(default)]
    pub return_check: Option<FieldTypeCheck>,
    /// The source file this function was defined in, if known — set at
    /// declaration from the interpreter's current module path. Used by
    /// `olang test --coverage` to attribute executed lines to the file
    /// that owns the code, even when a test in one file calls into
    /// another. `None` for reconstructed values and dynamic contexts.
    #[serde(default)]
    pub def_file: Option<String>,
}

impl Function {
    /// A copy whose `body` and `closure` live behind FRESH Arcs. The
    /// parallel workers use this: every call touches those two Arcs (the
    /// HOF cache verifies identity through `Weak::upgrade`, an atomic
    /// read-modify-write), and when a dozen cores share one kernel value
    /// they serialize on its reference counts. One localization per
    /// worker makes the atomics core-local; the body AST is cloned once
    /// per worker, not per element.
    pub fn thread_localized(&self) -> Function {
        let mut f = self.clone();
        f.body = std::sync::Arc::new((*self.body).clone());
        f.closure = std::sync::Arc::new((*self.closure).clone());
        f
    }
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

impl TypeAnnotation {
    /// Render the annotation as it would appear in source — used for
    /// error text where the runtime's bare type-name vocabulary would
    /// lose the information that matters (`(Int) -> Int`, not "Function").
    pub fn display_source(&self) -> String {
        match self {
            TypeAnnotation::Int => "Int".to_string(),
            TypeAnnotation::Float => "Float".to_string(),
            TypeAnnotation::Bool => "Bool".to_string(),
            TypeAnnotation::String => "String".to_string(),
            TypeAnnotation::Unit => "()".to_string(),
            TypeAnnotation::List(t) => format!("[{}]", t.display_source()),
            TypeAnnotation::Map {
                key_type,
                value_type,
            } => format!(
                "Map<{}, {}>",
                key_type.display_source(),
                value_type.display_source()
            ),
            TypeAnnotation::Tuple(ts) => format!(
                "({})",
                ts.iter()
                    .map(|t| t.display_source())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeAnnotation::Function {
                params,
                return_type,
            } => format!(
                "({}) -> {}",
                params
                    .iter()
                    .map(|t| t.display_source())
                    .collect::<Vec<_>>()
                    .join(", "),
                return_type.display_source()
            ),
            TypeAnnotation::Result { ok_type, err_type } => format!(
                "Result<{}, {}>",
                ok_type.display_source(),
                err_type.display_source()
            ),
            TypeAnnotation::Custom(n) | TypeAnnotation::TypeVariable(n) => n.clone(),
            TypeAnnotation::Generic {
                base_type,
                type_args,
            } => format!(
                "{}<{}>",
                base_type,
                type_args
                    .iter()
                    .map(|t| t.display_source())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeAnnotation::Union { types } => types
                .iter()
                .map(|t| t.display_source())
                .collect::<Vec<_>>()
                .join(" | "),
            _ => "_".to_string(),
        }
    }
}

/// A literal annotation's value, small enough to compare by ==.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LitCheck {
    Int(i64),
    Str(String),
    Bool(bool),
}

impl LitCheck {
    /// The literal as it appears in source: `"open"`, `5`, `true`.
    pub fn display(&self) -> String {
        match self {
            LitCheck::Int(i) => i.to_string(),
            LitCheck::Str(s) => format!("\"{}\"", s),
            LitCheck::Bool(b) => b.to_string(),
        }
    }
}

/// A scalar value viewed for literal-type comparison — the third leg of
/// the value view alongside the type name and the Result payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScalarView<'a> {
    Int(i64),
    Str(&'a str),
    Bool(bool),
}

/// A struct field's declared type reduced to the subset the runtime can
/// reliably enforce when a struct is constructed. Only these annotations are
/// checked; every other annotation (a generic type parameter, an
/// absent/unknown type, a reserved form) reduces to `None` and leaves the
/// field unchecked — construction never rejects what it cannot reliably
/// verify.
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
    /// Shallow container checks: the value must be a List/Map/Tuple, but
    /// element types are the static checker's concern — an O(1) promise,
    /// never an O(n) walk on a hot call.
    List,
    Map,
    Tuple,
    /// `Result<T, E>`: the value must be an Ok/Err, and the payload of
    /// whichever side is present checks *shallowly* against its declared
    /// type. Unlike containers, a Result holds one payload, so this stays
    /// a fixed number of O(1) name comparisons — never a walk.
    Result {
        ok: Option<Box<FieldTypeCheck>>,
        err: Option<Box<FieldTypeCheck>>,
    },
    /// A literal type: the value must EQUAL the literal. Scalars only
    /// (string/int/bool, what the grammar's literal_type produces), so
    /// the check is one comparison. Unions of literals are lightweight
    /// enums: `status: "open" | "in-progress" | "done"`.
    Literal(LitCheck),
    /// `(A, B) -> R`: the value must be callable with exactly the
    /// annotation's parameter count — required-parameter floor and total
    /// ceiling both respected when the value exposes them. Parameter and
    /// return *types* inside the signature are the checker's concern (and
    /// the callee's own boundaries enforce its annotations when called).
    /// `display` is the annotation's source rendering for error text.
    Function {
        arity: usize,
        display: String,
    },
    /// `A | B`: the value satisfies the union if it satisfies any branch.
    /// Branch count is annotation-sized, each branch check O(1), so the
    /// whole check stays O(1) in the value. Only constructed when every
    /// branch is itself checkable — a union with an unenforceable branch
    /// (e.g. a generic parameter) stays unchecked, because rejecting a
    /// value the unenforceable branch might accept would be wrong.
    Union(Vec<FieldTypeCheck>),
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
            // Containers check shallowly: List<Int> promises "a List".
            TypeAnnotation::List(_) => Some(Self::List),
            TypeAnnotation::Map { .. } => Some(Self::Map),
            TypeAnnotation::Tuple(_) => Some(Self::Tuple),
            // Result<T, E> checks the present side's payload shallowly.
            TypeAnnotation::Result { ok_type, err_type } => Some(Self::Result {
                ok: Self::from_annotation(ok_type, type_params).map(Box::new),
                err: Self::from_annotation(err_type, type_params).map(Box::new),
            }),
            // A literal annotation checks by value equality.
            TypeAnnotation::Literal { value } => match value.as_ref() {
                Value::Integer(i) => Some(Self::Literal(LitCheck::Int(*i))),
                Value::String(st) => Some(Self::Literal(LitCheck::Str(st.as_ref().clone()))),
                Value::Boolean(b) => Some(Self::Literal(LitCheck::Bool(*b))),
                _ => None,
            },
            // A function annotation enforces callability + arity.
            TypeAnnotation::Function { params, .. } => Some(Self::Function {
                arity: params.len(),
                display: ann.display_source(),
            }),
            // A union enforces only when every branch does: one
            // unenforceable branch (e.g. a generic parameter) makes the
            // whole union unchecked rather than wrongly strict.
            TypeAnnotation::Union { types } => {
                let branches: Option<Vec<_>> = types
                    .iter()
                    .map(|t| Self::from_annotation(t, type_params))
                    .collect();
                branches.map(Self::Union)
            }
            TypeAnnotation::Generic {
                base_type,
                type_args,
            } if !type_params.iter().any(|p| p == base_type) => match base_type.as_str() {
                "List" => Some(Self::List),
                "Map" => Some(Self::Map),
                "Result" => Some(Self::Result {
                    ok: type_args
                        .first()
                        .and_then(|t| Self::from_annotation(t, type_params))
                        .map(Box::new),
                    err: type_args
                        .get(1)
                        .and_then(|t| Self::from_annotation(t, type_params))
                        .map(Box::new),
                }),
                other => Some(Self::Named(other.to_string())),
            },
            _ => None,
        }
    }

    /// The declared type's name, as it must appear as a value's runtime type
    /// name to satisfy the field.
    ///
    /// If this check is a bare declared-type name — a struct or enum — its
    /// name, so the enforcer can ask whether that type actually exists and
    /// report "unknown type 'X'" rather than blaming the value when it does
    /// not. A union or built-in check has no single such name.
    pub fn named_type(&self) -> Option<&str> {
        match self {
            Self::Named(name) => Some(name),
            _ => None,
        }
    }

    pub fn expected_name(&self) -> &str {
        match self {
            Self::Int => "Int",
            Self::Float => "Float",
            Self::Bool => "Bool",
            Self::String => "String",
            Self::List => "List",
            Self::Map => "Map",
            Self::Tuple => "Tuple",
            Self::Result { .. } => "Result",
            Self::Function { .. } => "Function",
            Self::Literal(LitCheck::Int(_)) => "Int",
            Self::Literal(LitCheck::Str(_)) => "String",
            Self::Literal(LitCheck::Bool(_)) => "Bool",
            // A union has no single name; callers that can see values use
            // `check_value`/`accepts`, and error text uses `display_name`.
            Self::Union(_) => "Union",
            Self::Named(name) => name,
        }
    }

    /// Does a value whose runtime type name is `actual` satisfy this field?
    /// For a union: does any branch?
    pub fn accepts(&self, actual: &str) -> bool {
        match self {
            Self::Union(branches) => branches.iter().any(|b| b.accepts(actual)),
            // Builtins are callable too.
            Self::Function { .. } => actual == "Function" || actual == "Builtin",
            // A name alone can never prove a VALUE; value-aware checking
            // happens in check_value, and static discharge must refuse.
            Self::Literal(_) => false,
            other => other.expected_name() == actual,
        }
    }

    /// The declared type as error text: `Result<Int, _>` where payload
    /// checks exist, `Int | String` for unions, the bare name otherwise.
    pub fn display_name(&self) -> String {
        match self {
            Self::Result { ok, err } => {
                let side = |s: &Option<Box<FieldTypeCheck>>| match s {
                    Some(c) => c.display_name(),
                    None => "_".to_string(),
                };
                format!("Result<{}, {}>", side(ok), side(err))
            }
            Self::Function { display, .. } => display.clone(),
            Self::Literal(lit) => lit.display(),
            Self::Union(branches) => branches
                .iter()
                .map(|b| b.display_name())
                .collect::<Vec<_>>()
                .join(" | "),
            other => other.expected_name().to_string(),
        }
    }

    /// Value-aware check shared by every tier. `type_name` is the value's
    /// outer runtime type name; `result_payload` is `Some((is_ok,
    /// payload_type_name))` when the value is an Ok/Err. Returns the
    /// `(expected, actual)` pair for an "expects X, got Y" message on
    /// violation, `None` when the promise holds. Payload checks are one
    /// O(1) name comparison — a Result holds a single payload, so this
    /// never walks anything.
    pub fn check_value(
        &self,
        type_name: &str,
        result_payload: Option<(bool, &str)>,
        fn_arity: Option<(usize, usize)>,
        scalar: Option<ScalarView>,
    ) -> Option<(String, String)> {
        // Literals compare by value — before the name-based paths, which
        // can never satisfy them.
        if let Self::Literal(lit) = self {
            let satisfied = matches!(
                (lit, scalar),
                (LitCheck::Int(a), Some(ScalarView::Int(b))) if *a == b
            ) || matches!(
                (lit, scalar),
                (LitCheck::Str(a), Some(ScalarView::Str(b))) if a == b
            ) || matches!(
                (lit, scalar),
                (LitCheck::Bool(a), Some(ScalarView::Bool(b))) if *a == b
            );
            if satisfied {
                return None;
            }
            let actual = match scalar {
                Some(ScalarView::Str(v)) => format!("\"{}\"", v),
                Some(ScalarView::Int(v)) => v.to_string(),
                Some(ScalarView::Bool(v)) => v.to_string(),
                None => type_name.to_string(),
            };
            return Some((lit.display(), actual));
        }
        // Unions next: satisfied by any branch (checked in full, so a
        // Result branch's payload or a literal branch's value applies
        // inside a union too).
        if let Self::Union(branches) = self {
            if branches.iter().any(|b| {
                b.check_value(type_name, result_payload, fn_arity, scalar)
                    .is_none()
            }) {
                return None;
            }
            let actual = match (scalar, result_payload) {
                (Some(ScalarView::Str(v)), _) => format!("\"{}\"", v),
                (Some(ScalarView::Int(v)), _) => v.to_string(),
                (Some(ScalarView::Bool(v)), _) => v.to_string(),
                (None, Some((true, p))) => format!("Ok({})", p),
                (None, Some((false, p))) => format!("Err({})", p),
                (None, None) => type_name.to_string(),
            };
            return Some((self.display_name(), actual));
        }
        if !self.accepts(type_name) {
            // Function expectations name the signature, not "Function" —
            // the signature is the information that matters.
            let expected = match self {
                Self::Function { .. } => self.display_name(),
                other => other.expected_name().to_string(),
            };
            return Some((expected, type_name.to_string()));
        }
        if let (Self::Function { arity, .. }, Some((required, total))) = (self, fn_arity)
            && !(required <= *arity && *arity <= total)
        {
            let desc = if required == total {
                format!(
                    "a function taking {} parameter{}",
                    total,
                    if total == 1 { "" } else { "s" }
                )
            } else {
                format!("a function taking {} to {} parameters", required, total)
            };
            return Some((self.display_name(), desc));
        }
        if let (Self::Result { ok, err }, Some((is_ok, payload))) = (self, result_payload) {
            let side = if is_ok { ok } else { err };
            if let Some(check) = side
                && !check.accepts(payload)
            {
                return Some((
                    self.display_name(),
                    format!("{}({})", if is_ok { "Ok" } else { "Err" }, payload),
                ));
            }
        }
        None
    }
}

/// Reduce a parameter list to its runtime checks, position-aligned.
/// `type_params` are the declaration's generic parameters (erased, never
/// checked). Entirely-unannotated lists produce all-None cheaply.
pub fn param_checks_of(
    parameters: &[Parameter],
    type_params: &[String],
) -> Vec<Option<FieldTypeCheck>> {
    parameters
        .iter()
        .map(|p| {
            p.type_annotation
                .as_ref()
                .and_then(|ann| FieldTypeCheck::from_annotation(ann, type_params))
        })
        .collect()
}

/// Reduce a declared return annotation to its runtime check.
pub fn return_check_of(
    ret: Option<&TypeAnnotation>,
    type_params: &[String],
) -> Option<FieldTypeCheck> {
    ret.and_then(|ann| FieldTypeCheck::from_annotation(ann, type_params))
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDecl {
    pub name: String,
    /// 1-based (line, column) of the type name in source.
    #[serde(default)]
    pub name_span: Option<(u32, u32)>,
    pub type_params: Vec<String>, // Type parameters for generic types
    pub definition: TypeDefinition,
}

// Declaration equality is *semantic*: `name_span` is source metadata
// (where the name sat in the file), not part of what was declared.
// Ignoring it is what lets `olang fmt`'s safety gate compare the AST
// before and after a whitespace change that shifts line numbers — the
// same philosophy as Statement's eq unwrapping Located.
impl PartialEq for LetDecl {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern
            && self.type_annotation == other.type_annotation
            && self.value == other.value
            && self.mutable == other.mutable
    }
}

impl PartialEq for FunctionDecl {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.type_params == other.type_params
            && self.type_param_bounds == other.type_param_bounds
            && self.parameters == other.parameters
            && self.return_type == other.return_type
            && self.body == other.body
    }
}

impl PartialEq for TypeDecl {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.type_params == other.type_params
            && self.definition == other.definition
    }
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
