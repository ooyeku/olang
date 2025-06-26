//! OVM Bytecode Virtual Machine
//!
//! Register-based bytecode VM for intermediate-tier execution between interpreter and JIT

use crate::ast::{BinaryOp, Expr, FunctionDecl, Statement, Value};
use crate::ovm::{FunctionId, OvmValue};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// Register-based bytecode virtual machine``
pub struct BytecodeVm {
    // Bytecode compiler
    compiler: BytecodeCompiler,
    
    // Compiled bytecode cache
    bytecode_cache: Arc<RwLock<HashMap<FunctionId, CompiledBytecode>>>,
    
    // Runtime execution state
    execution_state: ExecutionState,
    
    // Performance statistics
    stats: VmStatistics,
}

/// Bytecode compiler that transforms AST to bytecode
pub struct BytecodeCompiler {
    // Register allocator
    register_allocator: RegisterAllocator,
    
    // Instruction emitter
    emitter: InstructionEmitter,
    
    // Optimization passes
    optimizer: BytecodeOptimizer,
}

/// Bytecode optimization engine
pub struct BytecodeOptimizer {
    // Dead code elimination
    dead_code_eliminator: DeadCodeEliminator,
    
    // Register reuse optimization
    register_optimizer: RegisterOptimizer,
    
    // Control flow optimization
    control_flow_optimizer: ControlFlowOptimizer,
}

/// Compiled bytecode representation
#[derive(Debug, Clone)]
pub struct CompiledBytecode {
    pub function_id: FunctionId,
    pub instructions: Vec<Instruction>,
    pub register_count: u32,
    pub constants: Vec<OvmValue>,
    pub debug_info: BytecodeDebugInfo,
    pub optimization_level: u8,
}

/// Bytecode instruction set
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
    Return { value: Option<Register> },
    
    // Collection operations
    MakeList { dst: Register, elements: Vec<Register> },
    ListGet { dst: Register, list: Register, index: Register },
    ListSet { list: Register, index: Register, value: Register },
    ListLen { dst: Register, list: Register },
    
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
    
    // Debug operations
    Nop,
    DebugPrint { src: Register },
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
}

/// Register allocator for bytecode generation
pub struct RegisterAllocator {
    next_register: u32,
    free_registers: Vec<Register>,
    register_usage: HashMap<Register, RegisterUsage>,
}

#[derive(Debug, Clone)]
struct RegisterUsage {
    first_use: usize,
    last_use: usize,
    is_temporary: bool,
}

/// Instruction emitter
pub struct InstructionEmitter {
    instructions: Vec<Instruction>,
    labels: HashMap<String, Label>,
    next_label: u32,
    constants: Vec<OvmValue>,
    constant_map: HashMap<String, u32>, // For deduplication
}

/// Bytecode optimization passes
pub struct DeadCodeEliminator {
    live_registers: std::collections::HashSet<Register>,
}

pub struct RegisterOptimizer {
    register_map: HashMap<Register, Register>,
}

pub struct ControlFlowOptimizer {
    basic_blocks: Vec<BasicBlock>,
}

#[derive(Debug, Clone)]
struct BasicBlock {
    instructions: Vec<Instruction>,
    predecessors: Vec<usize>,
    successors: Vec<usize>,
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
}

#[derive(Debug)]
pub enum VmException {
    RuntimeError(String),
    TypeError(String),
    StackOverflow,
    DivisionByZero,
    IndexOutOfBounds,
}

