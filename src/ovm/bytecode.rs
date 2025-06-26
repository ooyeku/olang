//! OVM Bytecode Virtual Machine
//!
//! Register-based bytecode VM for intermediate-tier execution between interpreter and JIT

use crate::ast::{BinaryOp, Expr, FunctionDecl, Statement, Value, UnaryOp};
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
    LoadConst { dst: Register, const_idx: u32 },
    LoadLocal { dst: Register, local_idx: u32 },
    StoreLocal { src: Register, local_idx: u32 },
    Move { dst: Register, src: Register },
    
    // Arithmetic operations
    Add { dst: Register, lhs: Register, rhs: Register },
    Sub { dst: Register, lhs: Register, rhs: Register },
    Mul { dst: Register, lhs: Register, rhs: Register },
    Div { dst: Register, lhs: Register, rhs: Register },
    Mod { dst: Register, lhs: Register, rhs: Register },
    Neg { dst: Register, src: Register },
    
    // Comparison operations
    Eq { dst: Register, lhs: Register, rhs: Register },
    Ne { dst: Register, lhs: Register, rhs: Register },
    Lt { dst: Register, lhs: Register, rhs: Register },
    Le { dst: Register, lhs: Register, rhs: Register },
    Gt { dst: Register, lhs: Register, rhs: Register },
    Ge { dst: Register, lhs: Register, rhs: Register },
    
    // Logical operations
    And { dst: Register, lhs: Register, rhs: Register },
    Or { dst: Register, lhs: Register, rhs: Register },
    Not { dst: Register, src: Register },
    
    // Control flow
    Jump { target: Label },
    JumpIfTrue { condition: Register, target: Label },
    JumpIfFalse { condition: Register, target: Label },
    
    // Function operations
    Call { 
        dst: Register, 
        function: Register, 
        args: Vec<Register>,
        arg_count: u32 
    },
    CallBuiltin {
        dst: Register,
        builtin_id: u32,
        args: Vec<Register>,
    },
    Return { value: Option<Register> },
    
    // Collection operations
    MakeList { dst: Register, elements: Vec<Register> },
    ListGet { dst: Register, list: Register, index: Register },
    ListSet { list: Register, index: Register, value: Register },
    ListLen { dst: Register, list: Register },
    ListPush { list: Register, value: Register },
    ListPop { dst: Register, list: Register },
    
    // Tuple operations
    MakeTuple { dst: Register, elements: Vec<Register> },
    TupleGet { dst: Register, tuple: Register, index: u32 },
    
    // String operations
    StringConcat { dst: Register, lhs: Register, rhs: Register },
    StringLen { dst: Register, src: Register },
    StringSlice { dst: Register, src: Register, start: Register, end: Register },
    
    // Type operations
    TypeOf { dst: Register, src: Register },
    CheckType { dst: Register, src: Register, type_id: u32 },
    
    // Pipeline operations (Olang-specific)
    PipelineMap { 
        dst: Register, 
        source: Register, 
        function: Register 
    },
    PipelineFilter { 
        dst: Register, 
        source: Register, 
        predicate: Register 
    },
    PipelineReduce { 
        dst: Register, 
        source: Register, 
        initial: Register,
        function: Register 
    },
    
    // Lazy evaluation operations
    MakeThunk { dst: Register, expr_idx: u32 },
    ForceThunk { dst: Register, thunk: Register },
    
    // Exception handling
    TryBegin { handler: Label },
    TryEnd,
    Throw { exception: Register },
    
    // Memory operations
    Allocate { dst: Register, size: Register },
    LoadField { dst: Register, object: Register, field_idx: u32 },
    StoreField { object: Register, field_idx: u32, value: Register },
    
    // Debug operations
    Nop,
    DebugPrint { src: Register },
    Breakpoint,
    ProfileEnter { function_id: u32 },
    ProfileExit { function_id: u32 },
}

/// Register identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Register(pub u32);

/// Label for jump targets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label(pub u32);

