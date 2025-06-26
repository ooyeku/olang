    # Olang Virtual Machine (OVM) Design Specification

## Executive Summary

The **Olang Virtual Machine (OVM)** is a runtime system that provides garbage collection, lazy evaluation, and performance optimization for the Olang programming language. OVM aims to provide performance improvements while maintaining Olang's functional programming semantics.

> **Implementation Status**: 🚧 **DEVELOPMENT PHASE** - The OVM infrastructure has been implemented with core components operational. Several advanced features are in various stages of development and testing.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Core Components](#core-components)
3. [Memory Management](#memory-management)
4. [Execution Engine](#execution-engine)
5. [Implementation Status](#implementation-status)
6. [Integration](#integration)
7. [Usage Examples](#usage-examples)
8. [Configuration](#configuration)

## Architecture Overview

### System Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Olang Virtual Machine (OVM)                 │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │  Execution      │  │  Memory         │  │  Optimization   │ │
│  │  Engine         │  │  Manager        │  │  Engine         │ │
│  │                 │  │                 │  │                 │ │
│  │ • Interpreter ✅│  │ • GC System ✅  │  │ • Profiler ✅   │ │
│  │ • Bytecode VM🚧 │  │ • Allocator ✅  │  │ • Adaptive 🚧   │ │
│  │ • Dispatch ✅   │  │ • Barriers ✅   │  │ • SIMD ✅       │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │  Lazy           │  │  Pipeline       │  │  Integration    │ │
│  │  Evaluation ✅  │  │  Engine ✅      │  │  Layer ✅       │ │
│  │                 │  │                 │  │                 │ │
│  │ • Thunks ✅     │  │ • Operations ✅ │  │ • REPL ✅       │ │
│  │ • Streams ✅    │  │ • Fusion 🚧     │  │ • CLI ✅        │ │
│  │ • Memoization✅ │  │ • Vectorize 🚧  │  │ • Fallback ✅   │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
├─────────────────────────────────────────────────────────────────┤
│                        Value System ✅                        │
└─────────────────────────────────────────────────────────────────┘
```

**Legend**: ✅ Implemented | 🚧 In Development | ❌ Not Implemented

### Execution Model

Currently, OVM operates primarily in **Interpreter Mode** with:
- Garbage-collected memory management
- Lazy evaluation support
- Pipeline operation optimization
- Automatic fallback to classic interpreter when needed

## Core Components

### 1. OVM Core Engine

```rust
// src/ovm/mod.rs
pub struct OlangVirtualMachine {
    execution_engine: ExecutionEngine,
    memory_manager: MemoryManager,
    optimization_engine: OptimizationEngine,
    lazy_engine: LazyEngine,
    pipeline_engine: PipelineEngine,
    config: OvmConfig,
    metrics: Arc<Mutex<OvmMetrics>>,
}

#[derive(Debug, Clone)]
pub struct OvmConfig {
    pub memory: MemoryConfig,
    pub execution: ExecutionConfig,
    pub optimization: OptimizationConfig,
    pub lazy: LazyConfig,
    pub pipeline: PipelineConfig,
}

#[derive(Debug, Clone, Copy)]
pub enum OptimizationLevel {
    Debug,      // Minimal optimization
    Balanced,   // Moderate optimization
    Release,    // Aggressive optimization
}
```

### 2. Value System

```rust
// src/ovm/value.rs
#[repr(C)]
pub struct OvmValue {
    pub header: ValueHeader,
    pub data: ValueData,
}

#[repr(C)]
pub struct ValueHeader {
    pub gc_bits: AtomicU32,
    pub type_tag: TypeTag,
    pub lazy_state: LazyState,
    pub ref_count: AtomicU32,
}

#[derive(Debug, Clone, Copy)]
pub enum ValueData {
    // Immediate values
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Unit,
    
    // GC-managed values
    String(GcPtr<String>),
    List(GcPtr<ValueArray>),
    Function(GcPtr<FunctionObject>),
    
    // Lazy values
    Thunk(GcPtr<ThunkObject>),
    Stream(GcPtr<StreamObject>),
}

#[derive(Debug, Clone, Copy)]
pub enum LazyState {
    Eager,    // Value is computed
    Lazy,     // Value is not yet computed
    Forcing,  // Value is being computed
    Cached,   // Value was lazy but is now cached
}
```

### 3. Memory Manager

```rust
// src/ovm/memory.rs
pub struct MemoryManager {
    gc: ConcurrentGarbageCollector,
    heap: UnifiedHeap,
    allocator: TieredAllocator,
    lazy_manager: LazyMemoryManager,
}

pub struct ConcurrentGarbageCollector {
    marking_engine: ConcurrentMarkingEngine,
    sweeping_engine: IncrementalSweepingEngine,
    safepoint_manager: SafepointManager,
    write_barrier_manager: WriteBarrierManager,
}
```

### 4. Execution Engine

```rust
// src/ovm/execution.rs
pub struct ExecutionEngine {
    interpreter: Arc<Mutex<Interpreter>>,
    bytecode_vm: Arc<Mutex<BytecodeVm>>,
    optimization_engine: Arc<Mutex<OptimizationEngine>>,
    functions: HashMap<FunctionId, FunctionDecl>,
    execution_stats: HashMap<FunctionId, ExecutionStats>,
}

pub enum ExecutionTier {
    Interpreter,  // Current primary execution mode
    Bytecode,     // Infrastructure ready
    Native,       // Future development
}
```

## Memory Management

### Garbage Collector

The OVM includes a concurrent garbage collector with:

- **Concurrent Marking**: Tri-color marking algorithm with work stealing
- **Incremental Sweeping**: Low-pause collection with pause budgets  
- **Write Barriers**: Card table-based barrier system for generational collection
- **Lazy Integration**: Special handling for lazy evaluation constructs

```rust
impl ConcurrentGarbageCollector {
    pub fn collect_with_lazy_awareness(&self) -> GcResult {
        self.scan_lazy_roots();
        self.mark_with_lazy_boundaries();
        self.selective_lazy_forcing();
        self.sweep_phase();
        Ok(GcStats::new())
    }
}
```

### Memory Layout

- **Nursery**: Fast allocation for short-lived objects
- **Young Generation**: Recently allocated objects
- **Old Generation**: Long-lived objects
- **Large Object Space**: Objects > 32KB
- **Lazy Space**: Lazy evaluation metadata

## Execution Engine

### Current Implementation

The execution engine currently operates with:

1. **Interpreter Tier**: Primary execution mode with full Olang language support
2. **Bytecode Tier**: Infrastructure implemented, register-based VM with compiler
3. **Optimization**: Profiling and adaptive optimization framework

```rust
impl ExecutionEngine {
    pub fn execute_function(&mut self, func_id: FunctionId, args: &[OvmValue]) -> Result<OvmValue, ExecutionError> {
        // Currently uses interpreter with potential bytecode compilation
        match self.get_execution_tier(func_id) {
            ExecutionTier::Interpreter => self.execute_with_interpreter(func_id, args),
            ExecutionTier::Bytecode => self.execute_with_bytecode(func_id, args),
            _ => self.execute_with_interpreter(func_id, args) // Fallback
        }
    }
}
```

### Bytecode Virtual Machine

```rust
// src/ovm/bytecode.rs
pub struct BytecodeVm {
    compiler: BytecodeCompiler,
    bytecode_cache: Arc<RwLock<HashMap<FunctionId, CompiledBytecode>>>,
    execution_state: ExecutionState,
}

pub enum Instruction {
    LoadConst { dst: Register, const_idx: u32 },
    Add { dst: Register, lhs: Register, rhs: Register },
    Call { dst: Register, function: Register, args: Vec<Register> },
    Return { value: Option<Register> },
    // ... other instructions
}
```

## Implementation Status

### ✅ Completed Components

#### Core Infrastructure
- ✅ OVM main engine and lifecycle management
- ✅ Value system with GC integration
- ✅ Configuration system with presets
- ✅ Error handling and fallback mechanisms
- ✅ Integration with REPL and CLI tools

#### Memory Management  
- ✅ Concurrent garbage collector algorithms
- ✅ Write barriers and root scanning
- ✅ Lazy-aware memory management
- ✅ Memory allocation and heap organization

#### Execution System
- ✅ Interpreter execution with OVM values
- ✅ Function registration and execution
- ✅ Performance monitoring and statistics
- ✅ Tier management infrastructure

#### Advanced Features
- ✅ Lazy evaluation engine with thunks and streams
- ✅ Pipeline processing with basic optimization
- ✅ SIMD vectorization infrastructure
- ✅ Adaptive optimization framework

### 🚧 In Development

#### Bytecode Virtual Machine
- 🚧 Register-based VM (infrastructure complete)
- 🚧 Bytecode compiler (functional but basic)
- 🚧 Bytecode optimization passes
- 🚧 Integration with tier management

#### Advanced Optimization
- 🚧 JIT compilation with Cranelift backend
- 🚧 Advanced pipeline fusion optimization
- 🚧 Machine learning-driven optimization
- 🚧 Hardware-specific optimizations

#### Performance Enhancement
- 🚧 SIMD vectorization for array operations
- 🚧 Multi-threaded parallel execution
- 🚧 Advanced memory optimization
- 🚧 Real-time performance tuning

### ❌ Future Work

- ❌ Native code generation (JIT backend needs completion)
- ❌ Cross-platform SIMD optimization
- ❌ Distributed execution support
- ❌ Advanced debugging tools
