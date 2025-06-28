#![allow(dead_code)]

//! Register-based bytecode VM for intermediate-tier execution between interpreter and JIT

use crate::ast::{BinaryOp, Expr, FunctionDecl, UnaryOp, Value};
use crate::ovm::{FunctionId, OvmValue};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// Register-based bytecode virtual machine
pub struct BytecodeVm {
    // Bytecode compiler
    compiler: BytecodeCompiler,

    // Compiled bytecode cache
    bytecode_cache: Arc<RwLock<HashMap<FunctionId, CompiledBytecode>>>,

    // Runtime execution state
    execution_state: ExecutionState,

    // Performance statistics
    stats: VmStatistics,

    // Call stack for nested function calls
    call_stack: Vec<CallFrame>,

    // Exception handler stack
    exception_handlers: Vec<ExceptionHandler>,

    // Function registry for dynamic calls
    function_registry: HashMap<String, FunctionId>,

    // Builtin function registry
    builtin_registry: HashMap<String, u32>,
}

/// Call frame for function execution
#[derive(Debug, Clone)]
struct CallFrame {
    function_id: FunctionId,
    return_address: usize,
    base_register: usize,
    local_count: usize,
}

/// Exception handler for error recovery
#[derive(Debug, Clone)]
struct ExceptionHandler {
    handler_address: usize,
    stack_depth: usize,
}

/// Bytecode compiler that transforms AST to bytecode
pub struct BytecodeCompiler {
    // Register allocator
    register_allocator: RegisterAllocator,

    // Instruction emitter
    emitter: InstructionEmitter,

    // Optimization passes
    optimizer: BytecodeOptimizer,

    // Local variable tracking
    local_variables: HashMap<String, u32>,
    next_local_idx: u32,

    // Label tracking for control flow
    label_counter: u32,

    // Function registry for calls
    function_registry: HashMap<String, FunctionId>,
}

/// Bytecode optimization engine
pub struct BytecodeOptimizer {
    // Dead code elimination
    dead_code_eliminator: DeadCodeEliminator,

    // Register reuse optimization
    register_optimizer: RegisterOptimizer,

    // Control flow optimization
    control_flow_optimizer: ControlFlowOptimizer,

    // Constant folding
    constant_folder: ConstantFolder,

    // Peephole optimization
    peephole_optimizer: PeepholeOptimizer,
}

/// Compiled bytecode representation
#[derive(Debug, Clone)]
pub struct CompiledBytecode {
    pub function_id: FunctionId,
    pub instructions: Vec<Instruction>,
    pub register_count: u32,
    pub local_count: u32,
    pub constants: Vec<OvmValue>,
    pub debug_info: BytecodeDebugInfo,
    pub optimization_level: u8,
    pub entry_point: usize,
}

