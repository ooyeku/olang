use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// Represents a complete Olang program
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    ImportDecl(ImportDecl),
    ExportDecl(ExportDecl),
}

/// Error type declaration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorTypeDecl {
    pub name: String,
    pub fields: Vec<StructField>,
}

/// Variable declaration with optional type annotation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LetDecl {
    pub name: String,
    pub type_annotation: Option<TypeAnnotation>,
    pub value: Option<Expr>,
}

/// Function declaration for named/recursive functions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionDecl {
    pub name: String,
    pub type_params: Vec<String>, // Type parameters for generic functions
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
    String(Rc<String>),
    Boolean(bool),
    List(Rc<[Expr]>),
    Tuple(Rc<Vec<Expr>>),

    // Variables and calls
    Identifier(String),
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
    WhileLoop {
        condition: Box<Expr>,
        body: Box<Expr>,
    },
    Loop {
        body: Box<Expr>,
    },
    Break,
    Continue,

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
    All(Vec<Expr>),   // Promise.all([...])
    Race(Vec<Expr>),  // Promise.race([...])
    Spawn(Box<Expr>), // spawn async_expr

    // New string literal variants
    RawString(Rc<String>),
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
}

/// Argument types for function calls
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Argument {
    Positional(Expr),
    Named {
        name: String,
        value: Expr,
    },
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
    Unit,                              // Simple variant: Red
    Tuple(Vec<Value>),                // Tuple variant: Point(x, y)
    Struct(HashMap<String, Value>),   // Struct variant: Person { name, age }
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

    // Promise values for async operations
    Promise {
        state: PromiseState,
        value: Option<Box<Value>>,
        error: Option<Box<Value>>,
    },
}

// Enable parallel processing by implementing Send and Sync for Value
unsafe impl Send for Value {}
unsafe impl Sync for Value {}

/// Function value
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Function {
    pub name: Option<String>,
    pub parameters: Vec<Parameter>,
    pub body: Expr,
    pub closure: HashMap<String, Value>,
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

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Integer(n) => write!(f, "{}", n),
            Value::Float(x) => write!(f, "{}", x),
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
            Value::Promise {
                state,
                value,
                error,
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
            Value::Promise { .. } => "Promise".to_string(),
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