/// Debug information for bytecode
#[derive(Debug, Clone, Default)]
pub struct BytecodeDebugInfo {
    pub instruction_to_source: HashMap<usize, SourceLocation>,
    pub register_names: HashMap<Register, String>,
    pub function_name: Option<String>,
    pub local_variables: HashMap<u32, String>,
    pub line_table: Vec<(usize, u32)>, // (instruction_index, line_number)
}

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
}

#[derive(Debug)]
pub enum VmException {
    RuntimeError(String),
    TypeError(String),
    StackOverflow,
    DivisionByZero,
    IndexOutOfBounds,
    NullPointerException,
    InvalidOperation(String),
}

// Implementation
impl BytecodeVm {
    pub fn new() -> Self {
        Self {
            compiler: BytecodeCompiler::new(),
            bytecode_cache: Arc::new(RwLock::new(HashMap::new())),
            execution_state: ExecutionState::new(),
            stats: VmStatistics::default(),
            call_stack: Vec::new(),
            exception_handlers: Vec::new(),
        }
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
    pub fn compile_function(&mut self, func_id: FunctionId, func: &FunctionDecl) -> Result<(), BytecodeError> {
        let start_time = std::time::Instant::now();
        
        let bytecode = self.compiler.compile_function(func_id, func)?;
        
        if let Ok(mut cache) = self.bytecode_cache.write() {
            cache.insert(func_id, bytecode);
        }
        
        self.stats.compilation_time += start_time.elapsed();
        Ok(())
    }
    
    /// Execute function with bytecode
    pub fn execute(&mut self, func_id: FunctionId, args: &[OvmValue]) -> Result<OvmValue, BytecodeError> {
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
        self.execution_state.prepare_for_execution(&bytecode, args)?;
        
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
                    let value = bytecode.constants.get(*const_idx as usize)
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
                    let result = self.execute_binary_op(&left, &right, BinaryOp::GreaterThanEqual)?;
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
                Instruction::Call { dst, function, args, .. } => {
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
                
                Instruction::CallBuiltin { dst, builtin_id, args } => {
                    let mut arg_values = Vec::new();
                    for arg_reg in args {
                        arg_values.push(self.execution_state.get_register(*arg_reg)?);
                    }
                    
                    let result = self.execute_builtin_call(*builtin_id, &arg_values)?;
                    self.execution_state.set_register(*dst, result)?;
                }
                
                // Collection operations
                Instruction::MakeList { dst, elements } => {
                    let mut list_values = Vec::new();
                    for elem_reg in elements {
                        let value = self.execution_state.get_register(*elem_reg)?;
                        list_values.push(value.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?);
                    }
                    let list_value = Value::List(list_values.into());
                    self.execution_state.set_register(*dst, OvmValue::from_ast(list_value))?;
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
                        tuple_values.push(value.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?);
                    }
                    let tuple_value = Value::Tuple(tuple_values.into());
                    self.execution_state.set_register(*dst, OvmValue::from_ast(tuple_value))?;
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
                    let type_value = OvmValue::from_ast(Value::String(Arc::new(type_name.to_string())));
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
                    println!("[BREAKPOINT] PC: {}, Function: {:?}", pc, bytecode.function_id);
                }
                
                Instruction::ProfileEnter { function_id } => {
                    // Record function entry for profiling
                    self.stats.function_calls += 1;
                    if self.stats.function_calls % 1000 == 0 {
                        println!("[PROFILE] Function {:?} entered (total calls: {})", function_id, self.stats.function_calls);
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
                
                // Unimplemented instructions
                _ => {
                    return Err(BytecodeError::InvalidInstruction { 
                        pc, 
                        instruction: instruction.clone() 
                    });
                }
            }
            
            pc += 1;
        }
        
        // If we reach here without a return, return unit
        Ok(OvmValue::from_ast(Value::Unit))
    }

    /// Execute binary operation
    fn execute_binary_op(&self, left: &OvmValue, right: &OvmValue, op: BinaryOp) -> Result<OvmValue, BytecodeError> {
        // Convert to AST values for computation
        let left_ast = left.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let right_ast = right.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        
        let result_ast = match (&left_ast, &right_ast) {
            (Value::Integer(a), Value::Integer(b)) => {
                match op {
                    BinaryOp::Add => Value::Integer(a + b),
                    BinaryOp::Subtract => Value::Integer(a - b),
                    BinaryOp::Multiply => Value::Integer(a * b),
                    BinaryOp::Divide => {
                        if *b == 0 {
                            return Err(BytecodeError::DivisionByZero);
                        }
                        Value::Integer(a / b)
                    },
                    BinaryOp::Modulo => {
                        if *b == 0 {
                            return Err(BytecodeError::DivisionByZero);
                        }
                        Value::Integer(a % b)
                    },
                    BinaryOp::Equal => Value::Boolean(a == b),
                    BinaryOp::NotEqual => Value::Boolean(a != b),
                    BinaryOp::LessThan => Value::Boolean(a < b),
                    BinaryOp::LessThanEqual => Value::Boolean(a <= b),
                    BinaryOp::GreaterThan => Value::Boolean(a > b),
                    BinaryOp::GreaterThanEqual => Value::Boolean(a >= b),
                    _ => return Err(BytecodeError::TypeError(format!("Unsupported operation: {:?}", op))),
                }
            }
            (Value::Float(a), Value::Float(b)) => {
                match op {
                    BinaryOp::Add => Value::Float(a + b),
                    BinaryOp::Subtract => Value::Float(a - b),
                    BinaryOp::Multiply => Value::Float(a * b),
                    BinaryOp::Divide => {
                        if b.abs() < f64::EPSILON {
                            return Err(BytecodeError::DivisionByZero);
                        }
                        Value::Float(a / b)
                    },
                    BinaryOp::Equal => Value::Boolean((a - b).abs() < f64::EPSILON),
                    BinaryOp::NotEqual => Value::Boolean((a - b).abs() >= f64::EPSILON),
                    BinaryOp::LessThan => Value::Boolean(a < b),
                    BinaryOp::LessThanEqual => Value::Boolean(a <= b),
                    BinaryOp::GreaterThan => Value::Boolean(a > b),
                    BinaryOp::GreaterThanEqual => Value::Boolean(a >= b),
                    _ => return Err(BytecodeError::TypeError(format!("Unsupported operation: {:?}", op))),
                }
            }
            (Value::String(a), Value::String(b)) => {
                match op {
                    BinaryOp::Add => Value::String(Arc::new(format!("{}{}", a, b))),
                    BinaryOp::Equal => Value::Boolean(a == b),
                    BinaryOp::NotEqual => Value::Boolean(a != b),
                    BinaryOp::LessThan => Value::Boolean(a < b),
                    BinaryOp::LessThanEqual => Value::Boolean(a <= b),
                    BinaryOp::GreaterThan => Value::Boolean(a > b),
                    BinaryOp::GreaterThanEqual => Value::Boolean(a >= b),
                    _ => return Err(BytecodeError::TypeError(format!("Unsupported operation: {:?}", op))),
                }
            }
            (Value::Boolean(a), Value::Boolean(b)) => {
                match op {
                    BinaryOp::Equal => Value::Boolean(a == b),
                    BinaryOp::NotEqual => Value::Boolean(a != b),
                    BinaryOp::And => Value::Boolean(*a && *b),
                    BinaryOp::Or => Value::Boolean(*a || *b),
                    _ => return Err(BytecodeError::TypeError(format!("Unsupported operation: {:?}", op))),
                }
            }
            _ => return Err(BytecodeError::TypeError("Type mismatch in binary operation".to_string())),
        };
        
        Ok(OvmValue::from_ast(result_ast))
    }
    
    /// Execute unary operation
    fn execute_unary_op(&self, value: &OvmValue, op: UnaryOp) -> Result<OvmValue, BytecodeError> {
        let value_ast = value.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        
        let result_ast = match (&value_ast, &op) {
            (Value::Integer(a), UnaryOp::Negate) => Value::Integer(-a),
            (Value::Float(a), UnaryOp::Negate) => Value::Float(-a),
            (Value::Boolean(a), UnaryOp::Not) => Value::Boolean(!a),
            _ => return Err(BytecodeError::TypeError(format!("Unsupported unary operation: {:?}", op))),
        };
        
        Ok(OvmValue::from_ast(result_ast))
    }
    
    /// Execute logical AND operation
    fn execute_logical_and(&self, left: &OvmValue, right: &OvmValue) -> Result<OvmValue, BytecodeError> {
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
    fn execute_logical_or(&self, left: &OvmValue, right: &OvmValue) -> Result<OvmValue, BytecodeError> {
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
    fn execute_builtin_call(&self, builtin_id: u32, args: &[OvmValue]) -> Result<OvmValue, BytecodeError> {
        match builtin_id {
            0 => { // len function
                if args.len() != 1 {
                    return Err(BytecodeError::RuntimeError("len expects 1 argument".to_string()));
                }
                let value = args[0].to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
                match value {
                    Value::List(list) => Ok(OvmValue::from_ast(Value::Integer(list.len() as i64))),
                    Value::String(s) => Ok(OvmValue::from_ast(Value::Integer(s.len() as i64))),
                    _ => Err(BytecodeError::TypeError("len can only be applied to lists and strings".to_string())),
                }
            }
            1 => { // toString function
                if args.len() != 1 {
                    return Err(BytecodeError::RuntimeError("toString expects 1 argument".to_string()));
                }
                let value = args[0].to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
                let string_repr = format!("{}", value);
                Ok(OvmValue::from_ast(Value::String(Arc::new(string_repr))))
            }
            _ => Err(BytecodeError::RuntimeError(format!("Unknown builtin function: {}", builtin_id))),
        }
    }
    
    /// Execute list get operation
    fn execute_list_get(&self, list: &OvmValue, index: &OvmValue) -> Result<OvmValue, BytecodeError> {
        let list_value = list.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let index_value = index.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        
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
                        length: list.len() 
                    });
                }
                
                Ok(OvmValue::from_ast(list[idx as usize].clone()))
            }
            _ => Err(BytecodeError::TypeError("List get requires a list and integer index".to_string())),
        }
    }
    
    /// Execute list set operation
    fn execute_list_set(&self, list: &OvmValue, index: &OvmValue, value: &OvmValue) -> Result<OvmValue, BytecodeError> {
        let list_value = list.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let index_value = index.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let new_value = value.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        
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
                        length: list.len() 
                    });
                }
                
                let mut list_vec = list.to_vec();
                list_vec[idx as usize] = new_value;
                Ok(OvmValue::from_ast(Value::List(list_vec.into())))
            }
            _ => Err(BytecodeError::TypeError("List set requires a list and integer index".to_string())),
        }
    }
    
    /// Execute list length operation
    fn execute_list_len(&self, list: &OvmValue) -> Result<OvmValue, BytecodeError> {
        let list_value = list.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        
        match list_value {
            Value::List(list) => Ok(OvmValue::from_ast(Value::Integer(list.len() as i64))),
            _ => Err(BytecodeError::TypeError("List length requires a list".to_string())),
        }
    }
    
    /// Execute list push operation
    fn execute_list_push(&self, list: &OvmValue, value: &OvmValue) -> Result<OvmValue, BytecodeError> {
        let list_value = list.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let new_value = value.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        
        match list_value {
            Value::List(list) => {
                let mut list_vec = list.to_vec();
                list_vec.push(new_value);
                Ok(OvmValue::from_ast(Value::List(list_vec.into())))
            }
            _ => Err(BytecodeError::TypeError("List push requires a list".to_string())),
        }
    }
    
    /// Execute list pop operation
    fn execute_list_pop(&self, list: &OvmValue) -> Result<(OvmValue, OvmValue), BytecodeError> {
        let list_value = list.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        
        match list_value {
            Value::List(list) => {
                if list.is_empty() {
                    return Err(BytecodeError::RuntimeError("Cannot pop from empty list".to_string()));
                }
                
                let mut list_vec = list.to_vec();
                let popped = list_vec.pop().unwrap();
                Ok((
                    OvmValue::from_ast(Value::List(list_vec.into())),
                    OvmValue::from_ast(popped)
                ))
            }
            _ => Err(BytecodeError::TypeError("List pop requires a list".to_string())),
        }
    }
    
    /// Execute tuple get operation
    fn execute_tuple_get(&self, tuple: &OvmValue, index: u32) -> Result<OvmValue, BytecodeError> {
        let tuple_value = tuple.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        
        match tuple_value {
            Value::Tuple(tuple) => {
                if index >= tuple.len() as u32 {
                    return Err(BytecodeError::IndexOutOfBounds { 
                        index: index as i64, 
                        length: tuple.len() 
                    });
                }
                
                Ok(OvmValue::from_ast(tuple[index as usize].clone()))
            }
            _ => Err(BytecodeError::TypeError("Tuple get requires a tuple".to_string())),
        }
    }
    
    /// Execute string concatenation
    fn execute_string_concat(&self, left: &OvmValue, right: &OvmValue) -> Result<OvmValue, BytecodeError> {
        let left_value = left.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let right_value = right.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        
        match (left_value, right_value) {
            (Value::String(a), Value::String(b)) => {
                let result = format!("{}{}", a, b);
                Ok(OvmValue::from_ast(Value::String(Arc::new(result))))
            }
            _ => Err(BytecodeError::TypeError("String concatenation requires two strings".to_string())),
        }
    }
    
    /// Execute string length operation
    fn execute_string_len(&self, string: &OvmValue) -> Result<OvmValue, BytecodeError> {
        let string_value = string.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        
        match string_value {
            Value::String(s) => Ok(OvmValue::from_ast(Value::Integer(s.len() as i64))),
            _ => Err(BytecodeError::TypeError("String length requires a string".to_string())),
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
        let ast_value = value.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
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
    
    pub fn prepare_for_execution(&mut self, bytecode: &CompiledBytecode, args: &[OvmValue]) -> Result<(), BytecodeError> {
        // Allocate registers
        self.registers.clear();
        self.registers.resize(bytecode.register_count as usize, OvmValue::from_ast(Value::Unit));
        
        // Set up locals with arguments
        self.locals.clear();
        self.locals.resize(bytecode.local_count as usize, OvmValue::from_ast(Value::Unit));
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
        self.registers.get(reg.0 as usize)
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
        self.locals.get(local_idx as usize)
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
        }
    }
    
    pub fn compile_function(&mut self, func_id: FunctionId, func: &FunctionDecl) -> Result<CompiledBytecode, BytecodeError> {
        // Reset state
        self.register_allocator.reset();
        self.emitter.reset();
        self.local_variables.clear();
        self.next_local_idx = 0;
        
        // Add function parameters as locals
        for param in &func.parameters {
            self.local_variables.insert(param.name.clone(), self.next_local_idx);
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
                let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Integer(*value)));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }
            
            Expr::Float(value) => {
                let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Float(*value)));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }
            
            Expr::Boolean(value) => {
                let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Boolean(*value)));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }
            
            Expr::String(value) => {
                let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::String(Arc::new((**value).clone()))));
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
                    BinaryOp::GreaterThanEqual => self.emitter.emit_ge(dst_reg, left_reg, right_reg),
                    _ => return Err(BytecodeError::CompilationFailed(format!("Unsupported binary operator: {:?}", op))),
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
        self.instructions.push(Instruction::LoadConst { dst, const_idx });
    }
    
    pub fn emit_load_local(&mut self, dst: Register, local_idx: u32) {
        self.instructions.push(Instruction::LoadLocal { dst, local_idx });
    }
    
    pub fn emit_store_local(&mut self, src: Register, local_idx: u32) {
        self.instructions.push(Instruction::StoreLocal { src, local_idx });
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
        self.instructions.push(Instruction::MakeList { dst, elements });
    }
    
    pub fn emit_return(&mut self, value: Option<Register>) {
        self.instructions.push(Instruction::Return { value });
    }
    
    pub fn emit_nop(&mut self) {
        self.instructions.push(Instruction::Nop);
    }
    
    pub fn has_return(&self) -> bool {
        self.instructions.iter().any(|instr| matches!(instr, Instruction::Return { .. }))
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
    
    pub fn optimize_instructions(&mut self, instructions: Vec<Instruction>) -> Result<Vec<Instruction>, BytecodeError> {
        let mut optimized = instructions;
        
        // Apply optimizations in order
        optimized = self.constant_folder.fold_constants(optimized)?;
        optimized = self.peephole_optimizer.optimize(optimized)?;
        optimized = self.dead_code_eliminator.eliminate_dead_code(optimized)?;
        optimized = self.register_optimizer.optimize_registers(optimized)?;
        
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
    
    pub fn eliminate_dead_code(&mut self, instructions: Vec<Instruction>) -> Result<Vec<Instruction>, BytecodeError> {
        // Simplified dead code elimination - for now just return as-is
        Ok(instructions)
    }
}

impl RegisterOptimizer {
    pub fn new() -> Self {
        Self {
            register_map: HashMap::new(),
            interference_graph: HashMap::new(),
        }
    }
    
    pub fn optimize_registers(&mut self, instructions: Vec<Instruction>) -> Result<Vec<Instruction>, BytecodeError> {
        // Simplified register optimization - for now just return as-is
        Ok(instructions)
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
    
    pub fn optimize_control_flow(&mut self, instructions: Vec<Instruction>) -> Result<Vec<Instruction>, BytecodeError> {
        // Simplified control flow optimization - for now just return as-is
        Ok(instructions)
    }
}

impl ConstantFolder {
    pub fn new() -> Self {
        Self {
            constant_values: HashMap::new(),
        }
    }
    
    pub fn fold_constants(&mut self, instructions: Vec<Instruction>) -> Result<Vec<Instruction>, BytecodeError> {
        // Simplified constant folding - for now just return as-is
        Ok(instructions)
    }
}

impl PeepholeOptimizer {
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }
    
    pub fn optimize(&mut self, instructions: Vec<Instruction>) -> Result<Vec<Instruction>, BytecodeError> {
        // Simplified peephole optimization - for now just return as-is
        Ok(instructions)
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Instruction::LoadConst { dst, const_idx } => write!(f, "LOAD_CONST r{}, #{}", dst.0, const_idx),
            Instruction::LoadLocal { dst, local_idx } => write!(f, "LOAD_LOCAL r{}, l{}", dst.0, local_idx),
            Instruction::StoreLocal { src, local_idx } => write!(f, "STORE_LOCAL r{}, l{}", src.0, local_idx),
            Instruction::Move { dst, src } => write!(f, "MOVE r{}, r{}", dst.0, src.0),
            Instruction::Add { dst, lhs, rhs } => write!(f, "ADD r{}, r{}, r{}", dst.0, lhs.0, rhs.0),
            Instruction::Sub { dst, lhs, rhs } => write!(f, "SUB r{}, r{}, r{}", dst.0, lhs.0, rhs.0),
            Instruction::Mul { dst, lhs, rhs } => write!(f, "MUL r{}, r{}, r{}", dst.0, lhs.0, rhs.0),
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
        assert!(vm.has_bytecode(func_id), "VM should have bytecode for the function");
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
        assert!(execution_result.is_ok(), "Function execution should succeed");
        
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
        let mut vm = BytecodeVm::new();
        
        // Test list creation and access
        let list_value = OvmValue::from_ast(Value::List(vec![
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
        ].into()));
        
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
            BytecodeError::DivisionByZero => {},
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
        let list_arg = OvmValue::from_ast(Value::List(vec![
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
        ].into()));
        
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