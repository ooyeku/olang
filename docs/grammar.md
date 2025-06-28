# Olang Grammar Proposals

This document outlines proposed additions to the Olang programming language grammar, with detailed implementation plans based on the current codebase structure.

## Current Architecture Overview

Olang uses a Pest-based parser with the following key components:
- **Grammar**: `grammar.pest` - Defines syntax rules
- **AST**: `src/ast.rs` - Abstract syntax tree structures
- **Parser**: `src/parser.rs` - Converts tokens to AST nodes
- **Interpreter**: `src/interpreter.rs` - Executes AST nodes

## 1. Pipeline Fusion Syntax

### Proposal
Add explicit fusion operators to control lazy evaluation optimization:

```olang
// Force fusion (guaranteed optimization)
list |> map(f) |>! filter(g)

// Suggest fusion (hint to optimizer)
list |> map(f) |>? filter(g)

// Normal pipeline (current behavior)
list |> map(f) |> filter(g)
```

### Implementation Plan

#### Grammar Changes (`grammar.pest`)
```pest
// Add new pipeline operators
pipe_op = { "|>" | "|>!" | "|>?" }

// Update pipe_expr to handle fusion hints
pipe_expr = { range_expr ~ (pipe_op ~ range_expr)* }
```

#### AST Changes (`src/ast.rs`)
```rust
// Add fusion hint enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FusionHint {
    Normal,    // |> (current behavior)
    Force,     // |>! (guaranteed fusion)
    Suggest,   // |>? (optimization hint)
}

// Update Pipeline expression
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pipeline {
    pub left: Box<Expr>,
    pub right: Box<Expr>,
    pub fusion_hint: FusionHint,
}
```

#### Parser Changes (`src/parser.rs`)
```rust
fn build_pipe_expr(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
    let mut current = self.build_range_expr(pairs.next().unwrap().into_inner())?;
    
    while let Some(op_pair) = pairs.next() {
        let fusion_hint = match op_pair.as_str() {
            "|>" => FusionHint::Normal,
            "|>!" => FusionHint::Force,
            "|>?" => FusionHint::Suggest,
            _ => return Err(ParseError::InvalidSyntax { message: "Invalid pipeline operator".to_string() })
        };
        
        let right = self.build_range_expr(pairs.next().unwrap().into_inner())?;
        current = Expr::Pipeline {
            left: Box::new(current),
            right: Box::new(right),
            fusion_hint,
        };
    }
    
    Ok(current)
}
```

#### Interpreter Changes (`src/interpreter.rs`)
```rust
fn eval_pipeline(&mut self, left: Expr, right: Expr, fusion_hint: FusionHint) -> Result<Value, InterpreterError> {
    match fusion_hint {
        FusionHint::Force => {
            // Always attempt fusion
            self.force_fusion_evaluation(left, right)
        }
        FusionHint::Suggest => {
            // Try fusion, fallback to normal if not possible
            self.suggest_fusion_evaluation(left, right)
        }
        FusionHint::Normal => {
            // Current behavior
            self.normal_pipeline_evaluation(left, right)
        }
    }
}
```

## 2. Lazy Evaluation Control

### Proposal
Add explicit lazy/eager control keywords:

```olang
// Force lazy evaluation
lazy list |> map(f) |> filter(g)

// Force eager evaluation
eager list |> map(f) |> filter(g)

// Lazy blocks
lazy {
    range(1, 1000000) |> map(square) |> filter(even)
}
```

### Implementation Plan

#### Grammar Changes (`grammar.pest`)
```pest
// Add lazy/eager keywords
lazy_keyword = { "lazy" }
eager_keyword = { "eager" }

// Update primary expressions
primary = { 
    lazy_keyword ~ (block | expr) |
    eager_keyword ~ (block | expr) |
    lambda | async_expr | await_expr | promise_expr | 
    all_expr | race_expr | spawn_expr | match_expr | 
    if_expr | try_catch_expr | struct_literal | 
    literal | identifier | "(" ~ expr ~ ")" | block 
}
```

