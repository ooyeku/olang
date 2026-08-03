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
    /// Current nesting depth of execute(); bounds Rust stack growth from
    /// recursive CallNamed so runaway recursion errors instead of aborting
    call_depth: u32,
    max_call_depth: u32,
}

/// Call frame for function execution
#[derive(Debug, Clone)]
struct CallFrame {
    #[allow(dead_code)]
    function_id: FunctionId,
    #[allow(dead_code)]
    return_address: usize,
    #[allow(dead_code)]
    base_register: usize,
    #[allow(dead_code)]
    local_count: usize,
}

/// Exception handler for error recovery
#[derive(Debug, Clone)]
struct ExceptionHandler {
    #[allow(dead_code)]
    handler_address: usize,
    #[allow(dead_code)]
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
    /// Variable name -> the register that holds it (a register window:
    /// parameters occupy registers 0..n, so reading a variable is free
    /// rather than a LoadLocal that clones out of a separate array)
    local_variables: HashMap<String, Register>,
    /// Builtin names the VM implements (for compile-time callee validation)
    builtin_names: std::collections::HashSet<String>,

    // Label tracking for control flow
    _label_counter: u32,

    // Function registry for calls
    function_registry: HashMap<String, FunctionId>,
}

/// Bytecode optimization engine
#[allow(dead_code)]
/// Placeholder for future optimization passes.
///
/// The previous pipeline (dead-code elimination, register renaming, peephole
/// rewrites, control-flow "optimization") was deleted rather than fixed: it
/// removed live control flow and stores, renamed registers for only a subset
/// of opcodes, and treated label IDs as instruction addresses. Passes may
/// return once they can be validated against the differential test suite.
pub struct BytecodeOptimizer {}

/// Compiled bytecode representation
#[derive(Debug, Clone)]
pub struct CompiledBytecode {
    pub function_id: FunctionId,
    pub instructions: Vec<Instruction>,
    pub register_count: u32,
    pub local_count: u32,
    /// Declared parameter count, enforced at call time
    pub param_count: usize,
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

    // Range operations
    MakeRange {
        dst: Register,
        start: Register,
        end: Register,
        inclusive: bool,
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
    #[allow(dead_code)]
    call_stack: Vec<StackFrame>,

    // Program counter
    pc: usize,

    // Exception state
    exception: Option<VmException>,

    // Current bytecode being executed
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
    max_registers: u32,
}

/// Instruction emitter
pub struct InstructionEmitter {
    instructions: Vec<Instruction>,
    /// Label id -> instruction offset where the label was placed
    label_positions: HashMap<u32, usize>,
    next_label_id: u32,
    constants: Vec<OvmValue>,
    constant_map: HashMap<String, u32>, // For deduplication
    current_line: u32,
    debug_info: BytecodeDebugInfo,
}

/// VM errors
#[derive(Debug, Error)]
pub enum BytecodeError {
    #[error("Compilation failed: {0}")]
    CompilationFailed(String),

    /// A call to a function the VM doesn't know yet. Reported separately from
    /// CompilationFailed so a caller can compile the callee and retry rather
    /// than giving up on the whole function.
    #[error("Unresolved callee: {0}")]
    UnresolvedCallee(String),

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

        // Only builtins the VM actually implements may be advertised —
        // registering names without implementations turned working programs
        // into runtime "Unknown builtin function" errors.
        builtin_registry.insert("len".to_string(), 0);
        builtin_registry.insert("to_string".to_string(), 1);

        Self {
            compiler: BytecodeCompiler::new(),
            bytecode_cache: Arc::new(RwLock::new(HashMap::new())),
            execution_state: ExecutionState::new(),
            stats: VmStatistics::default(),
            call_stack: Vec::new(),
            exception_handlers: Vec::new(),
            function_registry: HashMap::new(),
            builtin_registry,
            call_depth: 0,
            // Must match the interpreter's own limit: a program that recurses
            // 900 deep has to behave the same whether or not it was promoted
            max_call_depth: 1000,
        }
    }

    /// Register a function for dynamic calls
    pub fn register_function(&mut self, name: String, func_id: FunctionId) {
        self.function_registry.insert(name, func_id);
    }

