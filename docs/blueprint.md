# Olang Architecture Blueprint

## Executive Overview

Olang is a modern, functional programming language built in Rust that features a dual-execution architecture: a mature classic interpreter for complete language support and an advanced virtual machine (OVM) for high-performance execution. The project consists of three main components:

1. **Olang Language** - The core language implementation with functional programming features
2. **OVM (Olang Virtual Machine)** - Advanced runtime with JIT compilation and optimization
3. **OTC (Olang Tool Chain)** - Command-line tools and development utilities

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Olang Architecture                           │
├─────────────────────────────────────────────────────────────────────┤
│                    OTC (Command Line Interface)                     │
├─────────────────────────────────────────────────────────────────────┤
│                    Integration Layer System                         │
├──────────────────────────────┬──────────────────────────────────────┤
│     Classic Interpreter       │        OVM (Virtual Machine)        │
│   ┌─────────────────────┐    │    ┌──────────────────────────┐    │
│   │  Language Core      │    │    │  OVM Core System         │    │
│   │  Interpreter System │    │    │  OVM Optimization System │    │
│   │  Type System        │    │    │  Memory Management       │    │
│   │  Built-in Functions │    │    │  JIT Compilation         │    │
│   │  Standard Library   │    │    │  Bytecode VM             │    │
│   │  Async Runtime      │    │    │  Performance Monitoring  │    │
│   └─────────────────────┘    │    └──────────────────────────┘    │
├──────────────────────────────┴──────────────────────────────────────┤
│                Analysis System & REPL System                        │
└─────────────────────────────────────────────────────────────────────┘
```

## The 12 Main Systems

### 1. Language Core System
**Purpose**: Define the language structure and syntax
**Functionality**:
- Abstract Syntax Tree (AST) definitions
- Grammar rules using Pest parser
- Expression and statement types
- Pattern matching structures
- Type annotations

**Associated Files**:
- `grammar.pest` - Language grammar definition
- `src/ast.rs` - AST node definitions
- `src/parser.rs` - Pest-based parser implementation

### 2. Interpreter System
**Purpose**: Execute Olang programs using tree-walking interpretation
**Functionality**:
- Expression evaluation
- Environment management (variable scoping)
- Function call handling
- Control flow execution
- Pattern matching evaluation
- Error handling and reporting

**Associated Files**:
- `src/interpreter.rs` - Main interpreter implementation
- `src/repl.rs` - Basic REPL functionality

### 3. Type System
**Purpose**: Provide optional static type checking and inference
**Functionality**:
- Type checking for expressions and statements
- Type inference for unannotated code
- Generic type support
- Type compatibility verification
- Function type checking
- Promise and Result type handling

**Associated Files**:
- `src/type_checker.rs` - Type checking implementation
- `src/ast.rs` (TypeAnnotation, TypeContext definitions)

### 4. Standard Library System
**Purpose**: Provide essential functionality through 11 stdlib modules
**Functionality**:
- File system operations (fs)
- HTTP client/server (http)
- Mathematical functions (math)
- Date/time handling (dates)
- Random number generation (random)
- CSV processing (csv)
- JSON manipulation (json)
- Base64 encoding/decoding (base64)
- Operating system interface (os)
- Cryptographic functions (crypto)
- Testing utilities (testing)

**Associated Files**:
- `src/stdlib/mod.rs` - Module registry
- `src/stdlib/fs.rs` - Filesystem operations
- `src/stdlib/http.rs` - HTTP functionality
- `src/stdlib/math.rs` - Mathematical functions
- `src/stdlib/dates.rs` - Date/time operations
- `src/stdlib/random.rs` - Random generation
- `src/stdlib/csv.rs` - CSV processing
- `src/stdlib/json.rs` - JSON manipulation
- `src/stdlib/base64.rs` - Base64 encoding
- `src/stdlib/os.rs` - OS interface
- `src/stdlib/crypto.rs` - Cryptography
- `src/stdlib/testing.rs` - Testing utilities

### 5. Built-in Functions System
**Purpose**: Provide core language functions for common operations
**Functionality**:
- I/O operations (println, print)
- List operations (map, filter, reduce, fold)
- Type conversions (to_string, to_int, to_float)
- Utility functions (range, zip, typeof)
- List manipulation (reverse, sort, join, split)
- Mathematical aggregates (sum, average, min, max)
- Lazy evaluation functions (take, skip, force, lazy)
- Parallel control (set_parallel)

**Associated Files**:
- `src/builtin.rs` - Built-in function implementations
- `src/help.rs` - Function documentation

### 6. OVM Core System
**Purpose**: Provide the foundation for high-performance execution
**Functionality**:
- Virtual machine lifecycle management
- Core execution engine with tiered compilation
- Memory management integration
- Lazy evaluation engine
- Pipeline processing engine
- Performance metrics collection
- Adaptive optimization control

**Associated Files**:
- `src/ovm/mod.rs` - Main OVM module
- `src/ovm/execution.rs` - Execution engine
- `src/ovm/value.rs` - OVM value system
- `src/ovm/config.rs` - Configuration management
- `src/ovm/metrics.rs` - Performance monitoring

### 7. OVM Optimization System
**Purpose**: Enhance performance through advanced optimization techniques
**Functionality**:
- JIT compilation using Cranelift
- Bytecode compilation and VM
- Profile-guided optimization
- Function inlining and specialization
- Pipeline fusion optimization
- SIMD vectorization
- Adaptive optimization strategies
- Machine learning guided optimization

**Associated Files**:
- `src/ovm/optimization.rs` - Optimization engine
- `src/ovm/bytecode.rs` - Bytecode VM
- `src/ovm/adaptive.rs` - Adaptive optimization
- `src/ovm/fusion.rs` - Pipeline fusion
- `src/ovm/simd.rs` - SIMD operations
- `src/jit.rs` - Legacy JIT interface

### 8. Integration Layer System
**Purpose**: Intelligently dispatch between classic interpreter and OVM
**Functionality**:
- Execution routing decisions
- Automatic fallback handling
- Performance statistics tracking
- Function registration management
- Builtin function routing
- Configuration management
- Health monitoring

**Associated Files**:
- `src/ovm_integration.rs` - Integration layer
- `src/ovm_repl.rs` - Enhanced REPL with OVM

### 9. Async Runtime System
**Purpose**: Support asynchronous programming with async/await
**Functionality**:
- Task scheduling and execution
- Promise creation and resolution
- Async function handling
- Promise.all and Promise.race operations
- Delayed execution (Promise.delay)
- Runtime statistics and monitoring

**Associated Files**:
- `src/async_runtime.rs` - Async runtime implementation
- `src/ast.rs` (PromiseState, async expression types)

### 10. Analysis System
**Purpose**: Provide static analysis and code optimization
**Functionality**:
- Variable usage analysis
- Dead code detection
- Pattern exhaustiveness checking
- Type inference
- Undefined variable detection
- Duplicate variable checking
- Scope analysis

**Associated Files**:
- `src/analyze.rs` - Static analysis implementation

### 11. REPL System
**Purpose**: Provide interactive development environment
**Functionality**:
- Interactive code execution
- Command history (rustyline)
- Multi-line input support
- Error display and recovery
- OVM integration for enhanced performance
- Execution mode switching (classic/OVM)
- Performance monitoring

**Associated Files**:
- `src/repl.rs` - Basic REPL
- `src/ovm_repl.rs` - Enhanced OVM REPL

### 12. Toolchain System (OTC)
**Purpose**: Provide command-line tools for development
**Functionality**:
- Program execution (run command)
- Static analysis (check command)
- REPL launching
- OVM execution with optimization levels
- Version information
- Verbose output control
- Parallel processing initialization

**Associated Files**:
- `otc/src/main.rs` - CLI entry point
- `otc/src/commands/mod.rs` - Command definitions
- `otc/src/commands/run.rs` - Run command
- `otc/src/commands/check.rs` - Check command
- `otc/src/commands/repl.rs` - REPL command
- `otc/src/commands/ovm.rs` - OVM command
- `otc/src/commands/version.rs` - Version command

## Supporting Components

### Internal Systems
**Purpose**: Provide internal functionality not exposed to users
**Files**:
- `src/internal/mod.rs` - Lazy evaluation internals
- `src/internal/tests.rs` - Internal tests
- `src/parallel.rs` - Parallelization utilities
- `src/help.rs` - Help system
- `src/version.rs` - Version management

### Memory Management Subsystem
**Part of**: OVM Core System
**Purpose**: Advanced garbage collection and memory optimization
**Files**:
- `src/ovm/memory.rs` - Memory manager
- `src/ovm/gc.rs` - Garbage collector
- `src/ovm/lazy.rs` - Lazy evaluation memory management

### Configuration System
**Purpose**: Manage system-wide configuration
**Files**:
- `src/ovm/config.rs` - OVM configuration
- `Cargo.toml` - Project dependencies
- `otc/Cargo.toml` - OTC dependencies

## Execution Flow

1. **Source Code** → Grammar Parser → AST Generation
2. **AST** → Integration Layer → Routing Decision
3. **Classic Path**: AST → Interpreter → Environment → Result
4. **OVM Path**: AST → OVM → Optimization → Execution → Result
5. **Fallback**: OVM Error → Classic Interpreter → Result

## Key Design Principles

1. **Dual Execution Model**: Classic interpreter for compatibility, OVM for performance
2. **Intelligent Routing**: Automatic selection of best execution path
3. **Graceful Fallback**: Always maintain functionality via classic interpreter
4. **Modular Architecture**: Clear separation of concerns
5. **Performance First**: OVM provides significant speedups for suitable workloads
6. **Developer Experience**: Comprehensive REPL and tooling support
7. **Type Safety**: Optional but powerful type system
8. **Functional Paradigm**: First-class functions, immutability, pipelines

## Performance Characteristics

- **Classic Interpreter**: Baseline 1x performance, complete feature support
- **OVM Execution**: 2-12x performance improvement for suitable workloads
- **Memory Usage**: OVM provides up to 92% reduction for lazy operations
- **JIT Compilation**: Hot functions compiled to native code
- **Pipeline Fusion**: Automatic optimization of chained operations
- **Parallel Execution**: Automatic parallelization for data operations

## Future Evolution

The architecture is designed to support:
- WebAssembly compilation target
- Distributed execution
- GPU acceleration
- Neural network compilation
- Quantum computing integration
- Advanced type system features
- Package management system