#### AST Changes (`src/ast.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EvaluationMode {
    Lazy,
    Eager,
    Auto, // Current behavior
}

// Add new expression variants
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Expr {
    // ... existing variants ...
    
    // Lazy evaluation control
    LazyBlock {
        body: Box<Expr>,
    },
    EagerBlock {
        body: Box<Expr>,
    },
}
```

#### Parser Changes (`src/parser.rs`)
```rust
fn build_primary(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
    if let Some(pair) = pairs.next() {
        match pair.as_rule() {
            Rule::lazy_keyword => {
                let body = self.build_expr(pairs)?;
                Ok(Expr::LazyBlock { body: Box::new(body) })
            }
            Rule::eager_keyword => {
                let body = self.build_expr(pairs)?;
                Ok(Expr::EagerBlock { body: Box::new(body) })
            }
            // ... existing cases ...
        }
    }
    // ... rest of implementation
}
```

## 3. Enhanced Pattern Matching

### Proposal
Add destructuring patterns and guard clauses:

```olang
// Destructuring patterns
match list {
    [head, ..tail] => head + sum(tail),
    [first, second, ..] => first + second,
    [] => 0
}

// Guard clauses
match x {
    n if n > 0 => "positive",
    n if n < 0 => "negative",
    _ => "zero"
}
```

### Implementation Plan

#### Grammar Changes (`grammar.pest`)
```pest
// Add rest pattern and guard clauses
pattern = { 
    result_pattern | struct_pattern | integer | string | boolean | 
    wildcard | list_pattern | tuple_pattern | identifier | 
    enum_variant_pattern | rest_pattern | guard_pattern 
}

rest_pattern = { ".." ~ identifier? }
guard_pattern = { pattern ~ "if" ~ expr }

// Update list patterns to support rest
list_pattern = { 
    "[" ~ (pattern ~ ("," ~ pattern)*)? ~ rest_pattern? ~ "]" 
}
```

#### AST Changes (`src/ast.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Pattern {
    // ... existing variants ...
    
    // New pattern variants
    Rest(Option<String>), // .. or ..tail
    Guard {
        pattern: Box<Pattern>,
        condition: Box<Expr>,
    },
}
```

#### Parser Changes (`src/parser.rs`)
```rust
fn build_pattern(&self, mut pairs: Pairs<Rule>) -> Result<Pattern, ParseError> {
    match pairs.next().unwrap().as_rule() {
        Rule::rest_pattern => {
            let rest_name = pairs.next().map(|p| p.as_str().to_string());
            Ok(Pattern::Rest(rest_name))
        }
        Rule::guard_pattern => {
            let pattern = self.build_pattern(pairs.next().unwrap().into_inner())?;
            let condition = self.build_expr(pairs.next().unwrap().into_inner())?;
            Ok(Pattern::Guard {
                pattern: Box::new(pattern),
                condition: Box::new(condition),
            })
        }
        // ... existing cases ...
    }
}
```

## 4. List Comprehensions

### Proposal
Add list comprehension syntax:

```olang
// Basic list comprehension
[x * 2 for x in range(1, 10) if x % 2 == 0]

// Multiple generators
[x + y for x in [1,2,3] for y in [4,5,6]]

// Generator expressions (lazy by default)
(x * 2 for x in range(1, 1000000) if x % 2 == 0)
```

### Implementation Plan

#### Grammar Changes (`grammar.pest`)
```pest
// List comprehension syntax
list_comprehension = { 
    "[" ~ expr ~ "for" ~ identifier ~ "in" ~ expr ~ 
    ("if" ~ expr)? ~ "]" 
}

generator_comprehension = { 
    "(" ~ expr ~ "for" ~ identifier ~ "in" ~ expr ~ 
    ("if" ~ expr)? ~ ")" 
}

// Multiple generators
list_comprehension = { 
    "[" ~ expr ~ comprehension_clause+ ~ "]" 
}

comprehension_clause = { 
    "for" ~ identifier ~ "in" ~ expr ~ ("if" ~ expr)? 
}