// Implementation
impl BytecodeVm {
    pub fn new() -> Self {
        Self {
            compiler: BytecodeCompiler::new(),
            bytecode_cache: Arc::new(RwLock::new(HashMap::new())),
            execution_state: ExecutionState::new(),
            stats: VmStatistics::default(),
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
        let start_time = std::time::Instant::now();
        
        // Get compiled bytecode
        let bytecode = {
            if let Ok(cache) = self.bytecode_cache.read() {
                if let Some(bytecode) = cache.get(&func_id) {
                    self.stats.bytecode_cache_hits += 1;
                    bytecode.clone()
                } else {
                    self.stats.bytecode_cache_misses += 1;
                    return Err(BytecodeError::FunctionNotFound(func_id));
                }
            } else {
                return Err(BytecodeError::RuntimeError("Cache lock failed".to_string()));
            }
        };
        
        // Set up execution state
        self.execution_state.prepare_for_execution(&bytecode, args)?;
        
        // Execute bytecode
        let result = self.execute_bytecode(&bytecode)?;
        
        self.stats.execution_time += start_time.elapsed();
        self.stats.function_calls += 1;
        
        Ok(result)
    }
    
    /// Execute bytecode instructions
    fn execute_bytecode(&mut self, bytecode: &CompiledBytecode) -> Result<OvmValue, BytecodeError> {
        let mut pc = 0;
        
        while pc < bytecode.instructions.len() {
            let instruction = &bytecode.instructions[pc];
            self.stats.instructions_executed += 1;
            
            match instruction {
                Instruction::LoadConst { dst, const_idx } => {
                    let value = bytecode.constants.get(*const_idx as usize)
                        .ok_or_else(|| BytecodeError::RuntimeError("Invalid constant index".to_string()))?;
                    self.execution_state.set_register(*dst, value.clone())?;
                }
                
                Instruction::Move { dst, src } => {
                    let value = self.execution_state.get_register(*src)?;
                    self.execution_state.set_register(*dst, value)?;
                }
                
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
                
                Instruction::Jump { target } => {
                    pc = target.0 as usize;
                    continue;
                }
                
                Instruction::Return { value } => {
                    if let Some(reg) = value {
                        return Ok(self.execution_state.get_register(*reg)?);
                    } else {
                        return Ok(OvmValue::from_ast(Value::Unit));
                    }
                }
                
                Instruction::Call { dst, function, args, .. } => {
                    // Simplified function call implementation
                    let _func_value = self.execution_state.get_register(*function)?;
                    let mut arg_values = Vec::new();
                    for arg_reg in args {
                        arg_values.push(self.execution_state.get_register(*arg_reg)?);
                    }
                    
                    // For now, return a placeholder value
                    // In a complete implementation, this would dispatch to the function
                    self.execution_state.set_register(*dst, OvmValue::from_ast(Value::Unit))?;
                }
                
                Instruction::MakeList { dst, elements } => {
                    let mut list_values = Vec::new();
                    for elem_reg in elements {
                        list_values.push(self.execution_state.get_register(*elem_reg)?.to_ast().map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?);
                    }
                    let list_value = Value::List(list_values.into());
                    self.execution_state.set_register(*dst, OvmValue::from_ast(list_value))?;
                }
                
                Instruction::Nop => {
                    // No operation
                }
                
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
        // Convert to AST values for existing operation logic
        let left_ast = left.to_ast().map_err(|e| BytecodeError::TypeError(format!("{:?}", e)))?;
        let right_ast = right.to_ast().map_err(|e| BytecodeError::TypeError(format!("{:?}", e)))?;
        
        let result_ast = match (left_ast, right_ast) {
            (Value::Integer(a), Value::Integer(b)) => {
                match op {
                    BinaryOp::Add => Value::Integer(a + b),
                    BinaryOp::Subtract => Value::Integer(a - b),
                    BinaryOp::Multiply => Value::Integer(a * b),
                    BinaryOp::Divide => {
                        if b == 0 {
                            return Err(BytecodeError::RuntimeError("Division by zero".to_string()));
                        }
                        Value::Integer(a / b)
                    }
                    BinaryOp::Modulo => {
                        if b == 0 {
                            return Err(BytecodeError::RuntimeError("Division by zero".to_string()));
                        }
                        Value::Integer(a % b)
                    }
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
                    BinaryOp::Divide => Value::Float(a / b),
                    BinaryOp::Equal => Value::Boolean((a - b).abs() < f64::EPSILON),
                    BinaryOp::NotEqual => Value::Boolean((a - b).abs() >= f64::EPSILON),
                    BinaryOp::LessThan => Value::Boolean(a < b),
                    BinaryOp::LessThanEqual => Value::Boolean(a <= b),
                    BinaryOp::GreaterThan => Value::Boolean(a > b),
                    BinaryOp::GreaterThanEqual => Value::Boolean(a >= b),
                    _ => return Err(BytecodeError::TypeError(format!("Unsupported operation: {:?}", op))),
                }
            }
            _ => return Err(BytecodeError::TypeError("Type mismatch in binary operation".to_string())),
        };
        
        Ok(OvmValue::from_ast(result_ast))
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

impl BytecodeCompiler {
    pub fn new() -> Self {
        Self {
            register_allocator: RegisterAllocator::new(),
            emitter: InstructionEmitter::new(),
            optimizer: BytecodeOptimizer::new(),
        }
    }
    
    /// Compile function to bytecode
    pub fn compile_function(&mut self, func_id: FunctionId, func: &FunctionDecl) -> Result<CompiledBytecode, BytecodeError> {
        // Reset state
        self.register_allocator.reset();
        self.emitter.reset();
        
        // Compile function body
        let _result_reg = self.compile_expression(&func.body)?;
        
        // Ensure function returns
        if !self.emitter.has_return() {
            self.emitter.emit_return(None);
        }
        
        // Apply optimizations
        let mut instructions = self.emitter.take_instructions();
        let constants = self.emitter.take_constants();
        
        instructions = self.optimizer.optimize_instructions(instructions)?;
        
        Ok(CompiledBytecode {
            function_id: func_id,
            instructions,
            register_count: self.register_allocator.max_register_used(),
            constants,
            debug_info: BytecodeDebugInfo {
                function_name: Some(func.name.clone()),
                ..Default::default()
            },
            optimization_level: 1,
        })
    }
    
    /// Compile list of statements
    fn compile_statement_list(&mut self, statements: &[Statement]) -> Result<(), BytecodeError> {
        for statement in statements {
            self.compile_statement(statement)?;
        }
        Ok(())
    }
    
    /// Compile single statement
    fn compile_statement(&mut self, statement: &Statement) -> Result<(), BytecodeError> {
        match statement {
            Statement::Expression(expr) => {
                let _result_reg = self.compile_expression(expr)?;
                Ok(())
            }
            Statement::LetDecl(decl) => {
                if let Some(value) = &decl.value {
                    let value_reg = self.compile_expression(value)?;
                    let local_idx = self.emitter.get_or_create_local(&decl.name);
                    self.emitter.emit_store_local(value_reg, local_idx);
                }
                Ok(())
            }
            Statement::FunctionDecl(_) | Statement::AsyncFunctionDecl(_) | 
            Statement::TypeDecl(_) | Statement::ErrorTypeDecl(_) |
            Statement::ImportDecl(_) | Statement::ExportDecl(_) => {
                // These statements don't generate runtime code
                self.emitter.emit_nop();
                Ok(())
            }
            _ => {
                // For other statement types, emit a placeholder
                self.emitter.emit_nop();
                Ok(())
            }
        }
    }
    
    /// Compile expression and return register containing result
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
                let local_idx = self.emitter.get_or_create_local(name);
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_local(dst_reg, local_idx);
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
            
            Expr::Assignment { name, value } => {
                let value_reg = self.compile_expression(value)?;
                let local_idx = self.emitter.get_or_create_local(name);
                self.emitter.emit_store_local(value_reg, local_idx);
                Ok(value_reg) // Return the assigned value
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
    
    pub fn prepare_for_execution(&mut self, bytecode: &CompiledBytecode, args: &[OvmValue]) -> Result<(), BytecodeError> {
        // Allocate registers
        self.registers.clear();
        self.registers.resize(bytecode.register_count as usize, OvmValue::from_ast(Value::Unit));
        
        // Set up locals with arguments
        self.locals.clear();
        self.locals.extend_from_slice(args);
        
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
}

impl RegisterAllocator {
    pub fn new() -> Self {
        Self {
            next_register: 0,
            free_registers: Vec::new(),
            register_usage: HashMap::new(),
        }
    }
    
    pub fn reset(&mut self) {
        self.next_register = 0;
        self.free_registers.clear();
        self.register_usage.clear();
    }
    
    pub fn allocate_register(&mut self) -> Register {
        if let Some(reg) = self.free_registers.pop() {
            reg
        } else {
            let reg = Register(self.next_register);
            self.next_register += 1;
            reg
        }
    }
    
    pub fn free_register(&mut self, reg: Register) {
        self.free_registers.push(reg);
    }
    
    pub fn max_register_used(&self) -> u32 {
        self.next_register
    }
}

impl InstructionEmitter {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            labels: HashMap::new(),
            next_label: 0,
            constants: Vec::new(),
            constant_map: HashMap::new(),
        }
    }
    
    pub fn reset(&mut self) {
        self.instructions.clear();
        self.labels.clear();
        self.next_label = 0;
        self.constants.clear();
        self.constant_map.clear();
    }
    
    pub fn add_constant(&mut self, value: OvmValue) -> u32 {
        // For simplicity, just add without deduplication
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
    
    pub fn get_or_create_local(&mut self, _name: &str) -> u32 {
        // Simplified: just return increasing indices
        // In a real implementation, this would maintain a mapping
        static mut NEXT_LOCAL: u32 = 0;
        unsafe {
            let idx = NEXT_LOCAL;
            NEXT_LOCAL += 1;
            idx
        }
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
        }
    }
    
    pub fn optimize_instructions(&mut self, instructions: Vec<Instruction>) -> Result<Vec<Instruction>, BytecodeError> {
        let mut optimized = instructions;
        
        // Apply dead code elimination
        optimized = self.dead_code_eliminator.eliminate_dead_code(optimized)?;
        
        // Apply register optimization
        optimized = self.register_optimizer.optimize_registers(optimized)?;
        
        // Apply control flow optimization
        optimized = self.control_flow_optimizer.optimize_control_flow(optimized)?;
        
        Ok(optimized)
    }
}

impl DeadCodeEliminator {
    pub fn new() -> Self {
        Self {
            live_registers: std::collections::HashSet::new(),
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
        }
    }
    
    pub fn optimize_control_flow(&mut self, instructions: Vec<Instruction>) -> Result<Vec<Instruction>, BytecodeError> {
        // Simplified control flow optimization - for now just return as-is
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