    /// Withdraw a registration.
    ///
    /// Names are registered before compilation so recursive calls resolve; if
    /// compilation then fails the name must be withdrawn, or a later function
    /// will compile a call against an id that has no bytecode and fail at
    /// runtime with FunctionNotFound.
    pub fn unregister_function(&mut self, name: &str) {
        self.function_registry.remove(name);
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

        // Set up registries so the compiler can validate callees
        self.compiler.function_registry = self.function_registry.clone();
        self.compiler.builtin_names = self.builtin_registry.keys().cloned().collect();

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

        if self.call_depth >= self.max_call_depth {
            return Err(BytecodeError::RuntimeError(format!(
                "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                self.max_call_depth
            )));
        }
        self.call_depth += 1;

        // Give this call its own frame: nested calls (e.g. recursion through
        // CallNamed) re-enter execute(), and sharing one ExecutionState would
        // clobber the caller's registers and locals.
        let caller_state = std::mem::replace(&mut self.execution_state, ExecutionState::new());

        let result = (|| {
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
        })();

        // Restore the caller's frame on both success and error paths
        self.execution_state = caller_state;
        self.call_depth -= 1;

        result
    }

    /// Execute bytecode instructions - Complete implementation
    fn execute_bytecode(&mut self, bytecode: &CompiledBytecode) -> Result<OvmValue, BytecodeError> {
        let mut pc = bytecode.entry_point;

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
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::Add)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Sub { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::Subtract)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Mul { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::Multiply)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Div { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::Divide)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Mod { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::Modulo)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Neg { dst, src } => {
                    let result = self

                        .execute_unary_op(self.execution_state.register_ref(*src)?, UnaryOp::Negate)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                // Comparison operations
                Instruction::Eq { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::Equal)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Ne { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::NotEqual)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Lt { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::LessThan)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Le { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::LessThanEqual)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Gt { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::GreaterThan)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Ge { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_binary_op(left, right, BinaryOp::GreaterThanEqual)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                // Logical operations
                Instruction::And { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_logical_and(left, right)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Or { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_logical_or(left, right)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Not { dst, src } => {
                    let result = self

                        .execute_unary_op(self.execution_state.register_ref(*src)?, UnaryOp::Not)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                // Control flow
                Instruction::Jump { target } => {
                    pc = target.0 as usize;
                    continue;
                }

                Instruction::JumpIfTrue { condition, target } => {
                    if self.is_truthy(self.execution_state.register_ref(*condition)?) {
                        pc = target.0 as usize;
                        continue;
                    }
                }

                Instruction::JumpIfFalse { condition, target } => {
                    if !self.is_truthy(self.execution_state.register_ref(*condition)?) {
                        pc = target.0 as usize;
                        continue;
                    }
                }

                Instruction::Return { value } => {
                    if let Some(reg) = value {
                        return Ok(self.execution_state.get_register(*reg)?);
                    } else {
                        return Ok(OvmValue::new_unit());
                    }
                }

                // Function operations. The compiler only emits CallNamed;
                // a dynamic Call reaching the VM means a compilation bug, and
                // the old placeholder silently returned Unit for it.
                Instruction::Call { .. } => {
                    return Err(BytecodeError::RuntimeError(
                        "Dynamic function calls are not supported in the bytecode tier".to_string(),
                    ));
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
                    let mut list_values = Vec::with_capacity(elements.len());
                    for elem_reg in elements {
                        list_values.push(self.execution_state.get_register(*elem_reg)?);
                    }
                    self.execution_state
                        .set_register(*dst, OvmValue::new_list(list_values))?;
                }

                Instruction::MakeRange { dst, start, end, inclusive } => {
                    let start_val = self.execution_state.get_register(*start)?;
                    let end_val = self.execution_state.get_register(*end)?;
                    
                    // Extract integer values for the range
                    let start_int = match &start_val.data {
                        crate::ovm::value::ValueData::Integer(i) => *i,
                        _ => return Err(BytecodeError::RuntimeError("Range start must be integer".to_string())),
                    };
                    
                    let end_int = match &end_val.data {
                        crate::ovm::value::ValueData::Integer(i) => *i,
                        _ => return Err(BytecodeError::RuntimeError("Range end must be integer".to_string())),
                    };
                    
                    let range_value = Value::Range {
                        start: start_int,
                        end: end_int,
                        inclusive: *inclusive,
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::from_ast(range_value))?;
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
                    let mut tuple_values = Vec::with_capacity(elements.len());
                    for elem_reg in elements {
                        tuple_values.push(self.execution_state.get_register(*elem_reg)?);
                    }
                    self.execution_state
                        .set_register(*dst, OvmValue::new_tuple(tuple_values))?;
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
        use crate::ovm::value::ValueData;

        // Operate directly on ValueData — converting operands through the AST
        // representation on every instruction dominated the dispatch loop.
        let result = match (&left.data, &right.data) {
            (ValueData::Integer(a), ValueData::Integer(b)) => match op {
                BinaryOp::Add => OvmValue::new_integer(a.checked_add(*b).ok_or_else(|| {
                    BytecodeError::RuntimeError("Integer overflow in addition".to_string())
                })?),
                BinaryOp::Subtract => OvmValue::new_integer(a.checked_sub(*b).ok_or_else(|| {
                    BytecodeError::RuntimeError("Integer overflow in subtraction".to_string())
                })?),
                BinaryOp::Multiply => OvmValue::new_integer(a.checked_mul(*b).ok_or_else(|| {
                    BytecodeError::RuntimeError("Integer overflow in multiplication".to_string())
                })?),
                BinaryOp::Divide => {
                    if *b == 0 {
                        return Err(BytecodeError::DivisionByZero);
                    }
                    // checked_div also rejects i64::MIN / -1, which overflows
                    OvmValue::new_integer(a.checked_div(*b).ok_or_else(|| {
                        BytecodeError::RuntimeError("Integer overflow in division".to_string())
                    })?)
                }
                BinaryOp::Modulo => {
                    if *b == 0 {
                        return Err(BytecodeError::DivisionByZero);
                    }
                    OvmValue::new_integer(a.checked_rem(*b).ok_or_else(|| {
                        BytecodeError::RuntimeError("Integer overflow in modulo".to_string())
                    })?)
                }
                BinaryOp::Equal => OvmValue::new_boolean(a == b),
                BinaryOp::NotEqual => OvmValue::new_boolean(a != b),
                BinaryOp::LessThan => OvmValue::new_boolean(a < b),
                BinaryOp::LessThanEqual => OvmValue::new_boolean(a <= b),
                BinaryOp::GreaterThan => OvmValue::new_boolean(a > b),
                BinaryOp::GreaterThanEqual => OvmValue::new_boolean(a >= b),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Unsupported operation: {:?}",
                        op
                    )))
                }
            },
            (ValueData::Float(a), ValueData::Float(b)) => {
                self.execute_float_binary_op(*a, *b, op)?
            }
            (ValueData::Integer(a), ValueData::Float(b)) => {
                self.execute_float_binary_op(*a as f64, *b, op)?
            }
            (ValueData::Float(a), ValueData::Integer(b)) => {
                self.execute_float_binary_op(*a, *b as f64, op)?
            }
            (ValueData::String(a), ValueData::String(b)) => match op {
                BinaryOp::Add => OvmValue::new_string(format!("{}{}", a, b)),
                BinaryOp::Equal => OvmValue::new_boolean(a == b),
                BinaryOp::NotEqual => OvmValue::new_boolean(a != b),
                BinaryOp::LessThan => OvmValue::new_boolean(a < b),
                BinaryOp::LessThanEqual => OvmValue::new_boolean(a <= b),
                BinaryOp::GreaterThan => OvmValue::new_boolean(a > b),
                BinaryOp::GreaterThanEqual => OvmValue::new_boolean(a >= b),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Unsupported operation: {:?}",
                        op
                    )))
                }
            },
            (ValueData::Boolean(a), ValueData::Boolean(b)) => match op {
                BinaryOp::Equal => OvmValue::new_boolean(a == b),
                BinaryOp::NotEqual => OvmValue::new_boolean(a != b),
                BinaryOp::And => OvmValue::new_boolean(*a && *b),
                BinaryOp::Or => OvmValue::new_boolean(*a || *b),
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

        Ok(result)
    }

    /// Float arithmetic shared by the Float/Float and mixed Int/Float paths,
    /// mirroring the interpreter's coercion semantics
    fn execute_float_binary_op(&self, a: f64, b: f64, op: BinaryOp) -> Result<OvmValue, BytecodeError> {
        Ok(match op {
            BinaryOp::Add => OvmValue::new_float(a + b),
            BinaryOp::Subtract => OvmValue::new_float(a - b),
            BinaryOp::Multiply => OvmValue::new_float(a * b),
            BinaryOp::Divide => {
                if b == 0.0 {
                    return Err(BytecodeError::DivisionByZero);
                }
                OvmValue::new_float(a / b)
            }
            BinaryOp::Modulo => {
                if b == 0.0 {
                    return Err(BytecodeError::DivisionByZero);
                }
                OvmValue::new_float(a % b)
            }
            BinaryOp::Equal => OvmValue::new_boolean(a == b),
            BinaryOp::NotEqual => OvmValue::new_boolean(a != b),
            BinaryOp::LessThan => OvmValue::new_boolean(a < b),
            BinaryOp::LessThanEqual => OvmValue::new_boolean(a <= b),
            BinaryOp::GreaterThan => OvmValue::new_boolean(a > b),
            BinaryOp::GreaterThanEqual => OvmValue::new_boolean(a >= b),
            _ => {
                return Err(BytecodeError::TypeError(format!(
                    "Unsupported operation: {:?}",
                    op
                )))
            }
        })
    }

    /// Execute unary operation
    fn execute_unary_op(&self, value: &OvmValue, op: UnaryOp) -> Result<OvmValue, BytecodeError> {
        use crate::ovm::value::ValueData;

        match (&value.data, &op) {
            (ValueData::Integer(a), UnaryOp::Negate) => {
                Ok(OvmValue::new_integer(a.checked_neg().ok_or_else(|| {
                    BytecodeError::RuntimeError("Integer overflow in negation".to_string())
                })?))
            }
            (ValueData::Float(a), UnaryOp::Negate) => Ok(OvmValue::new_float(-a)),
            (ValueData::Boolean(a), UnaryOp::Not) => Ok(OvmValue::new_boolean(!a)),
            _ => Err(BytecodeError::TypeError(format!(
                "Unsupported unary operation: {:?}",
                op
            ))),
        }
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
                    // Char count, matching the interpreter's len builtin
                    Value::String(s) => {
                        Ok(OvmValue::from_ast(Value::Integer(s.chars().count() as i64)))
                    }
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
                let popped = list_vec.pop().ok_or_else(|| {
                    BytecodeError::RuntimeError("List became empty during pop operation".to_string())
                })?;
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
            Ok(Value::Enum { type_name: _, .. }) => {
                // Return a static string for enum types
                // In a real implementation, we might want to cache type names
                "enum"
            },
            Ok(Value::Promise { .. }) => "promise",
            Ok(Value::Map(_)) => "map",
            Err(_) => "unknown",
            Ok(crate::ast::Value::TypeInfo { .. }) => "type",
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
        use crate::ovm::value::ValueData;

        match &value.data {
            ValueData::Boolean(b) => *b,
            ValueData::Integer(i) => *i != 0,
            ValueData::Float(f) => *f != 0.0,
            ValueData::Unit => false,
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
                // Slice by chars: byte slicing panics inside multibyte
                // characters (and on reversed indices)
                let start_idx = start_idx.max(0) as usize;
                let end_idx = end_idx.max(0) as usize;
                let slice: String = if end_idx > start_idx {
                    s.chars().skip(start_idx).take(end_idx - start_idx).collect()
                } else {
                    String::new()
                };
                Ok(OvmValue::from_ast(Value::String(Arc::new(slice))))
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
        }
    }

    pub fn prepare_for_execution(
        &mut self,
        bytecode: &CompiledBytecode,
        args: &[OvmValue],
    ) -> Result<(), BytecodeError> {
        if args.len() != bytecode.param_count {
            return Err(BytecodeError::RuntimeError(format!(
                "Function expects {} argument(s), got {}",
                bytecode.param_count,
                args.len()
            )));
        }

        // Allocate registers
        self.registers.clear();
        self.registers
            .resize(bytecode.register_count as usize, OvmValue::new_unit());

        // Arguments occupy the first registers (the compiler assigns
        // parameters registers 0..n in declaration order)
        for (i, arg) in args.iter().enumerate() {
            if i < self.registers.len() {
                self.registers[i] = arg.clone();
            }
        }

        self.locals.clear();
        self.locals
            .resize(bytecode.local_count as usize, OvmValue::new_unit());

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

    /// Borrow a register without cloning. Cloning an OvmValue copies its
    /// header (three atomics), which dominated the dispatch loop when every
    /// operand read went through get_register.
    #[inline]
    pub fn register_ref(&self, reg: Register) -> Result<&OvmValue, BytecodeError> {
        self.registers
            .get(reg.0 as usize)
            .ok_or(BytecodeError::InvalidRegister(reg))
    }

    /// Borrow two registers at once (operands of a binary instruction).
    #[inline]
    pub fn register_pair(
        &self,
        lhs: Register,
        rhs: Register,
    ) -> Result<(&OvmValue, &OvmValue), BytecodeError> {
        Ok((self.register_ref(lhs)?, self.register_ref(rhs)?))
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
            builtin_names: std::collections::HashSet::new(),
            _label_counter: 0,
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

        // Parameters occupy the first registers, in declaration order
        for param in &func.parameters {
            let reg = self.register_allocator.allocate_register();
            self.local_variables.insert(param.name.clone(), reg);
        }

        // Compile function body
        let result_reg = self.compile_expression(&func.body)?;

        // Ensure function returns
        if !self.emitter.has_return() {
            self.emitter.emit_return(Some(result_reg));
        }

        // Patch jump targets from label ids to instruction offsets
        self.emitter.resolve_labels()?;

        let mut instructions = self.emitter.take_instructions();
        let constants = self.emitter.take_constants();

        instructions = self.optimizer.optimize_instructions(instructions)?;

        Ok(CompiledBytecode {
            function_id: func_id,
            instructions,
            register_count: self.register_allocator.max_register_used(),
            local_count: 0,
            param_count: func.parameters.len(),
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
                if let Some(&reg) = self.local_variables.get(name) {
                    // The variable already lives in a register — nothing to emit
                    Ok(reg)
                } else {
                    // Refuse to compile references we can't resolve — loading
                    // Unit instead silently changed program results when a
                    // function was promoted to the bytecode tier
                    Err(BytecodeError::CompilationFailed(format!(
                        "Unresolved identifier '{}' (globals/closures not supported in bytecode tier)",
                        name
                    )))
                }
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
                    BinaryOp::Modulo => self.emitter.emit_mod(dst_reg, left_reg, right_reg),
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

            Expr::If { condition, then_branch, else_branch } => {
                // Compile condition
                let condition_reg = self.compile_expression(condition)?;
                
                // Create labels for branches
                let _then_label = self.emitter.create_label();
                let else_label = self.emitter.create_label();
                let end_label = self.emitter.create_label();
                
                // Branch on condition
                self.emitter.emit_branch_if_false(condition_reg, else_label);
                
                // Compile then branch
                let then_reg = self.compile_expression(then_branch)?;
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_move(dst_reg, then_reg);
                self.emitter.emit_jump(end_label);
                
                // Else branch
                self.emitter.place_label(else_label);
                if let Some(else_expr) = else_branch {
                    let else_reg = self.compile_expression(else_expr)?;
                    self.emitter.emit_move(dst_reg, else_reg);
                } else {
                    // No else branch, use unit
                    let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                    self.emitter.emit_load_const(dst_reg, const_idx);
                }
                
                self.emitter.place_label(end_label);
                Ok(dst_reg)
            }

            Expr::Range { start, end, inclusive } => {
                // Compile start and end expressions
                let start_reg = self.compile_expression(start)?;
                let end_reg = self.compile_expression(end)?;
                
                // Create range value - for now, we'll create a constant range
                // In a full implementation, this would handle dynamic ranges
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_make_range(dst_reg, start_reg, end_reg, *inclusive);
                Ok(dst_reg)
            }

            Expr::Call { callee, arguments } => {
                // Only direct calls to named functions/builtins are supported;
                // the callee is resolved by name at runtime through the VM's
                // registries (which also makes recursion work).
                let function_name = match callee.as_ref() {
                    Expr::Identifier(name) => name.clone(),
                    other => {
                        return Err(BytecodeError::CompilationFailed(format!(
                            "Unsupported callee in bytecode tier: {:?}",
                            std::mem::discriminant(other)
                        )))
                    }
                };

                // Reject callees the VM can't resolve at compile time rather
                // than failing mid-execution
                if !self.builtin_names.contains(&function_name)
                    && !self.function_registry.contains_key(&function_name)
                {
                    return Err(BytecodeError::UnresolvedCallee(function_name));
                }

                let mut arg_regs = Vec::new();
                for argument in arguments {
                    match argument {
                        crate::ast::Argument::Positional(expr) => {
                            arg_regs.push(self.compile_expression(expr)?);
                        }
                        crate::ast::Argument::Named { .. } => {
                            return Err(BytecodeError::CompilationFailed(
                                "Named arguments are not supported in the bytecode tier"
                                    .to_string(),
                            ))
                        }
                    }
                }

                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::CallNamed {
                    dst: dst_reg,
                    function_name,
                    args: arg_regs,
                });
                Ok(dst_reg)
            }

            Expr::Block(statements) => {
                // A block evaluates its statements in order; its value is the
                // value of the last statement (Unit for an empty block).
                let mut result_reg = None;
                for statement in statements.iter() {
                    result_reg = Some(self.compile_statement(statement)?);
                }
                match result_reg {
                    Some(reg) => Ok(reg),
                    None => {
                        let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                        let dst_reg = self.register_allocator.allocate_register();
                        self.emitter.emit_load_const(dst_reg, const_idx);
                        Ok(dst_reg)
                    }
                }
            }

            Expr::Assignment { target, value } => {
                let target_reg = match self.local_variables.get(target) {
                    Some(&reg) => reg,
                    None => {
                        return Err(BytecodeError::CompilationFailed(format!(
                            "Assignment to unresolved variable '{}' (globals not supported in bytecode tier)",
                            target
                        )))
                    }
                };
                let value_reg = self.compile_expression(value)?;
                if value_reg != target_reg {
                    self.emitter.emit_move(target_reg, value_reg);
                }
                Ok(target_reg)
            }

            Expr::WhileLoop { condition, body } => {
                let loop_start = self.emitter.create_label();
                let loop_end = self.emitter.create_label();

                self.emitter.place_label(loop_start);
                let condition_reg = self.compile_expression(condition)?;
                self.emitter.emit_branch_if_false(condition_reg, loop_end);

                self.compile_expression(body)?;
                self.emitter.emit_jump(loop_start);

                self.emitter.place_label(loop_end);

                // While loops evaluate to Unit
                let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }

            other => {
                // Refuse to compile unsupported expressions — substituting a
                // Unit constant (e.g. for a recursive call site) silently
                // changed program results on promotion to the bytecode tier
                Err(BytecodeError::CompilationFailed(format!(
                    "Unsupported expression in bytecode tier: {:?}",
                    std::mem::discriminant(other)
                )))
            }
        }
    }

    /// Compile a statement inside a block, returning the register holding its
    /// value (let-declarations evaluate to Unit like in the interpreter).
    fn compile_statement(
        &mut self,
        statement: &crate::ast::Statement,
    ) -> Result<Register, BytecodeError> {
        match statement {
            crate::ast::Statement::Expression(expr) => self.compile_expression(expr),
            crate::ast::Statement::LetDecl(let_decl) => {
                let name = match &let_decl.pattern {
                    crate::ast::Pattern::Identifier(name) => name.clone(),
                    other => {
                        return Err(BytecodeError::CompilationFailed(format!(
                            "Unsupported let pattern in bytecode tier: {:?}",
                            std::mem::discriminant(other)
                        )))
                    }
                };
                let value_reg = match &let_decl.value {
                    Some(expr) => self.compile_expression(expr)?,
                    None => {
                        let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                        let reg = self.register_allocator.allocate_register();
                        self.emitter.emit_load_const(reg, const_idx);
                        reg
                    }
                };
                // Bind the name to its own register so later assignments
                // don't clobber the (possibly shared) value register
                let var_reg = self.register_allocator.allocate_register();
                self.local_variables.insert(name, var_reg);
                self.emitter.emit_move(var_reg, value_reg);

                // Let evaluates to Unit
                let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }
            other => Err(BytecodeError::CompilationFailed(format!(
                "Unsupported statement in bytecode tier: {:?}",
                std::mem::discriminant(other)
            ))),
        }
    }
}