// Update primary expressions
primary = { 
    list_comprehension | generator_comprehension |
    // ... existing variants ...
}
```

#### AST Changes (`src/ast.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComprehensionClause {
    pub variable: String,
    pub iterable: Box<Expr>,
    pub condition: Option<Box<Expr>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Expr {
    // ... existing variants ...
    
    ListComprehension {
        expression: Box<Expr>,
        clauses: Vec<ComprehensionClause>,
    },
    GeneratorComprehension {
        expression: Box<Expr>,
        clauses: Vec<ComprehensionClause>,
    },
}
```

#### Parser Changes (`src/parser.rs`)
```rust
fn build_list_comprehension(&self, mut pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
    let expression = self.build_expr(pairs.next().unwrap().into_inner())?;
    let mut clauses = Vec::new();
    
    while let Some(clause_pair) = pairs.next() {
        if clause_pair.as_rule() == Rule::comprehension_clause {
            clauses.push(self.build_comprehension_clause(clause_pair.into_inner())?);
        }
    }
    
    Ok(Expr::ListComprehension {
        expression: Box::new(expression),
        clauses,
    })
}
```

## 5. Type System Enhancements

### Proposal
Add generic types and type constraints:

```olang
// Generic functions
fn map<T, U>(list: List<T>, f: fn(T) -> U) -> List<U> {
    // implementation
}

// Type constraints
fn sum<T: Numeric>(list: List<T>) -> T {
    // implementation
}

// Associated types
trait Iterator {
    type Item;
    fn next() -> Option<Self::Item>;
}
```

### Implementation Plan

#### Grammar Changes (`grammar.pest`)
```pest
// Type constraints
type_constraint = { 
    identifier ~ ":" ~ trait_name ~ ("+" ~ trait_name)* 
}

// Associated types
associated_type = { 
    "type" ~ identifier ~ ("=" ~ type_annotation)? 
}

// Trait definitions
trait_decl = { 
    "trait" ~ identifier ~ type_params? ~ "{" ~ 
    (associated_type | function_decl)* ~ "}" 
}

// Update function declarations
function_decl = { 
    "fn" ~ identifier ~ type_params? ~ "(" ~ param_list? ~ ")" ~ 
    ("->" ~ type_annotation)? ~ ("where" ~ type_constraint_list)? ~ 
    "=" ~ expr 
}
```

#### AST Changes (`src/ast.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeConstraint {
    pub type_var: String,
    pub traits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssociatedType {
    pub name: String,
    pub bound: Option<TypeAnnotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraitDecl {
    pub name: String,
    pub type_params: Vec<String>,
    pub items: Vec<TraitItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TraitItem {
    Function(FunctionDecl),
    AssociatedType(AssociatedType),
}

// Update FunctionDecl
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionDecl {
    pub name: String,
    pub type_params: Vec<String>,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub where_clause: Option<Vec<TypeConstraint>>,
    pub body: Expr,
}
```

## 6. String Interpolation

### Proposal
Add string interpolation syntax:

```olang
let name = "Alice";
let age = 30;
let message = "Hello, {name}! You are {age} years old.";

// Expression interpolation
let result = "The sum is {2 + 3}";
```

### Implementation Plan

#### Grammar Changes (`grammar.pest`)
```pest
// String interpolation
string = @{ 
    "\"" ~ (string_char | interpolation)* ~ "\"" 
}

interpolation = { 
    "{" ~ expr ~ "}" 
}

string_char = { 
    !("\"" | "\\" | "{") ~ ANY | 
    "\\" ~ ("\"" | "\\" | "/" | "b" | "f" | "n" | "r" | "t" | "u" ~ ASCII_HEX_DIGIT{4}) 
}
```

#### AST Changes (`src/ast.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StringPart {
    Literal(String),
    Interpolation(Box<Expr>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Expr {
    // ... existing variants ...
    
    InterpolatedString {
        parts: Vec<StringPart>,
    },
}
```

#### Parser Changes (`src/parser.rs`)
```rust
fn build_string(&self, pairs: Pairs<Rule>) -> Result<Expr, ParseError> {
    let mut parts = Vec::new();
    
    for pair in pairs {
        match pair.as_rule() {
            Rule::string_char => {
                let literal = self.process_string_escapes(pair.as_str())?;
                parts.push(StringPart::Literal(literal));
            }
            Rule::interpolation => {
                let expr = self.build_expr(pair.into_inner())?;
                parts.push(StringPart::Interpolation(Box::new(expr)));
            }
            _ => {}
        }
    }
    
    if parts.len() == 1 {
        if let StringPart::Literal(s) = &parts[0] {
            return Ok(Expr::String(Rc::new(s.clone())));
        }
    }
    
    Ok(Expr::InterpolatedString { parts })
}
```

## 7. Module System

### Proposal
Add proper module declarations and imports:

```olang
// Module declarations
module math {
    pub fn add(a: Int, b: Int) -> Int { a + b }
    fn internal_helper() { /* private */ }
}

// Import with renaming
import math::{add as plus, subtract as minus}
```

### Implementation Plan

#### Grammar Changes (`grammar.pest`)
```pest
// Module declarations
module_decl = { 
    "module" ~ identifier ~ "{" ~ (statement ~ ";")* ~ "}" 
}

// Enhanced import syntax
import_decl = { 
    "import" ~ (import_items ~ "from")? ~ string 
}

import_items = { 
    "{" ~ import_item ~ ("," ~ import_item)* ~ "}" | "*" 
}

import_item = { 
    identifier ~ ("as" ~ identifier)? 
}

// Update statements
statement = { 
    module_decl | error_type_decl | type_decl | let_decl | 
    async_function_decl | function_decl | import_decl | 
    export_decl | expr 
}
```

#### AST Changes (`src/ast.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModuleDecl {
    pub name: String,
    pub statements: Vec<Statement>,
}

// Update Statement enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Statement {
    // ... existing variants ...
    ModuleDecl(ModuleDecl),
}

// Update ImportDecl
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportDecl {
    pub items: Option<Vec<ImportItem>>,
    pub module_path: String,
}
```

## 8. Macro System

### Proposal
Add a basic macro system:

```olang
// Macro definitions
macro_rules! vec {
    ($($x:expr),*) => {
        [$($x),*]
    };
}

// Usage
let list = vec![1, 2, 3, 4];
```

### Implementation Plan

#### Grammar Changes (`grammar.pest`)
```pest
// Macro definitions
macro_decl = { 
    "macro_rules!" ~ identifier ~ "{" ~ macro_rule+ ~ "}" 
}

macro_rule = { 
    "(" ~ macro_pattern ~ ")" ~ "=>" ~ "(" ~ expr ~ ")" 
}

macro_pattern = { 
    macro_token+ 
}

macro_token = { 
    "$(" ~ macro_repeat ~ ")" ~ macro_separator? | 
    identifier | literal | punctuation 
}

macro_repeat = { 
    "$" ~ identifier ~ ":" ~ macro_type ~ ("*" | "+" | "?") 
}

macro_type = { 
    "expr" | "ident" | "ty" | "pat" | "stmt" | "block" 
}

macro_separator = { 
    "," | ";" | "+" 
}

// Update statements
statement = { 
    macro_decl | module_decl | error_type_decl | type_decl | 
    let_decl | async_function_decl | function_decl | 
    import_decl | export_decl | expr 
}
```

#### AST Changes (`src/ast.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MacroDecl {
    pub name: String,
    pub rules: Vec<MacroRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MacroRule {
    pub pattern: Vec<MacroToken>,
    pub expansion: Box<Expr>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MacroToken {
    Literal(String),
    Identifier(String),
    Repetition {
        var: String,
        kind: MacroType,
        quantifier: MacroQuantifier,
        separator: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MacroType {
    Expr,
    Ident,
    Type,
    Pattern,
    Statement,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MacroQuantifier {
    ZeroOrMore, // *
    OneOrMore,  // +
    ZeroOrOne,  // ?
}

// Update Statement enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Statement {
    // ... existing variants ...
    MacroDecl(MacroDecl),
}
```

## 9. Error Handling Improvements

### Proposal
Add enhanced error handling with multiple error types:

```olang
// Try expressions with multiple error types
try {
    let file = open_file(path)?;
    let content = read_content(file)?;
    parse_json(content)?
} catch FileError(e) {
    log_error(e);
    default_value
} catch ParseError(e) {
    log_error(e);
    empty_object
}
```

### Implementation Plan

#### Grammar Changes (`grammar.pest`)
```pest
// Enhanced try-catch
try_catch_expr = { 
    "try" ~ block ~ catch_arm+ 
}

catch_arm = { 
    "catch" ~ identifier ~ "(" ~ identifier ~ ")" ~ block 
}

// Update existing try_catch_expr rule
try_catch_expr = { 
    "try" ~ block ~ "catch" ~ "(" ~ identifier ~ ")" ~ block |
    "try" ~ block ~ catch_arm+ 
}
```

#### AST Changes (`src/ast.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CatchArm {
    pub error_type: String,
    pub error_var: String,
    pub handler: Box<Expr>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Expr {
    // ... existing variants ...
    
    TryCatch {
        try_block: Box<Expr>,
        catch_arms: Vec<CatchArm>,
    },
}
```

## 10. Advanced Pattern Matching

### Proposal
Add record patterns and or patterns:

```olang
// Record patterns
match person {
    Person{name: "Alice", age: age if age > 18} => "adult",
    Person{name, age: 0..=17} => format!("minor: {}", name),
    _ => "unknown"
}

// Or patterns
match value {
    1 | 2 | 3 => "small",
    4..=10 => "medium",
    _ => "large"
}
```

### Implementation Plan

#### Grammar Changes (`grammar.pest`)
```pest
// Or patterns
or_pattern = { 
    pattern ~ ("|" ~ pattern)+ 
}

// Range patterns
range_pattern = { 
    expr ~ "..=" ~ expr | 
    expr ~ ".." ~ expr 
}

// Update pattern rule
pattern = { 
    or_pattern | range_pattern | result_pattern | 
    struct_pattern | integer | string | boolean | 
    wildcard | list_pattern | tuple_pattern | 
    identifier | enum_variant_pattern | rest_pattern | 
    guard_pattern 
}
```

#### AST Changes (`src/ast.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Pattern {
    // ... existing variants ...
    
    Or(Vec<Pattern>),
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
    },
}
```

## Implementation Priority

### High Priority (Immediate Value)
1. **String Interpolation** - Improves developer experience significantly
2. **Enhanced Pattern Matching** - Makes code more readable and expressive
3. **List Comprehensions** - Very common and expressive syntax
4. **Lazy Evaluation Control** - Leverages existing lazy system

### Medium Priority (Significant Enhancement)
5. **Module System** - Better code organization
6. **Pipeline Fusion Syntax** - Explicit optimization control
7. **Error Handling Improvements** - Better error handling experience

### Lower Priority (Nice to Have)
8. **Macro System** - Advanced metaprogramming capabilities
9. **Type System Enhancements** - More type safety
10. **Advanced Pattern Matching** - Additional pattern matching features

## Testing Strategy

For each grammar addition:

1. **Grammar Tests**: Test parsing of new syntax
2. **AST Tests**: Verify correct AST construction
3. **Integration Tests**: Test end-to-end functionality
4. **Error Tests**: Test error handling for invalid syntax

## Migration Strategy

1. **Backward Compatibility**: All existing syntax should continue to work
2. **Gradual Rollout**: Implement features one at a time
3. **Documentation**: Update language documentation for each feature
4. **Examples**: Provide comprehensive examples for each new feature

## Conclusion

These grammar proposals would significantly enhance Olang's expressiveness and developer experience while maintaining the language's core philosophy of simplicity and functional programming principles. The implementation plans are designed to work within the existing codebase architecture and can be implemented incrementally. 