/// Bytecode instruction set - Enhanced with more operations
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    // Load/Store operations
    LoadConst {
        dst: Register,
        const_idx: u32,
    },
    LoadLocal {
        dst: Register,
        local_idx: u32,
    },
    StoreLocal {
        src: Register,
        local_idx: u32,
    },
    Move {
        dst: Register,
        src: Register,
    },

    // Arithmetic operations
    Add {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Sub {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Mul {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Div {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Mod {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Neg {
        dst: Register,
        src: Register,
    },

    // Comparison operations
    Eq {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Ne {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Lt {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Le {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Gt {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Ge {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },

    // Logical operations
    And {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Or {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Not {
        dst: Register,
        src: Register,
    },

    // Control flow
    Jump {
        target: Label,
    },
    JumpIfTrue {
        condition: Register,
        target: Label,
    },
    JumpIfFalse {
        condition: Register,
        target: Label,
    },

    // Function operations
    Call {
        dst: Register,
        function: Register,
        args: Vec<Register>,
        arg_count: u32,
    },
    CallBuiltin {
        dst: Register,
        builtin_id: u32,
        args: Vec<Register>,
    },
    CallNamed {
        dst: Register,
        function_name: String,
        args: Vec<Register>,
    },
    Return {
        value: Option<Register>,
    },

    // Collection operations
    MakeList {
        dst: Register,
        elements: Vec<Register>,
    },
    ListGet {
        dst: Register,
        list: Register,
        index: Register,
    },
    ListSet {
        list: Register,
        index: Register,
        value: Register,
    },
    ListLen {
        dst: Register,
        list: Register,
    },
    ListPush {
        list: Register,
        value: Register,
    },
    ListPop {
        dst: Register,
        list: Register,
    },

    // Tuple operations
    MakeTuple {
        dst: Register,
        elements: Vec<Register>,
    },
    TupleGet {
        dst: Register,
        tuple: Register,
        index: u32,
    },

    // String operations
    StringConcat {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    StringLen {
        dst: Register,
        src: Register,
    },
    StringSlice {
        dst: Register,
        src: Register,
        start: Register,
        end: Register,
    },

    // Type operations
    TypeOf {
        dst: Register,
        src: Register,
    },
    CheckType {
        dst: Register,
        src: Register,
        type_id: u32,
    },

    // Pipeline operations (Olang-specific)
    PipelineMap {
        dst: Register,
        source: Register,
        function: Register,
    },
    PipelineFilter {
        dst: Register,
        source: Register,
        predicate: Register,
    },
    PipelineReduce {
        dst: Register,
        source: Register,
        initial: Register,
        function: Register,
    },

    // Lazy evaluation operations
    MakeThunk {
        dst: Register,
        expr_idx: u32,
    },
    ForceThunk {
        dst: Register,
        thunk: Register,
    },

    // Exception handling
    TryBegin {
        handler: Label,
    },
    TryEnd,
    Throw {
        exception: Register,
    },

    // Memory operations
    Allocate {
        dst: Register,
        size: Register,
    },
    LoadField {
        dst: Register,
        object: Register,
        field_idx: u32,
    },
    StoreField {
        object: Register,
        field_idx: u32,
        value: Register,
    },

    // Debug operations
    Nop,
    DebugPrint {
        src: Register,
    },
    Breakpoint,
    ProfileEnter {
        function_id: u32,
    },
    ProfileExit {
        function_id: u32,
    },
}

/// Register identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Register(pub u32);

/// Label identifier for jumps
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label(pub u32);

/// Debug information for bytecode
#[derive(Debug, Clone)]
pub struct BytecodeDebugInfo {
    pub instruction_to_source: HashMap<usize, SourceLocation>,
    pub register_names: HashMap<Register, String>,
    pub function_name: Option<String>,
    pub local_variables: HashMap<u32, String>,
    pub line_table: Vec<(usize, u32)>, // (instruction_index, line_number)
}

/// Source location information
#[derive(Debug, Clone)]
pub struct SourceLocation {
    pub line: u32,
    pub column: u32,
    pub file: Option<String>,
}

/// VM execution state
#[derive(Debug)]
pub struct ExecutionState {
    // Register values
    registers: Vec<OvmValue>,

    // Local variables
    locals: Vec<OvmValue>,

    // Call stack
    call_stack: Vec<StackFrame>,

    // Program counter
    pc: usize,

    // Exception state
    exception: Option<VmException>,

    // Current bytecode being executed
    current_bytecode: Option<CompiledBytecode>,
}

/// Stack frame for function calls  
#[derive(Debug, Clone)]
pub struct StackFrame {
    pub function_id: FunctionId,
    pub return_pc: usize,
    pub return_register: Option<Register>,
    pub local_base: usize,
    pub register_base: usize,
}

/// VM performance statistics
#[derive(Debug, Default)]
pub struct VmStatistics {
    pub instructions_executed: u64,
    pub function_calls: u64,
    pub bytecode_cache_hits: u64,
    pub bytecode_cache_misses: u64,
    pub compilation_time: std::time::Duration,
    pub execution_time: std::time::Duration,
    pub optimization_time: std::time::Duration,
    pub memory_allocations: u64,
    pub gc_triggers: u64,
}

/// Register allocator for bytecode generation
pub struct RegisterAllocator {
    next_register: u32,
    free_registers: Vec<Register>,
    register_usage: HashMap<Register, RegisterUsage>,
    max_registers: u32,
}

#[derive(Debug, Clone)]
struct RegisterUsage {
    first_use: usize,
    last_use: usize,
    is_temporary: bool,
    live_range: Option<LiveRange>,
}

#[derive(Debug, Clone)]
struct LiveRange {
    start: usize,
    end: usize,
}

/// Instruction emitter
pub struct InstructionEmitter {
    instructions: Vec<Instruction>,
    labels: HashMap<String, Label>,
    unresolved_labels: HashMap<Label, Vec<usize>>, // Label -> instruction indices that reference it
    next_label: u32,
    constants: Vec<OvmValue>,
    constant_map: HashMap<String, u32>, // For deduplication
    current_line: u32,
    debug_info: BytecodeDebugInfo,
}

/// Bytecode optimization passes

pub struct DeadCodeEliminator {
    live_registers: std::collections::HashSet<Register>,
    live_instructions: std::collections::HashSet<usize>,
}

pub struct RegisterOptimizer {
    register_map: HashMap<Register, Register>,
    interference_graph: HashMap<Register, std::collections::HashSet<Register>>,
}

pub struct ControlFlowOptimizer {
    basic_blocks: Vec<BasicBlock>,
    cfg: ControlFlowGraph,
}

pub struct ConstantFolder {
    constant_values: HashMap<Register, OvmValue>,
}

pub struct PeepholeOptimizer {
    patterns: Vec<OptimizationPattern>,
}

#[derive(Debug, Clone)]
struct BasicBlock {
    id: usize,
    instructions: Vec<Instruction>,
    predecessors: Vec<usize>,
    successors: Vec<usize>,
    entry_point: usize,
    exit_point: usize,
}

#[derive(Debug, Clone)]
struct ControlFlowGraph {
    blocks: Vec<BasicBlock>,
    entry_block: usize,
    exit_blocks: Vec<usize>,
}

#[derive(Debug, Clone)]
struct OptimizationPattern {
    pattern: Vec<InstructionPattern>,
    replacement: Vec<Instruction>,
    condition: Option<fn(&[Instruction]) -> bool>,
}

#[derive(Debug, Clone)]
enum InstructionPattern {
    Exact(Instruction),
    Any,
    Register(String), // Named register for pattern matching
}

/// VM errors
#[derive(Debug, Error)]
pub enum BytecodeError {
    #[error("Compilation failed: {0}")]
    CompilationFailed(String),

    #[error("Runtime error: {0}")]
    RuntimeError(String),

    #[error("Invalid register: {0:?}")]
    InvalidRegister(Register),

    #[error("Invalid instruction at PC {pc}: {instruction:?}")]
    InvalidInstruction { pc: usize, instruction: Instruction },

    #[error("Stack overflow")]
    StackOverflow,

    #[error("Stack underflow")]
    StackUnderflow,

    #[error("Function not found: {0:?}")]
    FunctionNotFound(FunctionId),

    #[error("Type error: {0}")]
    TypeError(String),

    #[error("Invalid constant index: {0}")]
    InvalidConstantIndex(u32),

    #[error("Invalid local index: {0}")]
    InvalidLocalIndex(u32),

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Index out of bounds: {index} for length {length}")]
    IndexOutOfBounds { index: i64, length: usize },

    #[error("Unresolved label: {0:?}")]
    UnresolvedLabel(Label),

    #[error("Exception thrown: {0}")]
    ExceptionThrown(String),

    #[error("Named function not found: {0}")]
    NamedFunctionNotFound(String),
}

/// VM exception types
#[derive(Debug, Clone)]
pub enum VmException {
    RuntimeError(String),
    TypeError(String),
    StackOverflow,
    DivisionByZero,
    IndexOutOfBounds,
    NullPointerException,
    InvalidOperation(String),
}

// Default implementations
impl Default for BytecodeDebugInfo {
    fn default() -> Self {
        Self {
            instruction_to_source: HashMap::new(),
            register_names: HashMap::new(),
            function_name: None,
            local_variables: HashMap::new(),
            line_table: Vec::new(),
        }
    }
}

impl Default for Register {
    fn default() -> Self {
        Register(0)
    }
}

impl Default for Label {
    fn default() -> Self {
        Label(0)
    }
}

impl BytecodeVm {
    pub fn new() -> Self {
        let mut builtin_registry = HashMap::new();

        // Register common builtin functions
        builtin_registry.insert("len".to_string(), 0);
        builtin_registry.insert("toString".to_string(), 1);
        builtin_registry.insert("println".to_string(), 2);
        builtin_registry.insert("print".to_string(), 3);
        builtin_registry.insert("map".to_string(), 4);
        builtin_registry.insert("filter".to_string(), 5);
        builtin_registry.insert("reduce".to_string(), 6);
        builtin_registry.insert("range".to_string(), 7);

        Self {
            compiler: BytecodeCompiler::new(),
            bytecode_cache: Arc::new(RwLock::new(HashMap::new())),
            execution_state: ExecutionState::new(),
            stats: VmStatistics::default(),
            call_stack: Vec::new(),
            exception_handlers: Vec::new(),
            function_registry: HashMap::new(),
            builtin_registry,
        }
    }

    /// Register a function for dynamic calls
    pub fn register_function(&mut self, name: String, func_id: FunctionId) {
        self.function_registry.insert(name, func_id);
    }

    /// Check if function has compiled bytecode
    pub fn has_bytecode(&self, func_id: FunctionId) -> bool {
        if let Ok(cache) = self.bytecode_cache.read() {
            cache.contains_key(&func_id)
        } else {
            false
        }
    }

    /// Compile function to bytecode
    pub fn compile_function(
        &mut self,
        func_id: FunctionId,
        func: &FunctionDecl,
    ) -> Result<(), BytecodeError> {
        let start_time = std::time::Instant::now();

        // Set up function registry for the compiler
        self.compiler.function_registry = self.function_registry.clone();

        let bytecode = self.compiler.compile_function(func_id, func)?;

        if let Ok(mut cache) = self.bytecode_cache.write() {
            cache.insert(func_id, bytecode);
        }

        self.stats.compilation_time += start_time.elapsed();
        Ok(())
    }

    /// Execute function with bytecode
    pub fn execute(
        &mut self,
        func_id: FunctionId,
        args: &[OvmValue],
    ) -> Result<OvmValue, BytecodeError> {
        // Get bytecode from cache
        let bytecode = {
            if let Ok(cache) = self.bytecode_cache.read() {
                cache.get(&func_id).cloned()
            } else {
                None
            }
        };

        let bytecode = bytecode.ok_or(BytecodeError::FunctionNotFound(func_id))?;

        // Prepare execution state
        self.execution_state
            .prepare_for_execution(&bytecode, args)?;

        // Record cache hit
        self.stats.bytecode_cache_hits += 1;
        self.stats.function_calls += 1;

        // Execute bytecode
        let start_time = std::time::Instant::now();
        let result = self.execute_bytecode(&bytecode)?;
        self.stats.execution_time += start_time.elapsed();

        Ok(result)
    }

    /// Execute bytecode instructions - Complete implementation
    fn execute_bytecode(&mut self, bytecode: &CompiledBytecode) -> Result<OvmValue, BytecodeError> {
        let mut pc = bytecode.entry_point;
        self.execution_state.current_bytecode = Some(bytecode.clone());

        while pc < bytecode.instructions.len() {
            let instruction = &bytecode.instructions[pc];
            self.stats.instructions_executed += 1;

            match instruction {
                Instruction::LoadConst { dst, const_idx } => {
                    let value = bytecode
                        .constants
                        .get(*const_idx as usize)
                        .ok_or(BytecodeError::InvalidConstantIndex(*const_idx))?;
                    self.execution_state.set_register(*dst, value.clone())?;
                }

                Instruction::LoadLocal { dst, local_idx } => {
                    let value = self.execution_state.get_local(*local_idx)?;
                    self.execution_state.set_register(*dst, value)?;
                }

                Instruction::StoreLocal { src, local_idx } => {
                    let value = self.execution_state.get_register(*src)?;
                    self.execution_state.set_local(*local_idx, value)?;
                }

                Instruction::Move { dst, src } => {
                    let value = self.execution_state.get_register(*src)?;
                    self.execution_state.set_register(*dst, value)?;
                }

                // Arithmetic operations
                Instruction::Add { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_binary_op(&left, &right, BinaryOp::Add)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Sub { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_binary_op(&left, &right, BinaryOp::Subtract)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Mul { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_binary_op(&left, &right, BinaryOp::Multiply)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Div { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_binary_op(&left, &right, BinaryOp::Divide)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Mod { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_binary_op(&left, &right, BinaryOp::Modulo)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Neg { dst, src } => {
                    let value = self.execution_state.get_register(*src)?;
                    let result = self.execute_unary_op(&value, UnaryOp::Negate)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                // Comparison operations
                Instruction::Eq { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_binary_op(&left, &right, BinaryOp::Equal)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Ne { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_binary_op(&left, &right, BinaryOp::NotEqual)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Lt { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_binary_op(&left, &right, BinaryOp::LessThan)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Le { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_binary_op(&left, &right, BinaryOp::LessThanEqual)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Gt { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_binary_op(&left, &right, BinaryOp::GreaterThan)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Ge { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result =
                        self.execute_binary_op(&left, &right, BinaryOp::GreaterThanEqual)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                // Logical operations
                Instruction::And { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_logical_and(&left, &right)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Or { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_logical_or(&left, &right)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Not { dst, src } => {
                    let value = self.execution_state.get_register(*src)?;
                    let result = self.execute_unary_op(&value, UnaryOp::Not)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                // Control flow
                Instruction::Jump { target } => {
                    pc = target.0 as usize;
                    continue;
                }

                Instruction::JumpIfTrue { condition, target } => {
                    let cond_value = self.execution_state.get_register(*condition)?;
                    if self.is_truthy(&cond_value) {
                        pc = target.0 as usize;
                        continue;
                    }
                }

                Instruction::JumpIfFalse { condition, target } => {
                    let cond_value = self.execution_state.get_register(*condition)?;
                    if !self.is_truthy(&cond_value) {
                        pc = target.0 as usize;
                        continue;
                    }
                }

                Instruction::Return { value } => {
                    if let Some(reg) = value {
                        return Ok(self.execution_state.get_register(*reg)?);
                    } else {
                        return Ok(OvmValue::from_ast(Value::Unit));
                    }
                }

                // Function operations
                Instruction::Call {
                    dst,
                    function,
                    args,
                    ..
                } => {
                    let _func_value = self.execution_state.get_register(*function)?;
                    let mut arg_values = Vec::new();
                    for arg_reg in args {
                        arg_values.push(self.execution_state.get_register(*arg_reg)?);
                    }

                    // Create a call frame and handle function call
                    let call_frame = CallFrame {
                        function_id: FunctionId::new(), // Placeholder for dynamic dispatch
                        return_address: pc + 1,
                        base_register: 0,
                        local_count: arg_values.len(),
                    };

                    self.call_stack.push(call_frame);

                    // For now, implement a basic function call mechanism
                    // In a complete implementation, this would handle dynamic dispatch
                    let result = self.execute_function_call(&arg_values)?;
                    self.execution_state.set_register(*dst, result)?;

                    self.call_stack.pop();
                }

                Instruction::CallBuiltin {
                    dst,
                    builtin_id,
                    args,
                } => {
                    let mut arg_values = Vec::new();
                    for arg_reg in args {
                        arg_values.push(self.execution_state.get_register(*arg_reg)?);
                    }

                    let result = self.execute_builtin_call(*builtin_id, &arg_values)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::CallNamed {
                    dst,
                    function_name,
                    args,
                } => {
                    let mut arg_values = Vec::new();
                    for arg_reg in args {
                        arg_values.push(self.execution_state.get_register(*arg_reg)?);
                    }

                    // Check if it's a builtin function first
                    if let Some(&builtin_id) = self.builtin_registry.get(function_name) {
                        let result = self.execute_builtin_call(builtin_id, &arg_values)?;
                        self.execution_state.set_register(*dst, result)?;
                    } else if let Some(&func_id) = self.function_registry.get(function_name) {
                        // Recursive call to execute the named function
                        let result = self.execute(func_id, &arg_values)?;
                        self.execution_state.set_register(*dst, result)?;
                    } else {
                        return Err(BytecodeError::NamedFunctionNotFound(function_name.clone()));
                    }
                }

                // Collection operations
                Instruction::MakeList { dst, elements } => {
                    let mut list_values = Vec::new();
                    for elem_reg in elements {
                        let value = self.execution_state.get_register(*elem_reg)?;
                        list_values.push(
                            value
                                .to_ast()
                                .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?,
                        );
                    }
                    let list_value = Value::List(list_values.into());
                    self.execution_state
                        .set_register(*dst, OvmValue::from_ast(list_value))?;
                }

                Instruction::ListGet { dst, list, index } => {
                    let list_value = self.execution_state.get_register(*list)?;
                    let index_value = self.execution_state.get_register(*index)?;
                    let result = self.execute_list_get(&list_value, &index_value)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::ListSet { list, index, value } => {
                    let list_value = self.execution_state.get_register(*list)?;
                    let index_value = self.execution_state.get_register(*index)?;
                    let new_value = self.execution_state.get_register(*value)?;
                    let result = self.execute_list_set(&list_value, &index_value, &new_value)?;
                    self.execution_state.set_register(*list, result)?;
                }

                Instruction::ListLen { dst, list } => {
                    let list_value = self.execution_state.get_register(*list)?;
                    let length = self.execute_list_len(&list_value)?;
                    self.execution_state.set_register(*dst, length)?;
                }

                Instruction::ListPush { list, value } => {
                    let list_value = self.execution_state.get_register(*list)?;
                    let new_value = self.execution_state.get_register(*value)?;
                    let result = self.execute_list_push(&list_value, &new_value)?;
                    self.execution_state.set_register(*list, result)?;
                }

                Instruction::ListPop { dst, list } => {
                    let list_value = self.execution_state.get_register(*list)?;
                    let (new_list, popped_value) = self.execute_list_pop(&list_value)?;
                    self.execution_state.set_register(*list, new_list)?;
                    self.execution_state.set_register(*dst, popped_value)?;
                }

                // Tuple operations
                Instruction::MakeTuple { dst, elements } => {
                    let mut tuple_values = Vec::new();
                    for elem_reg in elements {
                        let value = self.execution_state.get_register(*elem_reg)?;
                        tuple_values.push(
                            value
                                .to_ast()
                                .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?,
                        );
                    }
                    let tuple_value = Value::Tuple(tuple_values.into());
                    self.execution_state
                        .set_register(*dst, OvmValue::from_ast(tuple_value))?;
                }

                Instruction::TupleGet { dst, tuple, index } => {
                    let tuple_value = self.execution_state.get_register(*tuple)?;
                    let result = self.execute_tuple_get(&tuple_value, *index)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                // String operations
                Instruction::StringConcat { dst, lhs, rhs } => {
                    let left = self.execution_state.get_register(*lhs)?;
                    let right = self.execution_state.get_register(*rhs)?;
                    let result = self.execute_string_concat(&left, &right)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::StringLen { dst, src } => {
                    let string_value = self.execution_state.get_register(*src)?;
                    let length = self.execute_string_len(&string_value)?;
                    self.execution_state.set_register(*dst, length)?;
                }

                // Type operations
                Instruction::TypeOf { dst, src } => {
                    let value = self.execution_state.get_register(*src)?;
                    let type_name = self.get_type_name(&value);
                    let type_value =
                        OvmValue::from_ast(Value::String(Arc::new(type_name.to_string())));
                    self.execution_state.set_register(*dst, type_value)?;
                }

                // Debug operations
                Instruction::Nop => {
                    // No operation
                }

                Instruction::DebugPrint { src } => {
                    let value = self.execution_state.get_register(*src)?;
                    println!("[DEBUG] Register r{}: {:?}", src.0, value);
                }

                Instruction::Breakpoint => {
                    // In a complete implementation, this would trigger the debugger
                    println!(
                        "[BREAKPOINT] PC: {}, Function: {:?}",
                        pc, bytecode.function_id
                    );
                }

                Instruction::ProfileEnter { function_id } => {
                    // Record function entry for profiling
                    self.stats.function_calls += 1;
                    if self.stats.function_calls % 1000 == 0 {
                        println!(
                            "[PROFILE] Function {:?} entered (total calls: {})",
                            function_id, self.stats.function_calls
                        );
                    }
                }

                Instruction::ProfileExit { function_id: _ } => {
                    // Record function exit for profiling
                    // In a complete implementation, this would measure execution time
                }

                // Exception handling
                Instruction::TryBegin { handler } => {
                    self.exception_handlers.push(ExceptionHandler {
                        handler_address: handler.0 as usize,
                        stack_depth: self.call_stack.len(),
                    });
                }

                Instruction::TryEnd => {
                    self.exception_handlers.pop();
                }

                Instruction::Throw { exception } => {
                    let exception_value = self.execution_state.get_register(*exception)?;
                    let exception_msg = self.value_to_string(&exception_value)?;
                    return Err(BytecodeError::ExceptionThrown(exception_msg));
                }

                // String slice operations
                Instruction::StringSlice {
                    dst,
                    src,
                    start,
                    end,
                } => {
                    let string_value = self.execution_state.get_register(*src)?;
                    let start_value = self.execution_state.get_register(*start)?;
                    let end_value = self.execution_state.get_register(*end)?;
                    let result =
                        self.execute_string_slice(&string_value, &start_value, &end_value)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                // Type checking operations
                Instruction::CheckType { dst, src, type_id } => {
                    let value = self.execution_state.get_register(*src)?;
                    let type_matches = self.check_type(&value, *type_id)?;
                    self.execution_state
                        .set_register(*dst, OvmValue::from_ast(Value::Boolean(type_matches)))?;
                }

                // Pipeline operations (Olang-specific)
                Instruction::PipelineMap {
                    dst,
                    source,
                    function,
                } => {
                    let source_value = self.execution_state.get_register(*source)?;
                    let function_value = self.execution_state.get_register(*function)?;
                    let result = self.execute_pipeline_map(&source_value, &function_value)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::PipelineFilter {
                    dst,
                    source,
                    predicate,
                } => {
                    let source_value = self.execution_state.get_register(*source)?;
                    let predicate_value = self.execution_state.get_register(*predicate)?;
                    let result = self.execute_pipeline_filter(&source_value, &predicate_value)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::PipelineReduce {
                    dst,
                    source,
                    initial,
                    function,
                } => {
                    let source_value = self.execution_state.get_register(*source)?;
                    let initial_value = self.execution_state.get_register(*initial)?;
                    let function_value = self.execution_state.get_register(*function)?;
                    let result = self.execute_pipeline_reduce(
                        &source_value,
                        &initial_value,
                        &function_value,
                    )?;
                    self.execution_state.set_register(*dst, result)?;
                }

                // Lazy evaluation operations
                Instruction::MakeThunk { dst, expr_idx } => {
                    // For now, create a simple thunk placeholder
                    let thunk_value =
                        OvmValue::from_ast(Value::String(Arc::new(format!("thunk_{}", expr_idx))));
                    self.execution_state.set_register(*dst, thunk_value)?;
                }

                Instruction::ForceThunk { dst, thunk } => {
                    // For now, just return the thunk value as-is
                    let thunk_value = self.execution_state.get_register(*thunk)?;
                    self.execution_state.set_register(*dst, thunk_value)?;
                }

                // Memory operations
                Instruction::Allocate { dst, size } => {
                    let size_value = self.execution_state.get_register(*size)?;
                    let result = self.execute_allocate(&size_value)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::LoadField {
                    dst,
                    object,
                    field_idx,
                } => {
                    let object_value = self.execution_state.get_register(*object)?;
                    let result = self.execute_load_field(&object_value, *field_idx)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::StoreField {
                    object,
                    field_idx,
                    value,
                } => {
                    let object_value = self.execution_state.get_register(*object)?;
                    let new_value = self.execution_state.get_register(*value)?;
                    let result = self.execute_store_field(&object_value, *field_idx, &new_value)?;
                    self.execution_state.set_register(*object, result)?;
                }
            }

            pc += 1;
        }

        // If we reach here without a return, return unit
        Ok(OvmValue::from_ast(Value::Unit))
    }

    /// Execute binary operation
    fn execute_binary_op(
        &self,
        left: &OvmValue,
        right: &OvmValue,
        op: BinaryOp,
    ) -> Result<OvmValue, BytecodeError> {
        // Convert to AST values for computation
        let left_ast = left
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let right_ast = right
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        let result_ast = match (&left_ast, &right_ast) {
            (Value::Integer(a), Value::Integer(b)) => match op {
                BinaryOp::Add => Value::Integer(a + b),
                BinaryOp::Subtract => Value::Integer(a - b),
                BinaryOp::Multiply => Value::Integer(a * b),
                BinaryOp::Divide => {
                    if *b == 0 {
                        return Err(BytecodeError::DivisionByZero);
                    }
                    Value::Integer(a / b)
                }
                BinaryOp::Modulo => {
                    if *b == 0 {
                        return Err(BytecodeError::DivisionByZero);
                    }
                    Value::Integer(a % b)
                }
                BinaryOp::Equal => Value::Boolean(a == b),
                BinaryOp::NotEqual => Value::Boolean(a != b),
                BinaryOp::LessThan => Value::Boolean(a < b),
                BinaryOp::LessThanEqual => Value::Boolean(a <= b),
                BinaryOp::GreaterThan => Value::Boolean(a > b),
                BinaryOp::GreaterThanEqual => Value::Boolean(a >= b),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Unsupported operation: {:?}",
                        op
                    )))
                }
            },
            (Value::Float(a), Value::Float(b)) => match op {
                BinaryOp::Add => Value::Float(a + b),
                BinaryOp::Subtract => Value::Float(a - b),
                BinaryOp::Multiply => Value::Float(a * b),
                BinaryOp::Divide => {
                    if b.abs() < f64::EPSILON {
                        return Err(BytecodeError::DivisionByZero);
                    }
                    Value::Float(a / b)
                }
                BinaryOp::Equal => Value::Boolean((a - b).abs() < f64::EPSILON),
                BinaryOp::NotEqual => Value::Boolean((a - b).abs() >= f64::EPSILON),
                BinaryOp::LessThan => Value::Boolean(a < b),
                BinaryOp::LessThanEqual => Value::Boolean(a <= b),
                BinaryOp::GreaterThan => Value::Boolean(a > b),
                BinaryOp::GreaterThanEqual => Value::Boolean(a >= b),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Unsupported operation: {:?}",
                        op
                    )))
                }
            },
            (Value::String(a), Value::String(b)) => match op {
                BinaryOp::Add => Value::String(Arc::new(format!("{}{}", a, b))),
                BinaryOp::Equal => Value::Boolean(a == b),
                BinaryOp::NotEqual => Value::Boolean(a != b),
                BinaryOp::LessThan => Value::Boolean(a < b),
                BinaryOp::LessThanEqual => Value::Boolean(a <= b),
                BinaryOp::GreaterThan => Value::Boolean(a > b),
                BinaryOp::GreaterThanEqual => Value::Boolean(a >= b),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Unsupported operation: {:?}",
                        op
                    )))
                }
            },
            (Value::Boolean(a), Value::Boolean(b)) => match op {
                BinaryOp::Equal => Value::Boolean(a == b),
                BinaryOp::NotEqual => Value::Boolean(a != b),
                BinaryOp::And => Value::Boolean(*a && *b),
                BinaryOp::Or => Value::Boolean(*a || *b),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Unsupported operation: {:?}",
                        op
                    )))
                }
            },
            _ => {
                return Err(BytecodeError::TypeError(
                    "Type mismatch in binary operation".to_string(),
                ))
            }
        };

        Ok(OvmValue::from_ast(result_ast))
    }

    /// Execute unary operation
    fn execute_unary_op(&self, value: &OvmValue, op: UnaryOp) -> Result<OvmValue, BytecodeError> {
        let value_ast = value
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        let result_ast = match (&value_ast, &op) {
            (Value::Integer(a), UnaryOp::Negate) => Value::Integer(-a),
            (Value::Float(a), UnaryOp::Negate) => Value::Float(-a),
            (Value::Boolean(a), UnaryOp::Not) => Value::Boolean(!a),
            _ => {
                return Err(BytecodeError::TypeError(format!(
                    "Unsupported unary operation: {:?}",
                    op
                )))
            }
        };

        Ok(OvmValue::from_ast(result_ast))
    }

    /// Execute logical AND operation
    fn execute_logical_and(
        &self,
        left: &OvmValue,
        right: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        let left_truthy = self.is_truthy(left);
        if !left_truthy {
            // Short-circuit: return left if it's falsy
            Ok(left.clone())
        } else {
            // Return right if left is truthy
            Ok(right.clone())
        }
    }

    /// Execute logical OR operation
    fn execute_logical_or(
        &self,
        left: &OvmValue,
        right: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        let left_truthy = self.is_truthy(left);
        if left_truthy {
            // Short-circuit: return left if it's truthy
            Ok(left.clone())
        } else {
            // Return right if left is falsy
            Ok(right.clone())
        }
    }

    /// Execute function call
    fn execute_function_call(&self, _args: &[OvmValue]) -> Result<OvmValue, BytecodeError> {
        // Placeholder implementation for function calls
        // In a complete implementation, this would:
        // 1. Look up the function in the function registry
        // 2. Set up a new execution context
        // 3. Execute the function's bytecode
        // 4. Return the result
        Ok(OvmValue::from_ast(Value::Unit))
    }

    /// Execute builtin function call
    fn execute_builtin_call(
        &self,
        builtin_id: u32,
        args: &[OvmValue],
    ) -> Result<OvmValue, BytecodeError> {
        match builtin_id {
            0 => {
                // len function
                if args.len() != 1 {
                    return Err(BytecodeError::RuntimeError(
                        "len expects 1 argument".to_string(),
                    ));
                }
                let value = args[0]
                    .to_ast()
                    .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
                match value {
                    Value::List(list) => Ok(OvmValue::from_ast(Value::Integer(list.len() as i64))),
                    Value::String(s) => Ok(OvmValue::from_ast(Value::Integer(s.len() as i64))),
                    _ => Err(BytecodeError::TypeError(
                        "len can only be applied to lists and strings".to_string(),
                    )),
                }
            }
            1 => {
                // toString function
                if args.len() != 1 {
                    return Err(BytecodeError::RuntimeError(
                        "toString expects 1 argument".to_string(),
                    ));
                }
                let value = args[0]
                    .to_ast()
                    .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
                let string_repr = format!("{}", value);
                Ok(OvmValue::from_ast(Value::String(Arc::new(string_repr))))
            }
            _ => Err(BytecodeError::RuntimeError(format!(
                "Unknown builtin function: {}",
                builtin_id
            ))),
        }
    }

    /// Execute list get operation
    fn execute_list_get(
        &self,
        list: &OvmValue,
        index: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        let list_value = list
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let index_value = index
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match (list_value, index_value) {
            (Value::List(list), Value::Integer(idx)) => {
                let idx = if idx < 0 {
                    list.len() as i64 + idx
                } else {
                    idx
                };

                if idx < 0 || idx >= list.len() as i64 {
                    return Err(BytecodeError::IndexOutOfBounds {
                        index: idx,
                        length: list.len(),
                    });
                }

                Ok(OvmValue::from_ast(list[idx as usize].clone()))
            }
            _ => Err(BytecodeError::TypeError(
                "List get requires a list and integer index".to_string(),
            )),
        }
    }

    /// Execute list set operation
    fn execute_list_set(
        &self,
        list: &OvmValue,
        index: &OvmValue,
        value: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        let list_value = list
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let index_value = index
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let new_value = value
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match (list_value, index_value) {
            (Value::List(list), Value::Integer(idx)) => {
                let idx = if idx < 0 {
                    list.len() as i64 + idx
                } else {
                    idx
                };

                if idx < 0 || idx >= list.len() as i64 {
                    return Err(BytecodeError::IndexOutOfBounds {
                        index: idx,
                        length: list.len(),
                    });
                }

                let mut list_vec = list.to_vec();
                list_vec[idx as usize] = new_value;
                Ok(OvmValue::from_ast(Value::List(list_vec.into())))
            }
            _ => Err(BytecodeError::TypeError(
                "List set requires a list and integer index".to_string(),
            )),
        }
    }

    /// Execute list length operation
    fn execute_list_len(&self, list: &OvmValue) -> Result<OvmValue, BytecodeError> {
        let list_value = list
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match list_value {
            Value::List(list) => Ok(OvmValue::from_ast(Value::Integer(list.len() as i64))),
            _ => Err(BytecodeError::TypeError(
                "List length requires a list".to_string(),
            )),
        }
    }

    /// Execute list push operation
    fn execute_list_push(
        &self,
        list: &OvmValue,
        value: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        let list_value = list
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let new_value = value
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match list_value {
            Value::List(list) => {
                let mut list_vec = list.to_vec();
                list_vec.push(new_value);
                Ok(OvmValue::from_ast(Value::List(list_vec.into())))
            }
            _ => Err(BytecodeError::TypeError(
                "List push requires a list".to_string(),
            )),
        }
    }

    /// Execute list pop operation
    fn execute_list_pop(&self, list: &OvmValue) -> Result<(OvmValue, OvmValue), BytecodeError> {
        let list_value = list
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match list_value {
            Value::List(list) => {
                if list.is_empty() {
                    return Err(BytecodeError::RuntimeError(
                        "Cannot pop from empty list".to_string(),
                    ));
                }

                let mut list_vec = list.to_vec();
                let popped = list_vec.pop().unwrap();
                Ok((
                    OvmValue::from_ast(Value::List(list_vec.into())),
                    OvmValue::from_ast(popped),
                ))
            }
            _ => Err(BytecodeError::TypeError(
                "List pop requires a list".to_string(),
            )),
        }
    }

    /// Execute tuple get operation
    fn execute_tuple_get(&self, tuple: &OvmValue, index: u32) -> Result<OvmValue, BytecodeError> {
        let tuple_value = tuple
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match tuple_value {
            Value::Tuple(tuple) => {
                if index >= tuple.len() as u32 {
                    return Err(BytecodeError::IndexOutOfBounds {
                        index: index as i64,
                        length: tuple.len(),
                    });
                }

                Ok(OvmValue::from_ast(tuple[index as usize].clone()))
            }
            _ => Err(BytecodeError::TypeError(
                "Tuple get requires a tuple".to_string(),
            )),
        }
    }

    /// Execute string concatenation
    fn execute_string_concat(
        &self,
        left: &OvmValue,
        right: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        let left_value = left
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let right_value = right
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match (left_value, right_value) {
            (Value::String(a), Value::String(b)) => {
                let result = format!("{}{}", a, b);
                Ok(OvmValue::from_ast(Value::String(Arc::new(result))))
            }
            _ => Err(BytecodeError::TypeError(
                "String concatenation requires two strings".to_string(),
            )),
        }
    }

    /// Execute string length operation
    fn execute_string_len(&self, string: &OvmValue) -> Result<OvmValue, BytecodeError> {
        let string_value = string
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match string_value {
            Value::String(s) => Ok(OvmValue::from_ast(Value::Integer(s.len() as i64))),
            _ => Err(BytecodeError::TypeError(
                "String length requires a string".to_string(),
            )),
        }
    }

    /// Get type name of a value
    fn get_type_name(&self, value: &OvmValue) -> &'static str {
        match value.to_ast() {
            Ok(Value::Integer(_)) => "int",
            Ok(Value::Float(_)) => "float",
            Ok(Value::String(_)) => "string",
            Ok(Value::Boolean(_)) => "bool",
            Ok(Value::List(_)) => "list",
            Ok(Value::Tuple(_)) => "tuple",
            Ok(Value::Struct { .. }) => "struct",
            Ok(Value::Function(_)) => "function",
            Ok(Value::Builtin(_)) => "builtin",
            Ok(Value::Range { .. }) => "range",
            Ok(Value::Unit) => "unit",
            Ok(Value::Ok(_)) => "result",
            Ok(Value::Err(_)) => "result",
            Ok(Value::Promise { .. }) => "promise",
            Err(_) => "unknown",
        }
    }

    /// Convert value to string for error messages
    fn value_to_string(&self, value: &OvmValue) -> Result<String, BytecodeError> {
        let ast_value = value
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        Ok(format!("{}", ast_value))
    }

    /// Check if value is truthy
    fn is_truthy(&self, value: &OvmValue) -> bool {
        match value.to_ast() {
            Ok(Value::Boolean(b)) => b,
            Ok(Value::Integer(i)) => i != 0,
            Ok(Value::Float(f)) => f != 0.0,
            Ok(Value::Unit) => false,
            _ => true,
        }
    }

    /// Get VM statistics
    pub fn get_stats(&self) -> &VmStatistics {
        &self.stats
    }

    /// Execute string slice operation
    fn execute_string_slice(
        &self,
        string: &OvmValue,
        start: &OvmValue,
        end: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        let string_value = string
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let start_value = start
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let end_value = end
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match (string_value, start_value, end_value) {
            (Value::String(s), Value::Integer(start_idx), Value::Integer(end_idx)) => {
                let start_idx = start_idx.max(0) as usize;
                let end_idx = end_idx.max(0) as usize;
                let slice = if start_idx >= s.len() {
                    ""
                } else if end_idx >= s.len() {
                    &s[start_idx..]
                } else {
                    &s[start_idx..end_idx]
                };
                Ok(OvmValue::from_ast(Value::String(Arc::new(
                    slice.to_string(),
                ))))
            }
            _ => Err(BytecodeError::TypeError(
                "String slice requires string and integer indices".to_string(),
            )),
        }
    }

    /// Check if value matches a type ID
    fn check_type(&self, value: &OvmValue, type_id: u32) -> Result<bool, BytecodeError> {
        let ast_value = value
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        let matches = match (type_id, ast_value) {
            (0, Value::Integer(_)) => true,
            (1, Value::Float(_)) => true,
            (2, Value::Boolean(_)) => true,
            (3, Value::String(_)) => true,
            (4, Value::List(_)) => true,
            (5, Value::Tuple(_)) => true,
            (6, Value::Function(_)) => true,
            (7, Value::Unit) => true,
            (8, Value::Struct { .. }) => true,
            (9, Value::Range { .. }) => true,
            _ => false,
        };

        Ok(matches)
    }

    /// Execute pipeline map operation
    fn execute_pipeline_map(
        &self,
        source: &OvmValue,
        _function: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        // Simplified implementation - in a complete implementation, this would apply the function to each element
        let source_ast = source
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match source_ast {
            Value::List(list) => {
                // For now, just return the original list
                // In a complete implementation, this would apply the function to each element
                Ok(OvmValue::from_ast(Value::List(list)))
            }
            _ => Err(BytecodeError::TypeError(
                "Pipeline map requires a list".to_string(),
            )),
        }
    }

    /// Execute pipeline filter operation
    fn execute_pipeline_filter(
        &self,
        source: &OvmValue,
        _predicate: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        // Simplified implementation - in a complete implementation, this would filter elements
        let source_ast = source
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match source_ast {
            Value::List(list) => {
                // For now, just return the original list
                // In a complete implementation, this would filter elements based on the predicate
                Ok(OvmValue::from_ast(Value::List(list)))
            }
            _ => Err(BytecodeError::TypeError(
                "Pipeline filter requires a list".to_string(),
            )),
        }
    }

    /// Execute pipeline reduce operation
    fn execute_pipeline_reduce(
        &self,
        source: &OvmValue,
        initial: &OvmValue,
        _function: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        // Simplified implementation - in a complete implementation, this would reduce the list
        let _source_ast = source
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        // For now, just return the initial value
        // In a complete implementation, this would apply the function to reduce the list
        Ok(initial.clone())
    }

    /// Execute memory allocation
    fn execute_allocate(&self, size: &OvmValue) -> Result<OvmValue, BytecodeError> {
        let size_ast = size
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match size_ast {
            Value::Integer(size) => {
                if size < 0 {
                    return Err(BytecodeError::RuntimeError(
                        "Cannot allocate negative size".to_string(),
                    ));
                }
                // For now, create a placeholder allocation
                let allocation_id = format!("alloc_{}", size);
                Ok(OvmValue::from_ast(Value::String(Arc::new(allocation_id))))
            }
            _ => Err(BytecodeError::TypeError(
                "Allocation size must be an integer".to_string(),
            )),
        }
    }

    /// Execute field load operation
    fn execute_load_field(
        &self,
        object: &OvmValue,
        field_idx: u32,
    ) -> Result<OvmValue, BytecodeError> {
        let object_ast = object
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match object_ast {
            Value::Struct { fields, .. } => {
                // For struct field access by index, we need to convert to a vector first
                let field_values: Vec<_> = fields.values().cloned().collect();
                if let Some(field_value) = field_values.get(field_idx as usize) {
                    Ok(OvmValue::from_ast(field_value.clone()))
                } else {
                    Err(BytecodeError::IndexOutOfBounds {
                        index: field_idx as i64,
                        length: field_values.len(),
                    })
                }
            }
            Value::Tuple(tuple) => {
                if let Some(field_value) = tuple.get(field_idx as usize) {
                    Ok(OvmValue::from_ast(field_value.clone()))
                } else {
                    Err(BytecodeError::IndexOutOfBounds {
                        index: field_idx as i64,
                        length: tuple.len(),
                    })
                }
            }
            _ => Err(BytecodeError::TypeError(
                "Field access requires a struct or tuple".to_string(),
            )),
        }
    }

    /// Execute field store operation
    fn execute_store_field(
        &self,
        object: &OvmValue,
        field_idx: u32,
        value: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        let object_ast = object
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let new_value_ast = value
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;

        match object_ast {
            Value::Struct {
                type_name,
                mut fields,
            } => {
                // For struct field store by index, we need to work with field order
                let field_keys: Vec<_> = fields.keys().cloned().collect();
                if field_idx as usize >= field_keys.len() {
                    return Err(BytecodeError::IndexOutOfBounds {
                        index: field_idx as i64,
                        length: field_keys.len(),
                    });
                }
                let field_key = &field_keys[field_idx as usize];
                fields.insert(field_key.clone(), new_value_ast);
                Ok(OvmValue::from_ast(Value::Struct { type_name, fields }))
            }
            Value::Tuple(tuple) => {
                if field_idx as usize >= tuple.len() {
                    return Err(BytecodeError::IndexOutOfBounds {
                        index: field_idx as i64,
                        length: tuple.len(),
                    });
                }
                let mut tuple_vec = tuple.to_vec();
                tuple_vec[field_idx as usize] = new_value_ast;
                Ok(OvmValue::from_ast(Value::Tuple(Arc::new(tuple_vec))))
            }
            _ => Err(BytecodeError::TypeError(
                "Field store requires a struct or tuple".to_string(),
            )),
        }
    }
}

impl ExecutionState {
    pub fn new() -> Self {
        Self {
            registers: Vec::new(),
            locals: Vec::new(),
            call_stack: Vec::new(),
            pc: 0,
            exception: None,
            current_bytecode: None,
        }
    }

    pub fn prepare_for_execution(
        &mut self,
        bytecode: &CompiledBytecode,
        args: &[OvmValue],
    ) -> Result<(), BytecodeError> {
        // Allocate registers
        self.registers.clear();
        self.registers.resize(
            bytecode.register_count as usize,
            OvmValue::from_ast(Value::Unit),
        );

        // Set up locals with arguments
        self.locals.clear();
        self.locals.resize(
            bytecode.local_count as usize,
            OvmValue::from_ast(Value::Unit),
        );
        for (i, arg) in args.iter().enumerate() {
            if i < self.locals.len() {
                self.locals[i] = arg.clone();
            }
        }

        // Reset program counter
        self.pc = 0;

        // Clear exception state
        self.exception = None;

        Ok(())
    }

    pub fn get_register(&self, reg: Register) -> Result<OvmValue, BytecodeError> {
        self.registers
            .get(reg.0 as usize)
            .cloned()
            .ok_or(BytecodeError::InvalidRegister(reg))
    }

    pub fn set_register(&mut self, reg: Register, value: OvmValue) -> Result<(), BytecodeError> {
        if let Some(slot) = self.registers.get_mut(reg.0 as usize) {
            *slot = value;
            Ok(())
        } else {
            Err(BytecodeError::InvalidRegister(reg))
        }
    }

    pub fn get_local(&self, local_idx: u32) -> Result<OvmValue, BytecodeError> {
        self.locals
            .get(local_idx as usize)
            .cloned()
            .ok_or(BytecodeError::InvalidLocalIndex(local_idx))
    }

    pub fn set_local(&mut self, local_idx: u32, value: OvmValue) -> Result<(), BytecodeError> {
        if let Some(slot) = self.locals.get_mut(local_idx as usize) {
            *slot = value;
            Ok(())
        } else {
            Err(BytecodeError::InvalidLocalIndex(local_idx))
        }
    }
}

// Compiler and optimization implementations
impl BytecodeCompiler {
    pub fn new() -> Self {
        Self {
            register_allocator: RegisterAllocator::new(),
            emitter: InstructionEmitter::new(),
            optimizer: BytecodeOptimizer::new(),
            local_variables: HashMap::new(),
            next_local_idx: 0,
            label_counter: 0,
            function_registry: HashMap::new(),
        }
    }

    pub fn compile_function(
        &mut self,
        func_id: FunctionId,
        func: &FunctionDecl,
    ) -> Result<CompiledBytecode, BytecodeError> {
        // Reset state
        self.register_allocator.reset();
        self.emitter.reset();
        self.local_variables.clear();
        self.next_local_idx = 0;

        // Add function parameters as locals
        for param in &func.parameters {
            self.local_variables
                .insert(param.name.clone(), self.next_local_idx);
            self.next_local_idx += 1;
        }

        // Compile function body
        let result_reg = self.compile_expression(&func.body)?;

        // Ensure function returns
        if !self.emitter.has_return() {
            self.emitter.emit_return(Some(result_reg));
        }

        // Apply optimizations
        let mut instructions = self.emitter.take_instructions();
        let constants = self.emitter.take_constants();

        instructions = self.optimizer.optimize_instructions(instructions)?;

        Ok(CompiledBytecode {
            function_id: func_id,
            instructions,
            register_count: self.register_allocator.max_register_used(),
            local_count: self.next_local_idx,
            constants,
            debug_info: BytecodeDebugInfo {
                function_name: Some(func.name.clone()),
                ..Default::default()
            },
            optimization_level: 1,
            entry_point: 0,
        })
    }

    fn compile_expression(&mut self, expr: &Expr) -> Result<Register, BytecodeError> {
        match expr {
            Expr::Integer(value) => {
                let const_idx = self
                    .emitter
                    .add_constant(OvmValue::from_ast(Value::Integer(*value)));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }

            Expr::Float(value) => {
                let const_idx = self
                    .emitter
                    .add_constant(OvmValue::from_ast(Value::Float(*value)));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }

            Expr::Boolean(value) => {
                let const_idx = self
                    .emitter
                    .add_constant(OvmValue::from_ast(Value::Boolean(*value)));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }

            Expr::String(value) => {
                let const_idx =
                    self.emitter
                        .add_constant(OvmValue::from_ast(Value::String(Arc::new(
                            (**value).clone(),
                        ))));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }

            Expr::Identifier(name) => {
                let dst_reg = self.register_allocator.allocate_register();
                if let Some(&local_idx) = self.local_variables.get(name) {
                    self.emitter.emit_load_local(dst_reg, local_idx);
                } else {
                    // For now, treat as a constant unit value
                    let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                    self.emitter.emit_load_const(dst_reg, const_idx);
                }
                Ok(dst_reg)
            }

            Expr::BinaryOp { left, op, right } => {
                let left_reg = self.compile_expression(left)?;
                let right_reg = self.compile_expression(right)?;
                let dst_reg = self.register_allocator.allocate_register();

                match op {
                    BinaryOp::Add => self.emitter.emit_add(dst_reg, left_reg, right_reg),
                    BinaryOp::Subtract => self.emitter.emit_sub(dst_reg, left_reg, right_reg),
                    BinaryOp::Multiply => self.emitter.emit_mul(dst_reg, left_reg, right_reg),
                    BinaryOp::Divide => self.emitter.emit_div(dst_reg, left_reg, right_reg),
                    BinaryOp::Equal => self.emitter.emit_eq(dst_reg, left_reg, right_reg),
                    BinaryOp::NotEqual => self.emitter.emit_ne(dst_reg, left_reg, right_reg),
                    BinaryOp::LessThan => self.emitter.emit_lt(dst_reg, left_reg, right_reg),
                    BinaryOp::LessThanEqual => self.emitter.emit_le(dst_reg, left_reg, right_reg),
                    BinaryOp::GreaterThan => self.emitter.emit_gt(dst_reg, left_reg, right_reg),
                    BinaryOp::GreaterThanEqual => {
                        self.emitter.emit_ge(dst_reg, left_reg, right_reg)
                    }
                    _ => {
                        return Err(BytecodeError::CompilationFailed(format!(
                            "Unsupported binary operator: {:?}",
                            op
                        )))
                    }
                }

                Ok(dst_reg)
            }

            Expr::List(elements) => {
                let mut element_regs = Vec::new();
                for element in elements.iter() {
                    element_regs.push(self.compile_expression(element)?);
                }

                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_make_list(dst_reg, element_regs);
                Ok(dst_reg)
            }

            _ => {
                // For unsupported expressions, return a unit constant
                let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }
        }
    }
}

// Implementation stubs for optimization components
impl RegisterAllocator {
    pub fn new() -> Self {
        Self {
            next_register: 0,
            free_registers: Vec::new(),
            register_usage: HashMap::new(),
            max_registers: 0,
        }
    }

    pub fn reset(&mut self) {
        self.next_register = 0;
        self.free_registers.clear();
        self.register_usage.clear();
        self.max_registers = 0;
    }

    pub fn allocate_register(&mut self) -> Register {
        if let Some(reg) = self.free_registers.pop() {
            reg
        } else {
            let reg = Register(self.next_register);
            self.next_register += 1;
            self.max_registers = self.max_registers.max(self.next_register);
            reg
        }
    }

    pub fn free_register(&mut self, reg: Register) {
        self.free_registers.push(reg);
    }

    pub fn max_register_used(&self) -> u32 {
        self.max_registers
    }
}

impl InstructionEmitter {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            labels: HashMap::new(),
            unresolved_labels: HashMap::new(),
            next_label: 0,
            constants: Vec::new(),
            constant_map: HashMap::new(),
            current_line: 0,
            debug_info: BytecodeDebugInfo::default(),
        }
    }

    pub fn reset(&mut self) {
        self.instructions.clear();
        self.labels.clear();
        self.unresolved_labels.clear();
        self.next_label = 0;
        self.constants.clear();
        self.constant_map.clear();
        self.current_line = 0;
        self.debug_info = BytecodeDebugInfo::default();
    }

    pub fn add_constant(&mut self, value: OvmValue) -> u32 {
        let idx = self.constants.len() as u32;
        self.constants.push(value);
        idx
    }

    pub fn emit_load_const(&mut self, dst: Register, const_idx: u32) {
        self.instructions
            .push(Instruction::LoadConst { dst, const_idx });
    }

    pub fn emit_load_local(&mut self, dst: Register, local_idx: u32) {
        self.instructions
            .push(Instruction::LoadLocal { dst, local_idx });
    }

    pub fn emit_store_local(&mut self, src: Register, local_idx: u32) {
        self.instructions
            .push(Instruction::StoreLocal { src, local_idx });
    }

    pub fn emit_add(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Add { dst, lhs, rhs });
    }

    pub fn emit_sub(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Sub { dst, lhs, rhs });
    }

    pub fn emit_mul(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Mul { dst, lhs, rhs });
    }

    pub fn emit_div(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Div { dst, lhs, rhs });
    }

    pub fn emit_eq(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Eq { dst, lhs, rhs });
    }

    pub fn emit_ne(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Ne { dst, lhs, rhs });
    }

    pub fn emit_lt(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Lt { dst, lhs, rhs });
    }

    pub fn emit_le(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Le { dst, lhs, rhs });
    }

    pub fn emit_gt(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Gt { dst, lhs, rhs });
    }

    pub fn emit_ge(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Ge { dst, lhs, rhs });
    }

    pub fn emit_make_list(&mut self, dst: Register, elements: Vec<Register>) {
        self.instructions
            .push(Instruction::MakeList { dst, elements });
    }

    pub fn emit_return(&mut self, value: Option<Register>) {
        self.instructions.push(Instruction::Return { value });
    }

    pub fn emit_nop(&mut self) {
        self.instructions.push(Instruction::Nop);
    }

    pub fn has_return(&self) -> bool {
        self.instructions
            .iter()
            .any(|instr| matches!(instr, Instruction::Return { .. }))
    }

    pub fn take_instructions(&mut self) -> Vec<Instruction> {
        std::mem::take(&mut self.instructions)
    }

    pub fn take_constants(&mut self) -> Vec<OvmValue> {
        std::mem::take(&mut self.constants)
    }
}

impl BytecodeOptimizer {
    pub fn new() -> Self {
        Self {
            dead_code_eliminator: DeadCodeEliminator::new(),
            register_optimizer: RegisterOptimizer::new(),
            control_flow_optimizer: ControlFlowOptimizer::new(),
            constant_folder: ConstantFolder::new(),
            peephole_optimizer: PeepholeOptimizer::new(),
        }
    }

    pub fn optimize_instructions(
        &mut self,
        instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        let mut optimized = instructions;

        // Simplified optimization pipeline to avoid overly aggressive optimizations
        
        // 1. Basic constant folding only
        optimized = self.constant_folder.fold_constants(optimized)?;

        // 2. Simple peephole optimization
        optimized = self.peephole_optimizer.optimize(optimized)?;

        // Skip other optimizations for now to ensure correctness
        // TODO: Re-enable other optimizations after ensuring they don't break basic functionality

        Ok(optimized)
    }
}

impl DeadCodeEliminator {
    pub fn new() -> Self {
        Self {
            live_registers: std::collections::HashSet::new(),
            live_instructions: std::collections::HashSet::new(),
        }
    }

    pub fn eliminate_dead_code(
        &mut self,
        instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        let mut live_instructions = Vec::new();
        let mut live_registers = std::collections::HashSet::new();

        // Mark return instructions and their dependencies as live
        for (i, instruction) in instructions.iter().enumerate() {
            match instruction {
                Instruction::Return { value } => {
                    live_instructions.push(i);
                    if let Some(reg) = value {
                        live_registers.insert(*reg);
                    }
                }
                _ => {}
            }
        }

        // Backward pass to mark dependencies
        for i in (0..instructions.len()).rev() {
            let instruction = &instructions[i];
            let mut should_keep = live_instructions.contains(&i);

            // Check if instruction produces a live register
            match instruction {
                Instruction::LoadConst { dst, .. }
                | Instruction::LoadLocal { dst, .. }
                | Instruction::Move { dst, .. }
                | Instruction::Add { dst, .. }
                | Instruction::Sub { dst, .. }
                | Instruction::Mul { dst, .. }
                | Instruction::Div { dst, .. }
                | Instruction::Eq { dst, .. }
                | Instruction::Ne { dst, .. }
                | Instruction::Lt { dst, .. }
                | Instruction::Le { dst, .. }
                | Instruction::Gt { dst, .. }
                | Instruction::Ge { dst, .. }
                | Instruction::And { dst, .. }
                | Instruction::Or { dst, .. }
                | Instruction::Not { dst, .. }
                | Instruction::MakeList { dst, .. }
                | Instruction::ListGet { dst, .. }
                | Instruction::ListLen { dst, .. }
                | Instruction::ListPop { dst, .. }
                | Instruction::MakeTuple { dst, .. }
                | Instruction::TupleGet { dst, .. }
                | Instruction::StringConcat { dst, .. }
                | Instruction::StringLen { dst, .. }
                | Instruction::TypeOf { dst, .. } => {
                    if live_registers.contains(dst) {
                        should_keep = true;
                    }
                }
                _ => {}
            }

            if should_keep && !live_instructions.contains(&i) {
                live_instructions.push(i);

                // Mark input registers as live
                match instruction {
                    Instruction::Move { src, .. } => {
                        live_registers.insert(*src);
                    }
                    Instruction::Add { lhs, rhs, .. }
                    | Instruction::Sub { lhs, rhs, .. }
                    | Instruction::Mul { lhs, rhs, .. }
                    | Instruction::Div { lhs, rhs, .. }
                    | Instruction::Eq { lhs, rhs, .. }
                    | Instruction::Ne { lhs, rhs, .. }
                    | Instruction::Lt { lhs, rhs, .. }
                    | Instruction::Le { lhs, rhs, .. }
                    | Instruction::Gt { lhs, rhs, .. }
                    | Instruction::Ge { lhs, rhs, .. }
                    | Instruction::And { lhs, rhs, .. }
                    | Instruction::Or { lhs, rhs, .. } => {
                        live_registers.insert(*lhs);
                        live_registers.insert(*rhs);
                    }
                    _ => {}
                }
            }
        }

        // Filter out dead instructions
        live_instructions.sort();
        let mut result = Vec::new();
        for &i in &live_instructions {
            result.push(instructions[i].clone());
        }

        Ok(result)
    }
}

impl RegisterOptimizer {
    pub fn new() -> Self {
        Self {
            register_map: HashMap::new(),
            interference_graph: HashMap::new(),
        }
    }

    pub fn optimize_registers(
        &mut self,
        instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        // Phase 2: Advanced Register Allocation with Live Range Analysis
        let live_ranges = self.compute_live_ranges(&instructions)?;
        let interference_graph = self.build_interference_graph(&live_ranges)?;
        let coloring = self.graph_coloring_allocation(&interference_graph)?;

        // Apply register renaming based on coloring
        let optimized_instructions = self.apply_register_renaming(instructions, &coloring)?;

        Ok(optimized_instructions)
    }

    /// Compute live ranges for all registers using dataflow analysis
    fn compute_live_ranges(
        &mut self,
        instructions: &[Instruction],
    ) -> Result<HashMap<Register, LiveRange>, BytecodeError> {
        let mut live_ranges = HashMap::new();
        let mut register_uses = HashMap::<Register, Vec<usize>>::new();
        let mut register_defs = HashMap::<Register, Vec<usize>>::new();

        // First pass: collect all uses and definitions
        for (i, instruction) in instructions.iter().enumerate() {
            // Collect register definitions (writes)
            if let Some(def_reg) = self.get_definition_register(instruction) {
                register_defs.entry(def_reg).or_default().push(i);
            }

            // Collect register uses (reads)
            for use_reg in self.get_use_registers(instruction) {
                register_uses.entry(use_reg).or_default().push(i);
            }
        }

        // Second pass: compute live ranges
        for (reg, uses) in register_uses.iter() {
            if let Some(defs) = register_defs.get(reg) {
                if let (Some(&first_def), Some(&last_use)) = (defs.first(), uses.last()) {
                    live_ranges.insert(
                        *reg,
                        LiveRange {
                            start: first_def,
                            end: last_use,
                        },
                    );
                }
            }
        }

        Ok(live_ranges)
    }

    /// Build interference graph for register allocation
    fn build_interference_graph(
        &mut self,
        live_ranges: &HashMap<Register, LiveRange>,
    ) -> Result<HashMap<Register, std::collections::HashSet<Register>>, BytecodeError> {
        let mut interference_graph = HashMap::new();

        // For each pair of registers, check if their live ranges overlap
        let registers: Vec<_> = live_ranges.keys().cloned().collect();
        for i in 0..registers.len() {
            for j in (i + 1)..registers.len() {
                let reg1 = registers[i];
                let reg2 = registers[j];

                if let (Some(range1), Some(range2)) =
                    (live_ranges.get(&reg1), live_ranges.get(&reg2))
                {
                    if self.ranges_interfere(range1, range2) {
                        interference_graph
                            .entry(reg1)
                            .or_insert_with(std::collections::HashSet::new)
                            .insert(reg2);
                        interference_graph
                            .entry(reg2)
                            .or_insert_with(std::collections::HashSet::new)
                            .insert(reg1);
                    }
                }
            }
        }

        Ok(interference_graph)
    }

    /// Graph coloring register allocation using greedy algorithm
    fn graph_coloring_allocation(
        &mut self,
        interference_graph: &HashMap<Register, std::collections::HashSet<Register>>,
    ) -> Result<HashMap<Register, Register>, BytecodeError> {
        let mut coloring = HashMap::new();
        let available_colors = (0..32).map(Register).collect::<Vec<_>>(); // 32 physical registers

        // Sort registers by degree (most constrained first)
        let mut registers: Vec<_> = interference_graph.keys().cloned().collect();
        registers.sort_by_key(|reg| interference_graph.get(reg).map_or(0, |set| set.len()));
        registers.reverse(); // Most constrained first

        for reg in registers {
            // Find available color not used by interfering registers
            let mut used_colors = std::collections::HashSet::new();
            if let Some(interfering) = interference_graph.get(&reg) {
                for interfering_reg in interfering {
                    if let Some(&color) = coloring.get(interfering_reg) {
                        used_colors.insert(color);
                    }
                }
            }

            // Assign first available color
            let assigned_color = available_colors
                .iter()
                .find(|&&color| !used_colors.contains(&color))
                .copied()
                .unwrap_or(reg); // Fallback to original register if no color available

            coloring.insert(reg, assigned_color);
        }

        Ok(coloring)
    }

    /// Apply register renaming based on allocation results
    fn apply_register_renaming(
        &mut self,
        instructions: Vec<Instruction>,
        coloring: &HashMap<Register, Register>,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        let mut renamed_instructions = Vec::new();

        for instruction in instructions {
            let renamed = self.rename_instruction_registers(instruction, coloring);
            renamed_instructions.push(renamed);
        }

        Ok(renamed_instructions)
    }

    /// Helper methods for register analysis
    fn get_definition_register(&self, instruction: &Instruction) -> Option<Register> {
        match instruction {
            Instruction::LoadConst { dst, .. }
            | Instruction::LoadLocal { dst, .. }
            | Instruction::Move { dst, .. }
            | Instruction::Add { dst, .. }
            | Instruction::Sub { dst, .. }
            | Instruction::Mul { dst, .. }
            | Instruction::Div { dst, .. }
            | Instruction::Eq { dst, .. }
            | Instruction::Ne { dst, .. }
            | Instruction::Lt { dst, .. }
            | Instruction::Le { dst, .. }
            | Instruction::Gt { dst, .. }
            | Instruction::Ge { dst, .. }
            | Instruction::And { dst, .. }
            | Instruction::Or { dst, .. }
            | Instruction::Not { dst, .. }
            | Instruction::MakeList { dst, .. }
            | Instruction::ListGet { dst, .. }
            | Instruction::ListLen { dst, .. }
            | Instruction::ListPop { dst, .. }
            | Instruction::MakeTuple { dst, .. }
            | Instruction::TupleGet { dst, .. }
            | Instruction::StringConcat { dst, .. }
            | Instruction::StringLen { dst, .. }
            | Instruction::TypeOf { dst, .. } => Some(*dst),
            _ => None,
        }
    }

    fn get_use_registers(&self, instruction: &Instruction) -> Vec<Register> {
        match instruction {
            Instruction::Move { src, .. } => vec![*src],
            Instruction::Add { lhs, rhs, .. }
            | Instruction::Sub { lhs, rhs, .. }
            | Instruction::Mul { lhs, rhs, .. }
            | Instruction::Div { lhs, rhs, .. }
            | Instruction::Eq { lhs, rhs, .. }
            | Instruction::Ne { lhs, rhs, .. }
            | Instruction::Lt { lhs, rhs, .. }
            | Instruction::Le { lhs, rhs, .. }
            | Instruction::Gt { lhs, rhs, .. }
            | Instruction::Ge { lhs, rhs, .. }
            | Instruction::And { lhs, rhs, .. }
            | Instruction::Or { lhs, rhs, .. } => vec![*lhs, *rhs],
            Instruction::Not { src, .. } => vec![*src],
            Instruction::StoreLocal { src, .. } => vec![*src],
            Instruction::Return { value } => value.map_or(vec![], |v| vec![v]),
            Instruction::MakeList { elements, .. } => elements.clone(),
            Instruction::ListGet { list, index, .. } => vec![*list, *index],
            Instruction::ListSet { list, index, value } => vec![*list, *index, *value],
            Instruction::ListPush { list, value } => vec![*list, *value],
            Instruction::ListPop { list, .. } => vec![*list],
            Instruction::MakeTuple { elements, .. } => elements.clone(),
            Instruction::TupleGet { tuple, .. } => vec![*tuple],
            Instruction::StringConcat { lhs, rhs, .. } => vec![*lhs, *rhs],
            Instruction::StringLen { src, .. } => vec![*src],
            Instruction::TypeOf { src, .. } => vec![*src],
            _ => vec![],
        }
    }

    fn ranges_interfere(&self, range1: &LiveRange, range2: &LiveRange) -> bool {
        !(range1.end < range2.start || range2.end < range1.start)
    }

    fn rename_instruction_registers(
        &self,
        instruction: Instruction,
        coloring: &HashMap<Register, Register>,
    ) -> Instruction {
        match instruction {
            Instruction::LoadConst { dst, const_idx } => Instruction::LoadConst {
                dst: coloring.get(&dst).copied().unwrap_or(dst),
                const_idx,
            },
            Instruction::Move { dst, src } => Instruction::Move {
                dst: coloring.get(&dst).copied().unwrap_or(dst),
                src: coloring.get(&src).copied().unwrap_or(src),
            },
            Instruction::Add { dst, lhs, rhs } => Instruction::Add {
                dst: coloring.get(&dst).copied().unwrap_or(dst),
                lhs: coloring.get(&lhs).copied().unwrap_or(lhs),
                rhs: coloring.get(&rhs).copied().unwrap_or(rhs),
            },
            // Add similar renaming for other instructions...
            _ => instruction, // For now, return as-is for unhandled instructions
        }
    }
}

impl ControlFlowOptimizer {
    pub fn new() -> Self {
        Self {
            basic_blocks: Vec::new(),
            cfg: ControlFlowGraph {
                blocks: Vec::new(),
                entry_block: 0,
                exit_blocks: Vec::new(),
            },
        }
    }

    pub fn optimize_control_flow(
        &mut self,
        instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        // Phase 2: Advanced Control Flow Optimization
        let basic_blocks = self.build_basic_blocks(&instructions)?;
        let cfg = self.build_control_flow_graph(&basic_blocks)?;

        // Apply control flow optimizations
        let optimized_cfg = self.optimize_cfg(cfg)?;
        let optimized_instructions = self.reconstruct_instructions(&optimized_cfg)?;

        Ok(optimized_instructions)
    }

    fn build_basic_blocks(
        &mut self,
        instructions: &[Instruction],
    ) -> Result<Vec<BasicBlock>, BytecodeError> {
        let mut blocks = Vec::new();
        let mut current_block = Vec::new();
        let mut block_id = 0;
        let mut leaders = std::collections::HashSet::new();

        // Identify block leaders (first instruction, jump targets, instruction after jumps)
        leaders.insert(0); // First instruction is always a leader

        for (i, instruction) in instructions.iter().enumerate() {
            match instruction {
                Instruction::Jump { target }
                | Instruction::JumpIfTrue { target, .. }
                | Instruction::JumpIfFalse { target, .. } => {
                    leaders.insert(target.0 as usize);
                    if i + 1 < instructions.len() {
                        leaders.insert(i + 1); // Instruction after jump
                    }
                }
                _ => {}
            }
        }

        // Build basic blocks
        for (i, instruction) in instructions.iter().enumerate() {
            if leaders.contains(&i) && !current_block.is_empty() {
                // Start new block
                blocks.push(BasicBlock {
                    id: block_id,
                    instructions: current_block.clone(),
                    predecessors: Vec::new(),
                    successors: Vec::new(),
                    entry_point: i - current_block.len(),
                    exit_point: i - 1,
                });
                current_block.clear();
                block_id += 1;
            }
            current_block.push(instruction.clone());
        }

        // Add final block
        if !current_block.is_empty() {
            let block_len = current_block.len();
            blocks.push(BasicBlock {
                id: block_id,
                instructions: current_block,
                predecessors: Vec::new(),
                successors: Vec::new(),
                entry_point: instructions.len() - block_len,
                exit_point: instructions.len() - 1,
            });
        }

        Ok(blocks)
    }

    fn build_control_flow_graph(
        &mut self,
        blocks: &[BasicBlock],
    ) -> Result<ControlFlowGraph, BytecodeError> {
        let mut cfg_blocks = blocks.to_vec();

        // Build successor/predecessor relationships
        for i in 0..cfg_blocks.len() {
            if let Some(last_instruction) = cfg_blocks[i].instructions.last() {
                match last_instruction {
                    Instruction::Jump { target } => {
                        let target_block =
                            self.find_block_by_address(&cfg_blocks, target.0 as usize);
                        if let Some(target_id) = target_block {
                            cfg_blocks[i].successors.push(target_id);
                            cfg_blocks[target_id].predecessors.push(i);
                        }
                    }
                    Instruction::JumpIfTrue { target, .. }
                    | Instruction::JumpIfFalse { target, .. } => {
                        // Conditional jump has two successors
                        let target_block =
                            self.find_block_by_address(&cfg_blocks, target.0 as usize);
                        if let Some(target_id) = target_block {
                            cfg_blocks[i].successors.push(target_id);
                            cfg_blocks[target_id].predecessors.push(i);
                        }

                        // Fall-through successor
                        if i + 1 < cfg_blocks.len() {
                            cfg_blocks[i].successors.push(i + 1);
                            cfg_blocks[i + 1].predecessors.push(i);
                        }
                    }
                    Instruction::Return { .. } => {
                        // No successors for return
                    }
                    _ => {
                        // Fall-through to next block
                        if i + 1 < cfg_blocks.len() {
                            cfg_blocks[i].successors.push(i + 1);
                            cfg_blocks[i + 1].predecessors.push(i);
                        }
                    }
                }
            }
        }

        // Identify exit blocks
        let exit_blocks = cfg_blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.successors.is_empty())
            .map(|(i, _)| i)
            .collect();

        Ok(ControlFlowGraph {
            blocks: cfg_blocks,
            entry_block: 0,
            exit_blocks,
        })
    }

    fn optimize_cfg(
        &mut self,
        mut cfg: ControlFlowGraph,
    ) -> Result<ControlFlowGraph, BytecodeError> {
        // Apply various CFG optimizations
        self.eliminate_unreachable_blocks(&mut cfg)?;
        self.merge_sequential_blocks(&mut cfg)?;
        self.eliminate_empty_blocks(&mut cfg)?;
        self.optimize_jumps(&mut cfg)?;

        Ok(cfg)
    }

    fn eliminate_unreachable_blocks(
        &mut self,
        cfg: &mut ControlFlowGraph,
    ) -> Result<(), BytecodeError> {
        let mut reachable = std::collections::HashSet::new();
        let mut worklist = vec![cfg.entry_block];

        // Mark reachable blocks
        while let Some(block_id) = worklist.pop() {
            if reachable.insert(block_id) {
                for &successor in &cfg.blocks[block_id].successors {
                    worklist.push(successor);
                }
            }
        }

        // Remove unreachable blocks
        cfg.blocks.retain(|block| reachable.contains(&block.id));

        Ok(())
    }

    fn merge_sequential_blocks(&mut self, cfg: &mut ControlFlowGraph) -> Result<(), BytecodeError> {
        let mut merged = true;

        while merged {
            merged = false;

            for i in 0..cfg.blocks.len() {
                if cfg.blocks[i].successors.len() == 1 {
                    let successor_id = cfg.blocks[i].successors[0];
                    if successor_id < cfg.blocks.len()
                        && cfg.blocks[successor_id].predecessors.len() == 1
                    {
                        // Merge blocks
                        let mut successor_instructions =
                            cfg.blocks[successor_id].instructions.clone();
                        cfg.blocks[i]
                            .instructions
                            .append(&mut successor_instructions);
                        cfg.blocks[i].successors = cfg.blocks[successor_id].successors.clone();
                        cfg.blocks[i].exit_point = cfg.blocks[successor_id].exit_point;

                        // Update successor references
                        let successors = cfg.blocks[i].successors.clone();
                        for &succ in &successors {
                            if succ < cfg.blocks.len() {
                                if let Some(pos) = cfg.blocks[succ]
                                    .predecessors
                                    .iter()
                                    .position(|&x| x == successor_id)
                                {
                                    cfg.blocks[succ].predecessors[pos] = i;
                                }
                            }
                        }

                        // Mark for removal
                        cfg.blocks[successor_id].instructions.clear();
                        merged = true;
                        break;
                    }
                }
            }
        }

        // Remove empty blocks
        cfg.blocks.retain(|block| !block.instructions.is_empty());

        Ok(())
    }

    fn eliminate_empty_blocks(&mut self, cfg: &mut ControlFlowGraph) -> Result<(), BytecodeError> {
        // Remove blocks that only contain jumps
        for i in 0..cfg.blocks.len() {
            if cfg.blocks[i].instructions.len() == 1 {
                if let Instruction::Jump { target } = &cfg.blocks[i].instructions[0] {
                    let target_block = self.find_block_by_address(&cfg.blocks, target.0 as usize);
                    if let Some(target_id) = target_block {
                        // Redirect predecessors to target
                        for &pred in &cfg.blocks[i].predecessors.clone() {
                            if pred < cfg.blocks.len() {
                                // Update predecessor's successors
                                for succ in &mut cfg.blocks[pred].successors {
                                    if *succ == i {
                                        *succ = target_id;
                                    }
                                }
                            }
                        }

                        // Update target's predecessors
                        if target_id < cfg.blocks.len() {
                            cfg.blocks[target_id].predecessors.retain(|&x| x != i);
                            let predecessors = cfg.blocks[i].predecessors.clone();
                            cfg.blocks[target_id].predecessors.extend(&predecessors);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn optimize_jumps(&mut self, cfg: &mut ControlFlowGraph) -> Result<(), BytecodeError> {
        // Optimize jump instructions (e.g., jump to next instruction)
        for block in &mut cfg.blocks {
            if let Some(last_instruction) = block.instructions.last_mut() {
                match last_instruction {
                    Instruction::Jump { target } => {
                        // Check if jumping to immediately next instruction
                        if target.0 as usize == block.exit_point + 1 {
                            // Remove redundant jump
                            block.instructions.pop();
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    fn reconstruct_instructions(
        &mut self,
        cfg: &ControlFlowGraph,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        let mut instructions = Vec::new();

        for block in &cfg.blocks {
            instructions.extend(block.instructions.iter().cloned());
        }

        Ok(instructions)
    }

    fn find_block_by_address(&self, blocks: &[BasicBlock], address: usize) -> Option<usize> {
        blocks
            .iter()
            .position(|block| address >= block.entry_point && address <= block.exit_point)
    }
}

// Add new optimization passes for Phase 2

/// Loop optimization pass
pub struct LoopOptimizer {
    loop_info: HashMap<usize, LoopInfo>,
    dominance_tree: DominanceTree,
}

#[derive(Debug, Clone)]
struct LoopInfo {
    header: usize,
    body: Vec<usize>,
    exit_blocks: Vec<usize>,
    depth: u32,
    trip_count: Option<u32>,
}

#[derive(Debug, Clone)]
struct DominanceTree {
    dominators: HashMap<usize, Vec<usize>>,
    immediate_dominators: HashMap<usize, usize>,
}

impl LoopOptimizer {
    pub fn new() -> Self {
        Self {
            loop_info: HashMap::new(),
            dominance_tree: DominanceTree {
                dominators: HashMap::new(),
                immediate_dominators: HashMap::new(),
            },
        }
    }

    pub fn optimize_loops(
        &mut self,
        instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        // Phase 2: Loop optimization including unrolling and invariant code motion
        let loops = self.identify_loops(&instructions)?;
        let mut optimized = instructions;

        for loop_info in loops {
            if let Some(trip_count) = loop_info.trip_count {
                if trip_count <= 8 && trip_count > 1 {
                    // Apply loop unrolling for small loops
                    optimized = self.unroll_loop(optimized, &loop_info)?;
                }
            }

            // Apply loop invariant code motion
            optimized = self.move_loop_invariants(optimized, &loop_info)?;
        }

        Ok(optimized)
    }

    fn identify_loops(
        &mut self,
        instructions: &[Instruction],
    ) -> Result<Vec<LoopInfo>, BytecodeError> {
        // Simplified loop detection - look for backward jumps
        let mut loops = Vec::new();

        for (i, instruction) in instructions.iter().enumerate() {
            match instruction {
                Instruction::JumpIfTrue { target, .. }
                | Instruction::JumpIfFalse { target, .. } => {
                    let target_addr = target.0 as usize;
                    if target_addr < i {
                        // Backward jump - potential loop
                        loops.push(LoopInfo {
                            header: target_addr,
                            body: (target_addr..=i).collect(),
                            exit_blocks: vec![i + 1],
                            depth: 1,
                            trip_count: None, // Would need more analysis
                        });
                    }
                }
                _ => {}
            }
        }

        Ok(loops)
    }

    fn unroll_loop(
        &mut self,
        mut instructions: Vec<Instruction>,
        loop_info: &LoopInfo,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        if let Some(trip_count) = loop_info.trip_count {
            // Simple loop unrolling - duplicate loop body
            let loop_body: Vec<_> = instructions
                [loop_info.header..=*loop_info.body.last().unwrap_or(&loop_info.header)]
                .to_vec();

            // Replace loop with unrolled version
            let mut unrolled = Vec::new();
            for _ in 0..trip_count {
                unrolled.extend(loop_body.iter().cloned());
            }

            // Replace original loop
            instructions.splice(
                loop_info.header..=*loop_info.body.last().unwrap_or(&loop_info.header),
                unrolled,
            );
        }

        Ok(instructions)
    }

    fn move_loop_invariants(
        &mut self,
        instructions: Vec<Instruction>,
        _loop_info: &LoopInfo,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        // Simplified loop invariant code motion
        // In a complete implementation, this would:
        // 1. Analyze which computations are loop-invariant
        // 2. Move them outside the loop
        // 3. Update register usage accordingly

        Ok(instructions) // For now, return as-is
    }
}

/// Function inlining optimization
pub struct FunctionInliner {
    inline_candidates: HashMap<FunctionId, InlineInfo>,
    call_graph: CallGraph,
    inlining_budget: usize,
}

#[derive(Debug, Clone)]
struct InlineInfo {
    function_id: FunctionId,
    size: usize,
    call_frequency: u32,
    benefit_score: f64,
    can_inline: bool,
}

#[derive(Debug, Clone)]
struct CallGraph {
    edges: HashMap<FunctionId, Vec<FunctionId>>,
    call_counts: HashMap<(FunctionId, FunctionId), u32>,
}

impl FunctionInliner {
    pub fn new() -> Self {
        Self {
            inline_candidates: HashMap::new(),
            call_graph: CallGraph {
                edges: HashMap::new(),
                call_counts: HashMap::new(),
            },
            inlining_budget: 1000, // Maximum instructions to add through inlining
        }
    }

    pub fn inline_functions(
        &mut self,
        instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        // Phase 2: Intelligent function inlining based on call frequency and size
        let call_sites = self.identify_call_sites(&instructions)?;
        let mut optimized = instructions;

        // Sort call sites by benefit score
        let mut sorted_calls = call_sites;
        sorted_calls.sort_by(|a, b| {
            b.benefit_score
                .partial_cmp(&a.benefit_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for call_site in sorted_calls {
            if self.inlining_budget > 0 && call_site.can_inline {
                optimized = self.inline_call_site(optimized, &call_site)?;
                self.inlining_budget = self.inlining_budget.saturating_sub(call_site.size);
            }
        }

        Ok(optimized)
    }

    fn identify_call_sites(
        &mut self,
        instructions: &[Instruction],
    ) -> Result<Vec<InlineInfo>, BytecodeError> {
        let mut call_sites = Vec::new();

        for (_i, instruction) in instructions.iter().enumerate() {
            match instruction {
                Instruction::Call { .. } => {
                    // Analyze call site for inlining potential
                    call_sites.push(InlineInfo {
                        function_id: FunctionId::new(), // Would need actual function ID
                        size: 50,                       // Estimated size
                        call_frequency: 1,
                        benefit_score: 1.0,
                        can_inline: true,
                    });
                }
                _ => {}
            }
        }

        Ok(call_sites)
    }

    fn inline_call_site(
        &mut self,
        instructions: Vec<Instruction>,
        _inline_info: &InlineInfo,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        // Simplified inlining - would need actual function body
        // In a complete implementation, this would:
        // 1. Get the function body to inline
        // 2. Rename registers to avoid conflicts
        // 3. Replace call instruction with inlined body
        // 4. Handle parameter passing and return values

        Ok(instructions) // For now, return as-is
    }
}

// Phase 2: Enhanced Optimization Pass Implementations

impl ConstantFolder {
    pub fn new() -> Self {
        Self {
            constant_values: HashMap::new(),
        }
    }

    pub fn fold_constants(
        &mut self,
        instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        let mut result = Vec::new();
        self.constant_values.clear();

        for instruction in instructions {
            match instruction {
                // Track constant loads
                Instruction::LoadConst { dst: _, const_idx: _ } => {
                    // We can't actually fold without access to the constants table here
                    // In a complete implementation, this would be passed in
                    result.push(instruction);
                }

                // Fold binary operations on constants
                Instruction::Add { dst, lhs, rhs } => {
                    if let (Some(left_val), Some(right_val)) = (
                        self.constant_values.get(&lhs).cloned(),
                        self.constant_values.get(&rhs).cloned(),
                    ) {
                        // Try to fold the operation
                        if let (Ok(left_ast), Ok(right_ast)) =
                            (left_val.to_ast(), right_val.to_ast())
                        {
                            match (left_ast, right_ast) {
                                (Value::Integer(a), Value::Integer(b)) => {
                                    let folded_value = OvmValue::from_ast(Value::Integer(a + b));
                                    self.constant_values.insert(dst, folded_value);
                                    // In a complete implementation, we'd emit a LoadConst instead
                                    result.push(instruction);
                                }
                                (Value::Float(a), Value::Float(b)) => {
                                    let folded_value = OvmValue::from_ast(Value::Float(a + b));
                                    self.constant_values.insert(dst, folded_value);
                                    result.push(instruction);
                                }
                                _ => {
                                    result.push(instruction);
                                }
                            }
                        } else {
                            result.push(instruction);
                        }
                    } else {
                        result.push(instruction);
                    }
                }

                // Similar for other arithmetic operations
                Instruction::Sub { dst, lhs, rhs } => {
                    if let (Some(left_val), Some(right_val)) = (
                        self.constant_values.get(&lhs).cloned(),
                        self.constant_values.get(&rhs).cloned(),
                    ) {
                        if let (Ok(left_ast), Ok(right_ast)) =
                            (left_val.to_ast(), right_val.to_ast())
                        {
                            match (left_ast, right_ast) {
                                (Value::Integer(a), Value::Integer(b)) => {
                                    let folded_value = OvmValue::from_ast(Value::Integer(a - b));
                                    self.constant_values.insert(dst, folded_value);
                                    result.push(instruction);
                                }
                                (Value::Float(a), Value::Float(b)) => {
                                    let folded_value = OvmValue::from_ast(Value::Float(a - b));
                                    self.constant_values.insert(dst, folded_value);
                                    result.push(instruction);
                                }
                                _ => {
                                    result.push(instruction);
                                }
                            }
                        } else {
                            result.push(instruction);
                        }
                    } else {
                        result.push(instruction);
                    }
                }

                _ => {
                    result.push(instruction);
                }
            }
        }

        Ok(result)
    }
}

impl PeepholeOptimizer {
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }

    pub fn optimize(
        &mut self,
        instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        let mut result = Vec::new();
        let mut i = 0;

        while i < instructions.len() {
            let current = &instructions[i];

            // Phase 2: Enhanced pattern matching with more sophisticated optimizations

            // Look for 3-instruction patterns first
            if i + 2 < instructions.len() {
                let next = &instructions[i + 1];
                let next2 = &instructions[i + 2];

                // Pattern: Move r1, r2; Move r2, r3; Move r3, r1 -> Nop (cycle elimination)
                if let (
                    Instruction::Move {
                        dst: dst1,
                        src: src1,
                    },
                    Instruction::Move {
                        dst: dst2,
                        src: src2,
                    },
                    Instruction::Move {
                        dst: dst3,
                        src: src3,
                    },
                ) = (current, next, next2)
                {
                    if dst1 == src2 && dst2 == src3 && dst3 == src1 {
                        // Eliminate the entire cycle
                        i += 3;
                        continue;
                    }
                }
            }

            // Look for 2-instruction patterns
            if i + 1 < instructions.len() {
                let next = &instructions[i + 1];

                // Pattern: Move r1, r2; Move r2, r1 -> eliminate redundant moves
                if let (
                    Instruction::Move {
                        dst: dst1,
                        src: src1,
                    },
                    Instruction::Move {
                        dst: dst2,
                        src: src2,
                    },
                ) = (current, next)
                {
                    if dst1 == src2 && src1 == dst2 {
                        // Skip both instructions (they cancel out)
                        i += 2;
                        continue;
                    }
                }

                // Pattern: LoadConst r1, c; Move r2, r1 -> LoadConst r2, c
                if let (
                    Instruction::LoadConst {
                        dst: dst1,
                        const_idx,
                    },
                    Instruction::Move { dst: dst2, src },
                ) = (current, next)
                {
                    if dst1 == src {
                        result.push(Instruction::LoadConst {
                            dst: *dst2,
                            const_idx: *const_idx,
                        });
                        i += 2;
                        continue;
                    }
                }

                // Pattern: Add r1, r2, r3; Move r4, r1 -> Add r4, r2, r3
                if let (
                    Instruction::Add {
                        dst: dst1,
                        lhs,
                        rhs,
                    },
                    Instruction::Move { dst: dst2, src },
                ) = (current, next)
                {
                    if dst1 == src {
                        result.push(Instruction::Add {
                            dst: *dst2,
                            lhs: *lhs,
                            rhs: *rhs,
                        });
                        i += 2;
                        continue;
                    }
                }

                // Pattern: Sub r1, r2, r3; Move r4, r1 -> Sub r4, r2, r3
                if let (
                    Instruction::Sub {
                        dst: dst1,
                        lhs,
                        rhs,
                    },
                    Instruction::Move { dst: dst2, src },
                ) = (current, next)
                {
                    if dst1 == src {
                        result.push(Instruction::Sub {
                            dst: *dst2,
                            lhs: *lhs,
                            rhs: *rhs,
                        });
                        i += 2;
                        continue;
                    }
                }

                // Pattern: Mul r1, r2, r3; Move r4, r1 -> Mul r4, r2, r3
                if let (
                    Instruction::Mul {
                        dst: dst1,
                        lhs,
                        rhs,
                    },
                    Instruction::Move { dst: dst2, src },
                ) = (current, next)
                {
                    if dst1 == src {
                        result.push(Instruction::Mul {
                            dst: *dst2,
                            lhs: *lhs,
                            rhs: *rhs,
                        });
                        i += 2;
                        continue;
                    }
                }

                // Pattern: Not r1, r2; Not r3, r1 -> Move r3, r2 (double negation)
                if let (
                    Instruction::Not {
                        dst: dst1,
                        src: src1,
                    },
                    Instruction::Not {
                        dst: dst2,
                        src: src2,
                    },
                ) = (current, next)
                {
                    if dst1 == src2 {
                        result.push(Instruction::Move {
                            dst: *dst2,
                            src: *src1,
                        });
                        i += 2;
                        continue;
                    }
                }
            }

            // Single instruction optimizations
            match current {
                // Pattern: Move r1, r1 -> Nop (self-move elimination)
                Instruction::Move { dst, src } if dst == src => {
                    // Skip this instruction
                    i += 1;
                    continue;
                }

                _ => {}
            }

            // No optimization applied, keep the instruction
            result.push(current.clone());
            i += 1;
        }

        Ok(result)
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Instruction::LoadConst { dst, const_idx } => {
                write!(f, "LOAD_CONST r{}, #{}", dst.0, const_idx)
            }
            Instruction::LoadLocal { dst, local_idx } => {
                write!(f, "LOAD_LOCAL r{}, l{}", dst.0, local_idx)
            }
            Instruction::StoreLocal { src, local_idx } => {
                write!(f, "STORE_LOCAL r{}, l{}", src.0, local_idx)
            }
            Instruction::Move { dst, src } => write!(f, "MOVE r{}, r{}", dst.0, src.0),
            Instruction::Add { dst, lhs, rhs } => {
                write!(f, "ADD r{}, r{}, r{}", dst.0, lhs.0, rhs.0)
            }
            Instruction::Sub { dst, lhs, rhs } => {
                write!(f, "SUB r{}, r{}, r{}", dst.0, lhs.0, rhs.0)
            }
            Instruction::Mul { dst, lhs, rhs } => {
                write!(f, "MUL r{}, r{}, r{}", dst.0, lhs.0, rhs.0)
            }
            Instruction::Return { value } => {
                if let Some(reg) = value {
                    write!(f, "RETURN r{}", reg.0)
                } else {
                    write!(f, "RETURN")
                }
            }
            Instruction::Nop => write!(f, "NOP"),
            _ => write!(f, "{:?}", self),
        }
    }
}

impl Default for BytecodeVm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{FunctionDecl, Parameter, TypeAnnotation};

    #[test]
    fn test_bytecode_vm_creation() {
        let vm = BytecodeVm::new();
        assert_eq!(vm.stats.instructions_executed, 0);
        assert_eq!(vm.stats.function_calls, 0);
    }

    #[test]
    fn test_simple_function_compilation() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a simple function: fn test() -> int { 42 }
        let func = FunctionDecl {
            name: "test".to_string(),
            type_params: Vec::new(),
            parameters: Vec::new(),
            return_type: Some(TypeAnnotation::Int),
            body: Expr::Integer(42),
        };

        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function compilation should succeed");
        assert!(
            vm.has_bytecode(func_id),
            "VM should have bytecode for the function"
        );
    }

    #[test]
    fn test_function_with_parameters() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create function: fn add(a: int, b: int) -> int { a + b }
        let func = FunctionDecl {
            name: "add".to_string(),
            type_params: Vec::new(),
            parameters: vec![
                Parameter {
                    name: "a".to_string(),
                    type_annotation: Some(TypeAnnotation::Int),
                },
                Parameter {
                    name: "b".to_string(),
                    type_annotation: Some(TypeAnnotation::Int),
                },
            ],
            return_type: Some(TypeAnnotation::Int),
            body: Expr::BinaryOp {
                left: Box::new(Expr::Identifier("a".to_string())),
                op: BinaryOp::Add,
                right: Box::new(Expr::Identifier("b".to_string())),
            },
        };

        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function with parameters should compile");

        // Test execution with arguments
        let args = vec![
            OvmValue::from_ast(Value::Integer(5)),
            OvmValue::from_ast(Value::Integer(3)),
        ];

        let execution_result = vm.execute(func_id, &args);
        assert!(
            execution_result.is_ok(),
            "Function execution should succeed"
        );

        // The result should be 8 (5 + 3)
        let result_value = execution_result.unwrap();
        match result_value.to_ast() {
            Ok(Value::Integer(n)) => assert_eq!(n, 8, "Addition result should be 8"),
            Ok(value) => panic!("Expected integer result, but got: {:?}", value),
            Err(e) => panic!("Failed to convert result to AST: {:?}", e),
        }
    }

    #[test]
    fn test_list_operations() {
        let vm = BytecodeVm::new();

        // Test list creation and access
        let list_value = OvmValue::from_ast(Value::List(
            vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)].into(),
        ));

        let index_value = OvmValue::from_ast(Value::Integer(1));
        let result = vm.execute_list_get(&list_value, &index_value);

        assert!(result.is_ok(), "List get should succeed");
        match result.unwrap().to_ast() {
            Ok(Value::Integer(n)) => assert_eq!(n, 2, "Should get second element"),
            _ => panic!("Expected integer result"),
        }
    }

    #[test]
    fn test_string_operations() {
        let vm = BytecodeVm::new();

        let left = OvmValue::from_ast(Value::String(Arc::new("Hello".to_string())));
        let right = OvmValue::from_ast(Value::String(Arc::new(" World".to_string())));

        let result = vm.execute_string_concat(&left, &right);
        assert!(result.is_ok(), "String concatenation should succeed");

        match result.unwrap().to_ast() {
            Ok(Value::String(s)) => assert_eq!(*s, "Hello World", "Concatenation should work"),
            _ => panic!("Expected string result"),
        }
    }

    #[test]
    fn test_arithmetic_operations() {
        let vm = BytecodeVm::new();

        let left = OvmValue::from_ast(Value::Integer(10));
        let right = OvmValue::from_ast(Value::Integer(3));

        // Test addition
        let result = vm.execute_binary_op(&left, &right, BinaryOp::Add);
        assert!(result.is_ok());
        match result.unwrap().to_ast() {
            Ok(Value::Integer(n)) => assert_eq!(n, 13),
            _ => panic!("Expected integer result"),
        }

        // Test division
        let result = vm.execute_binary_op(&left, &right, BinaryOp::Divide);
        assert!(result.is_ok());
        match result.unwrap().to_ast() {
            Ok(Value::Integer(n)) => assert_eq!(n, 3),
            _ => panic!("Expected integer result"),
        }

        // Test division by zero
        let zero = OvmValue::from_ast(Value::Integer(0));
        let result = vm.execute_binary_op(&left, &zero, BinaryOp::Divide);
        assert!(result.is_err(), "Division by zero should fail");
        match result.unwrap_err() {
            BytecodeError::DivisionByZero => {}
            _ => panic!("Expected division by zero error"),
        }
    }

    #[test]
    fn test_type_operations() {
        let vm = BytecodeVm::new();

        let int_value = OvmValue::from_ast(Value::Integer(42));
        let string_value = OvmValue::from_ast(Value::String(Arc::new("test".to_string())));
        let bool_value = OvmValue::from_ast(Value::Boolean(true));

        assert_eq!(vm.get_type_name(&int_value), "int");
        assert_eq!(vm.get_type_name(&string_value), "string");
        assert_eq!(vm.get_type_name(&bool_value), "bool");
    }

    #[test]
    fn test_builtin_functions() {
        let vm = BytecodeVm::new();

        // Test len function (builtin_id = 0)
        let list_arg = OvmValue::from_ast(Value::List(
            vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)].into(),
        ));

        let result = vm.execute_builtin_call(0, &[list_arg]);
        assert!(result.is_ok());
        match result.unwrap().to_ast() {
            Ok(Value::Integer(n)) => assert_eq!(n, 3, "List length should be 3"),
            _ => panic!("Expected integer result"),
        }

        // Test toString function (builtin_id = 1)
        let int_arg = OvmValue::from_ast(Value::Integer(42));
        let result = vm.execute_builtin_call(1, &[int_arg]);
        assert!(result.is_ok());
        match result.unwrap().to_ast() {
            Ok(Value::String(s)) => assert_eq!(*s, "42", "Should convert to string"),
            _ => panic!("Expected string result"),
        }
    }

    #[test]
    fn test_register_allocator() {
        let mut allocator = RegisterAllocator::new();

        let reg1 = allocator.allocate_register();
        let reg2 = allocator.allocate_register();
        let reg3 = allocator.allocate_register();

        assert_eq!(reg1.0, 0);
        assert_eq!(reg2.0, 1);
        assert_eq!(reg3.0, 2);
        assert_eq!(allocator.max_register_used(), 3);

        // Test register reuse
        allocator.free_register(reg2);
        let reg4 = allocator.allocate_register();
        assert_eq!(reg4.0, 1, "Should reuse freed register");
    }

    #[test]
    fn test_instruction_emitter() {
        let mut emitter = InstructionEmitter::new();

        let reg1 = Register(0);
        let reg2 = Register(1);
        let const_idx = emitter.add_constant(OvmValue::from_ast(Value::Integer(42)));

        emitter.emit_load_const(reg1, const_idx);
        emitter.emit_add(reg2, reg1, reg1);
        emitter.emit_return(Some(reg2));

        assert!(emitter.has_return(), "Should have return instruction");

        let instructions = emitter.take_instructions();
        assert_eq!(instructions.len(), 3, "Should have 3 instructions");

        let constants = emitter.take_constants();
        assert_eq!(constants.len(), 1, "Should have 1 constant");
    }
}