// Implementation stubs for optimization components
impl RegisterAllocator {
    pub fn new() -> Self {
        Self {
            next_register: 0,
            free_registers: Vec::new(),
            max_registers: 0,
        }
    }

    pub fn reset(&mut self) {
        self.next_register = 0;
        self.free_registers.clear();
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
            label_positions: HashMap::new(),
            next_label_id: 0,
            constants: Vec::new(),
            constant_map: HashMap::new(),
            current_line: 0,
            debug_info: BytecodeDebugInfo::default(),
        }
    }

    pub fn reset(&mut self) {
        self.instructions.clear();
        self.label_positions.clear();
        self.next_label_id = 0;
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

    pub fn emit_mod(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Mod { dst, lhs, rhs });
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

    pub fn emit_branch_if_false(&mut self, condition: Register, target: Label) {
        self.instructions.push(Instruction::JumpIfFalse { condition, target });
    }

    pub fn emit_jump(&mut self, target: Label) {
        self.instructions.push(Instruction::Jump { target });
    }

    pub fn emit_move(&mut self, dst: Register, src: Register) {
        self.instructions.push(Instruction::Move { dst, src });
    }

    pub fn emit_make_range(&mut self, dst: Register, start: Register, end: Register, inclusive: bool) {
        self.instructions.push(Instruction::MakeRange { dst, start, end, inclusive });
    }

    pub fn create_label(&mut self) -> Label {
        let label = Label(self.next_label_id);
        self.next_label_id += 1;
        label
    }

    /// Bind a label to the current instruction offset.
    pub fn place_label(&mut self, label: Label) {
        self.label_positions.insert(label.0, self.instructions.len());
    }

    /// Patch every jump target from a label id to the instruction offset the
    /// label was placed at. Must run after emission, before execution — a
    /// label id is meaningless as a program counter.
    pub fn resolve_labels(&mut self) -> Result<(), BytecodeError> {
        let resolve = |target: &mut Label, positions: &HashMap<u32, usize>| {
            match positions.get(&target.0) {
                Some(&offset) => {
                    *target = Label(offset as u32);
                    Ok(())
                }
                None => Err(BytecodeError::CompilationFailed(format!(
                    "Jump references unplaced label {}",
                    target.0
                ))),
            }
        };

        for instruction in &mut self.instructions {
            match instruction {
                Instruction::Jump { target }
                | Instruction::JumpIfTrue { target, .. }
                | Instruction::JumpIfFalse { target, .. } => {
                    resolve(target, &self.label_positions)?;
                }
                _ => {}
            }
        }
        Ok(())
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
        Self {}
    }

    /// No-op: correctness first. See the struct docs for why the previous
    /// pipeline was removed.
    pub fn optimize_instructions(
        &mut self,
        instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, BytecodeError> {
        Ok(instructions)
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
                    default_value: None,
                },
                Parameter {
                    name: "b".to_string(),
                    type_annotation: Some(TypeAnnotation::Int),
                    default_value: None,
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

    #[test]
    fn test_bytecode_execution_arithmetic() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a simpler function first: fn simple() -> Int = 5 + 3
        let func = FunctionDecl {
            name: "simple".to_string(),
            type_params: vec![],
            parameters: vec![],
            body: Expr::BinaryOp {
                op: BinaryOp::Add,
                left: Box::new(Expr::Integer(5)),
                right: Box::new(Expr::Integer(3)),
            },
            return_type: None,
        };

        // Compile the function
        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function compilation should succeed: {:?}", result.err());

        // Execute the function with no arguments
        let args = vec![];

        let result = vm.execute(func_id, &args);
        assert!(result.is_ok(), "Function execution should succeed: {:?}", result.err());

        let result_value = result.unwrap();
        if let crate::ovm::value::ValueData::Integer(val) = result_value.data {
            assert_eq!(val, 8, "5 + 3 should equal 8");
        } else {
            panic!("Expected integer result, got: {:?}", result_value);
        }
    }

    #[test]
    fn test_bytecode_execution_with_optimizations() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a function with constant folding opportunity: fn const_expr() -> Int = 10 + 20 + 30
        let func = FunctionDecl {
            name: "const_expr".to_string(),
            type_params: vec![],
            parameters: vec![],
            body: Expr::BinaryOp {
                op: BinaryOp::Add,
                left: Box::new(Expr::BinaryOp {
                    op: BinaryOp::Add,
                    left: Box::new(Expr::Integer(10)),
                    right: Box::new(Expr::Integer(20)),
                }),
                right: Box::new(Expr::Integer(30)),
            },
            return_type: None,
        };

        // Compile the function (should apply constant folding optimization)
        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function compilation should succeed");

        // Execute the function
        let args = vec![];
        let result = vm.execute(func_id, &args);
        assert!(result.is_ok(), "Function execution should succeed");

        let result_value = result.unwrap();
        if let crate::ovm::value::ValueData::Integer(val) = result_value.data {
            assert_eq!(val, 60, "10 + 20 + 30 should equal 60");
        } else {
            panic!("Expected integer result, got: {:?}", result_value);
        }
    }

    #[test]
    fn test_bytecode_execution_control_flow() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a function with conditional: fn simple_if() -> Int = if 10 > 5 then 10 else 5
        let func = FunctionDecl {
            name: "simple_if".to_string(),
            type_params: vec![],
            parameters: vec![],
            body: Expr::If {
                condition: Box::new(Expr::BinaryOp {
                    op: BinaryOp::GreaterThan,
                    left: Box::new(Expr::Integer(10)),
                    right: Box::new(Expr::Integer(5)),
                }),
                then_branch: Box::new(Expr::Integer(10)),
                else_branch: Some(Box::new(Expr::Integer(5))),
            },
            return_type: None,
        };

        // Compile the function
        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function compilation should succeed");

        // Test the condition (10 > 5 is true, so should return 10)
        let args = vec![];
        let result = vm.execute(func_id, &args);
        assert!(result.is_ok(), "Function execution should succeed");
        
        let result_value = result.unwrap();
        if let crate::ovm::value::ValueData::Integer(val) = result_value.data {
            assert_eq!(val, 10, "if 10 > 5 then 10 else 5 should return 10");
        } else {
            panic!("Expected integer result, got: {:?}", result_value);
        }
    }

    #[test]
    fn test_bytecode_execution_range_operations() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a function that works with ranges: fn range_test() -> Range = 1..10
        let func = FunctionDecl {
            name: "range_test".to_string(),
            type_params: vec![],
            parameters: vec![],
            body: Expr::Range {
                start: Box::new(Expr::Integer(1)),
                end: Box::new(Expr::Integer(10)),
                inclusive: false,
            },
            return_type: None,
        };

        // Compile the function
        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function compilation should succeed");

        // Execute the function
        let args = vec![];
        let result = vm.execute(func_id, &args);
        assert!(result.is_ok(), "Function execution should succeed");

        let result_value = result.unwrap();
        if let crate::ovm::value::ValueData::Range(range) = &result_value.data {
            assert_eq!(range.start, 1, "Range start should be 1");
            assert_eq!(range.end, 10, "Range end should be 10");
            assert_eq!(range.inclusive, false, "Range should not be inclusive");
        } else {
            panic!("Expected range result, got: {:?}", result_value);
        }
    }

    #[test] 
    fn test_bytecode_execution_stats() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a simple function
        let func = FunctionDecl {
            name: "simple".to_string(),
            type_params: vec![],
            parameters: vec![],
            body: Expr::Integer(42),
            return_type: None,
        };

        // Compile and execute multiple times to generate stats
        vm.compile_function(func_id, &func).unwrap();
        
        for _ in 0..5 {
            let _ = vm.execute(func_id, &[]);
        }

        // Check that statistics are being tracked
        let stats = vm.get_stats();
        assert!(stats.instructions_executed > 0, "Should have executed instructions");
        assert!(stats.function_calls >= 5, "Should have recorded function calls");
    }

    #[test]
    fn test_bytecode_error_handling() {
        let mut vm = BytecodeVm::new();
        let invalid_func_id = FunctionId::new();

        // Try to execute a function that doesn't exist
        let result = vm.execute(invalid_func_id, &[]);
        assert!(result.is_err(), "Should fail to execute non-existent function");

        // Try to execute with wrong number of arguments
        let func_id = FunctionId::new();
        let func = FunctionDecl {
            name: "two_param".to_string(),
            type_params: vec![],
            parameters: vec![
                crate::ast::Parameter { 
                    name: "x".to_string(), 
                    type_annotation: None,
                    default_value: None,
                },
                crate::ast::Parameter { 
                    name: "y".to_string(), 
                    type_annotation: None,
                    default_value: None,
                },
            ],
            body: Expr::Identifier("x".to_string()),
            return_type: None,
        };

        vm.compile_function(func_id, &func).unwrap();
        
        // Execute with wrong number of args
        let result = vm.execute(func_id, &[OvmValue::new_integer(1)]); // Should need 2 args
        assert!(result.is_err(), "Should fail with wrong number of arguments");
    }
}
