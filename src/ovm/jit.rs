//! The baseline JIT: hot bytecode compiled to native machine code.
//!
//! This is the third tier, and it extends the correctness story unchanged:
//! interpreter → bytecode ("can't compile identically → stay interpreted")
//! → native ("can't compile natively → stay on bytecode"). A function
//! prequalifies at promotion time when every instruction falls in a pure
//! numeric/boolean whitelist — arithmetic, comparisons, branches,
//! self-recursion, return. The actual compilation is **type-specialized
//! and lazy**: it happens on the first call, using the argument kinds the
//! call actually carries (Int/Float per parameter), and the compiled
//! entry guards on exactly that signature — any other argument shape
//! runs on bytecode as before. One specialization per function.
//!
//! Pure is the load-bearing word: a qualifying function has no side
//! effects, so *any* guard failure (argument-kind mismatch, integer
//! overflow, division by zero, depth exhaustion) simply abandons the
//! native run and re-executes the same call on bytecode, which produces
//! the exact result or error the VM would have produced anyway. The JIT
//! never reproduces an error message; it only ever declines.
//!
//! Codegen notes:
//! - Registers are typed by inference: i64 (integers, booleans as 0/1)
//!   or f64. Mixed int/float arithmetic promotes the integer side with
//!   fcvt_from_sint, exactly as the VM's execute_binary_op does.
//! - Checked integer arithmetic is hand-rolled flag math; float add/sub/
//!   mul are plain IEEE (as in the VM), while float division guards
//!   b == 0.0 because olang errors there rather than producing inf.
//! - A register written with conflicting kinds is fine as long as nothing
//!   reads it (an `if` statement's dead result slot): its stores are
//!   skipped, but operation *guards* still run, because the VM would
//!   still error on an overflowing dead computation.
//! - Self-recursion is a direct native call carrying a depth budget; when
//!   it reaches zero the whole call deopts, and the bytecode re-run hits
//!   the VM's own max-call-depth error if the recursion really is runaway.
//! - Each internal function returns (value, status); status != 0 deopts
//!   the caller too, unwinding the native stack to the entry wrapper.

use crate::ast::BinaryOp;
use crate::ovm::bytecode::{CompiledBytecode, Instruction};
use crate::ovm::value::{OvmValue, ValueData};
use crate::ovm::FunctionId;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags, Type, Value as ClifValue};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

/// The C-ABI entry: (args pointer, depth budget, out pointer) -> status.
/// Status 0 means `*out` holds the result (raw bits; the caller knows the
/// kind); anything else means deopt. Float arguments and results travel
/// as their IEEE bit patterns in the i64 slots.
type NativeEntry = unsafe extern "C" fn(*const i64, i64, *mut i64) -> i64;

const STATUS_OK: i64 = 0;
const MAX_PARAMS: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Int,
    Bool,
    Float,
}

impl Kind {
    fn clif_type(self) -> Type {
        match self {
            Kind::Float => types::F64,
            _ => types::I64,
        }
    }
}

struct JittedFn {
    entry: NativeEntry,
    param_kinds: Vec<Kind>,
    ret_kind: Kind,
}

enum Slot {
    /// Whitelist passed at registration; compiles on first call with the
    /// argument kinds that call observes.
    Pending,
    /// Never native — failed the whitelist, inference, or codegen.
    Refused,
    /// One compiled specialization; the entry guard is an exact match on
    /// its param_kinds.
    Ready(JittedFn),
}

/// Per-VM JIT state. The module owns the executable memory; function
/// pointers stay valid until the module (and thus the VM) drops.
pub struct JitCache {
    module: Option<JITModule>,
    table: Vec<Option<Slot>>,
    pub compiled: u64,
    pub native_calls: u64,
}

// SAFETY: the module's executable memory is immutable after
// finalize_definitions; mutation only happens through &mut self on the
// owning (single-threaded) VM. Function pointers are plain code addresses.
unsafe impl Send for JitCache {}

impl std::fmt::Debug for JitCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JitCache")
            .field("compiled", &self.compiled)
            .field("native_calls", &self.native_calls)
            .finish()
    }
}

impl Default for JitCache {
    fn default() -> Self {
        Self::new()
    }
}

fn jit_debug() -> bool {
    std::env::var_os("OLANG_JIT_DEBUG").is_some()
}

impl JitCache {
    pub fn new() -> Self {
        Self {
            module: None,
            table: Vec::new(),
            compiled: 0,
            native_calls: 0,
        }
    }

    fn module(&mut self) -> Option<&mut JITModule> {
        if self.module.is_none() {
            // Speed over compile time; the functions are tiny.
            let builder = JITBuilder::with_flags(
                &[("opt_level", "speed")],
                cranelift_module::default_libcall_names(),
            )
            .ok()?;
            self.module = Some(JITModule::new(builder));
        }
        self.module.as_mut()
    }

    /// Called at bytecode registration: run the cheap syntactic whitelist
    /// so functions that can never compile pay nothing per call later.
    pub fn try_compile(&mut self, func_id: FunctionId, bytecode: &CompiledBytecode) {
        let idx = func_id.index();
        if self.table.len() <= idx {
            self.table.resize_with(idx + 1, || None);
        }
        if self.table[idx].is_some() {
            return;
        }
        if whitelist_ok(func_id, bytecode) {
            self.table[idx] = Some(Slot::Pending);
        } else {
            if jit_debug() {
                eprintln!(
                    "[jit] fn#{} does not qualify; instructions: {:?}",
                    idx,
                    bytecode
                        .instructions
                        .iter()
                        .map(instruction_name)
                        .collect::<Vec<_>>()
                );
            }
            self.table[idx] = Some(Slot::Refused);
        }
    }

    /// True when a native run is possible or still decidable — callers
    /// use it to decide whether extracting argument values is worth it.
    #[inline]
    pub fn has(&self, func_id: FunctionId) -> bool {
        matches!(
            self.table.get(func_id.index()),
            Some(Some(Slot::Pending)) | Some(Some(Slot::Ready(_)))
        )
    }

    /// Run the native body if the argument kinds fit (compiling the
    /// specialization on first use). None means "no native run happened"
    /// — the caller proceeds on bytecode exactly as before.
    #[inline]
    pub fn try_call(
        &mut self,
        func_id: FunctionId,
        bytecode: &CompiledBytecode,
        args: &[OvmValue],
        remaining_depth: u32,
    ) -> Option<OvmValue> {
        let mut bits = [0i64; MAX_PARAMS];
        let mut kinds = [Kind::Int; MAX_PARAMS];
        if args.len() > MAX_PARAMS {
            return None;
        }
        for (i, arg) in args.iter().enumerate() {
            match arg.data {
                ValueData::Integer(v) => {
                    bits[i] = v;
                    kinds[i] = Kind::Int;
                }
                ValueData::Float(f) => {
                    bits[i] = f.to_bits() as i64;
                    kinds[i] = Kind::Float;
                }
                _ => return None,
            }
        }
        self.try_call_raw(
            func_id,
            bytecode,
            &bits[..args.len()],
            &kinds[..args.len()],
            remaining_depth,
        )
    }

    /// Like try_call, but the caller already extracted raw bits and kinds.
    pub fn try_call_raw(
        &mut self,
        func_id: FunctionId,
        bytecode: &CompiledBytecode,
        bits: &[i64],
        kinds: &[Kind],
        remaining_depth: u32,
    ) -> Option<OvmValue> {
        let idx = func_id.index();
        match self.table.get(idx)? {
            Some(Slot::Ready(_)) => {}
            Some(Slot::Pending) => {
                // First call: specialize on the kinds this call carries.
                let slot = match self.specialize(func_id, bytecode, kinds) {
                    Some(jitted) => {
                        if jit_debug() {
                            eprintln!("[jit] fn#{} compiled to native for {:?}", idx, kinds);
                        }
                        self.compiled += 1;
                        Slot::Ready(jitted)
                    }
                    None => {
                        if jit_debug() {
                            eprintln!("[jit] fn#{} refused (inference or codegen)", idx);
                        }
                        Slot::Refused
                    }
                };
                self.table[idx] = Some(slot);
            }
            _ => return None,
        }

        let Some(Slot::Ready(jitted)) = self.table.get(idx)?.as_ref() else {
            return None;
        };
        if kinds != jitted.param_kinds.as_slice() {
            return None;
        }
        let mut out = 0i64;
        let status =
            unsafe { (jitted.entry)(bits.as_ptr(), remaining_depth as i64, &mut out as *mut i64) };
        if status != STATUS_OK {
            return None;
        }
        self.native_calls += 1;
        Some(match jitted.ret_kind {
            Kind::Int => OvmValue::new_integer(out),
            Kind::Bool => OvmValue::new_boolean(out != 0),
            Kind::Float => OvmValue::new_float(f64::from_bits(out as u64)),
        })
    }

    fn specialize(
        &mut self,
        func_id: FunctionId,
        bytecode: &CompiledBytecode,
        param_kinds: &[Kind],
    ) -> Option<JittedFn> {
        let debug = jit_debug();
        let inference = match infer_kinds(func_id, bytecode, param_kinds) {
            Some(inf) => inf,
            None => {
                if debug {
                    eprintln!(
                        "[jit] fn#{} kind inference failed for {:?}",
                        func_id.index(),
                        param_kinds
                    );
                }
                return None;
            }
        };
        let ret_kind = inference.ret_kind;
        let module = self.module()?;

        // Internal function: (typed params..., depth i64) -> (value, status),
        // cranelift's own fast calling convention so recursion stays in
        // registers.
        let mut inner_sig = module.make_signature();
        for k in param_kinds {
            inner_sig.params.push(AbiParam::new(k.clif_type()));
        }
        inner_sig.params.push(AbiParam::new(types::I64));
        inner_sig.returns.push(AbiParam::new(ret_kind.clif_type()));
        inner_sig.returns.push(AbiParam::new(types::I64));

        let inner_name = format!("olang_jit_{}", func_id.index());
        let inner_id = module
            .declare_function(&inner_name, Linkage::Local, &inner_sig)
            .ok()?;

        let mut ctx = module.make_context();
        ctx.func.signature = inner_sig.clone();
        let mut fbc = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            translate_body(&mut builder, module, inner_id, bytecode, &inference)?;
            builder.finalize();
        }
        if let Err(e) = module.define_function(inner_id, &mut ctx) {
            if debug {
                eprintln!("[jit] define inner failed: {:?}\nIR:\n{}", e, ctx.func);
            }
            return None;
        }
        module.clear_context(&mut ctx);

        // Entry wrapper: C ABI (args_ptr, depth, out_ptr) -> status. Float
        // params/results are read and written straight from the i64 slots
        // as f64 — same bytes, no conversion.
        let mut entry_sig = module.make_signature();
        entry_sig.params.push(AbiParam::new(types::I64));
        entry_sig.params.push(AbiParam::new(types::I64));
        entry_sig.params.push(AbiParam::new(types::I64));
        entry_sig.returns.push(AbiParam::new(types::I64));

        let entry_name = format!("olang_jit_{}_entry", func_id.index());
        let entry_id = module
            .declare_function(&entry_name, Linkage::Export, &entry_sig)
            .ok()?;

        let mut ctx = module.make_context();
        ctx.func.signature = entry_sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let block = builder.create_block();
            builder.append_block_params_for_function_params(block);
            builder.switch_to_block(block);
            let args_ptr = builder.block_params(block)[0];
            let depth = builder.block_params(block)[1];
            let out_ptr = builder.block_params(block)[2];

            let mut call_args = Vec::with_capacity(param_kinds.len() + 1);
            for (i, k) in param_kinds.iter().enumerate() {
                call_args.push(builder.ins().load(
                    k.clif_type(),
                    MemFlags::trusted(),
                    args_ptr,
                    (i * 8) as i32,
                ));
            }
            call_args.push(depth);

            let inner_ref = module.declare_func_in_func(inner_id, builder.func);
            let call = builder.ins().call(inner_ref, &call_args);
            let value = builder.inst_results(call)[0];
            let status = builder.inst_results(call)[1];
            builder
                .ins()
                .store(MemFlags::trusted(), value, out_ptr, 0i32);
            builder.ins().return_(&[status]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        module.define_function(entry_id, &mut ctx).ok()?;
        module.clear_context(&mut ctx);
        module.finalize_definitions().ok()?;

        let code = module.get_finalized_function(entry_id);
        // SAFETY: the signature matches entry_sig exactly and the memory
        // lives as long as the module (owned by this cache).
        let entry: NativeEntry = unsafe { std::mem::transmute(code) };
        Some(JittedFn {
            entry,
            param_kinds: param_kinds.to_vec(),
            ret_kind,
        })
    }
}

// ── prequalification: the syntactic whitelist ──────────────────────────

/// Every instruction the JIT knows how to translate, checked without any
/// type information — cheap enough to run at registration for every
/// promoted function.
fn whitelist_ok(self_id: FunctionId, bytecode: &CompiledBytecode) -> bool {
    if bytecode.param_count > MAX_PARAMS
        || bytecode.instructions.is_empty()
        || bytecode.entry_point >= bytecode.instructions.len()
    {
        return false;
    }
    bytecode.instructions.iter().all(|inst| match inst {
        Instruction::LoadConst { const_idx, .. } => matches!(
            bytecode.constants.get(*const_idx as usize).map(|c| &c.data),
            Some(
                ValueData::Integer(_)
                    | ValueData::Boolean(_)
                    | ValueData::Unit
                    | ValueData::Float(_)
            )
        ),
        Instruction::Move { .. }
        | Instruction::Add { .. }
        | Instruction::Sub { .. }
        | Instruction::Mul { .. }
        | Instruction::Div { .. }
        | Instruction::Mod { .. }
        | Instruction::Neg { .. }
        | Instruction::Eq { .. }
        | Instruction::Ne { .. }
        | Instruction::Lt { .. }
        | Instruction::Le { .. }
        | Instruction::Gt { .. }
        | Instruction::Ge { .. }
        | Instruction::And { .. }
        | Instruction::Or { .. }
        | Instruction::Not { .. }
        | Instruction::Jump { .. }
        | Instruction::JumpIfTrue { .. }
        | Instruction::JumpIfFalse { .. }
        | Instruction::MatchFail => true,
        Instruction::BinImm { imm, .. } => {
            matches!(imm.data, ValueData::Integer(_) | ValueData::Float(_))
        }
        Instruction::CallFn { func_id, args, .. } => {
            *func_id == self_id && args.len() == bytecode.param_count
        }
        Instruction::Return { value } => value.is_some(),
        _ => false,
    })
}

// ── qualification: register-kind inference ─────────────────────────────

// Kinds as a bitmask. A register may be WRITTEN with several kinds over
// its lifetime (e.g. the dead result slot of an `if` statement that is
// Unit on one path and Int on the other). What must be single-kinded is
// every register an instruction READS — reads carry *allowed-set*
// constraints, writes accumulate possibilities, and qualification
// demands each read register's write-set be a singleton inside its
// allowed set.
const K_INT: u8 = 1;
const K_BOOL: u8 = 2;
const K_UNIT: u8 = 4;
const K_FLOAT: u8 = 8;
const K_NUM: u8 = K_INT | K_FLOAT;
const K_ANY: u8 = K_INT | K_BOOL | K_UNIT | K_FLOAT;

fn kind_mask(k: Kind) -> u8 {
    match k {
        Kind::Int => K_INT,
        Kind::Bool => K_BOOL,
        Kind::Float => K_FLOAT,
    }
}

struct Inference {
    /// Singleton kind per register, None for dead (never-read) registers
    /// whose writes are skipped in codegen.
    reg_kind: Vec<Option<Kind>>,
    param_kinds: Vec<Kind>,
    ret_kind: Kind,
}

/// Prove the function kind-sound for this parameter specialization.
/// None = stay on bytecode (for every future call too — one shot).
fn infer_kinds(
    _self_id: FunctionId,
    bytecode: &CompiledBytecode,
    param_kinds: &[Kind],
) -> Option<Inference> {
    if param_kinds.len() != bytecode.param_count {
        return None;
    }
    let nregs = bytecode.register_count as usize;
    let mut writes: Vec<u8> = vec![0; nregs];
    // Allowed-kind constraint per register; starts fully permissive and
    // narrows at each read. K_ANY == "never read".
    let mut allowed: Vec<u8> = vec![K_ANY; nregs];
    let mut was_read: Vec<bool> = vec![false; nregs];
    let mut eq_pairs: Vec<(u32, u32)> = Vec::new();
    let mut return_regs: Vec<u32> = Vec::new();
    let mut ret_mask: u8 = 0;

    for (i, k) in param_kinds.iter().enumerate() {
        writes[i] = kind_mask(*k);
    }

    let const_mask = |idx: u32| -> u8 {
        match bytecode.constants.get(idx as usize).map(|c| &c.data) {
            Some(ValueData::Integer(_)) => K_INT,
            Some(ValueData::Boolean(_)) => K_BOOL,
            Some(ValueData::Unit) => K_UNIT,
            Some(ValueData::Float(_)) => K_FLOAT,
            _ => 0,
        }
    };

    // Fixpoint over monotone growth (writes gain bits, allowed loses
    // bits, both bounded); terminates.
    loop {
        let mut changed = false;
        let grow = |slot: &mut u8, bits: u8, changed: &mut bool| {
            if *slot | bits != *slot {
                *slot |= bits;
                *changed = true;
            }
        };
        let narrow =
            |allowed: &mut [u8], was_read: &mut [bool], r: u32, mask: u8, changed: &mut bool| {
                let slot = &mut allowed[r as usize];
                if *slot & mask != *slot {
                    *slot &= mask;
                    *changed = true;
                }
                if !was_read[r as usize] {
                    was_read[r as usize] = true;
                    *changed = true;
                }
            };

        for inst in &bytecode.instructions {
            match inst {
                Instruction::LoadConst { dst, const_idx } => {
                    grow(
                        &mut writes[dst.0 as usize],
                        const_mask(*const_idx),
                        &mut changed,
                    );
                }
                Instruction::Move { dst, src } => {
                    let src_mask = writes[src.0 as usize];
                    grow(&mut writes[dst.0 as usize], src_mask, &mut changed);
                    // Liveness flows backwards through copies: the move
                    // reads src only if someone reads dst, and the copied
                    // value must satisfy dst's constraint.
                    if was_read[dst.0 as usize] {
                        let dst_allowed = allowed[dst.0 as usize];
                        narrow(
                            &mut allowed,
                            &mut was_read,
                            src.0,
                            dst_allowed,
                            &mut changed,
                        );
                    }
                }
                Instruction::Add { dst, lhs, rhs }
                | Instruction::Sub { dst, lhs, rhs }
                | Instruction::Mul { dst, lhs, rhs }
                | Instruction::Div { dst, lhs, rhs }
                | Instruction::Mod { dst, lhs, rhs } => {
                    narrow(&mut allowed, &mut was_read, lhs.0, K_NUM, &mut changed);
                    narrow(&mut allowed, &mut was_read, rhs.0, K_NUM, &mut changed);
                    let l = writes[lhs.0 as usize];
                    let r = writes[rhs.0 as usize];
                    // Result: float if either side can be float; int only
                    // when both sides can be int.
                    if (l | r) & K_FLOAT != 0 {
                        grow(&mut writes[dst.0 as usize], K_FLOAT, &mut changed);
                    }
                    if l & K_INT != 0 && r & K_INT != 0 {
                        grow(&mut writes[dst.0 as usize], K_INT, &mut changed);
                    }
                }
                Instruction::Neg { dst, src } => {
                    narrow(&mut allowed, &mut was_read, src.0, K_NUM, &mut changed);
                    let s = writes[src.0 as usize];
                    grow(&mut writes[dst.0 as usize], s & K_NUM, &mut changed);
                }
                Instruction::Eq { dst, lhs, rhs } | Instruction::Ne { dst, lhs, rhs } => {
                    // Equality: both numeric (mixed promotes) or both bool.
                    narrow(
                        &mut allowed,
                        &mut was_read,
                        lhs.0,
                        K_NUM | K_BOOL,
                        &mut changed,
                    );
                    narrow(
                        &mut allowed,
                        &mut was_read,
                        rhs.0,
                        K_NUM | K_BOOL,
                        &mut changed,
                    );
                    eq_pairs.push((lhs.0, rhs.0));
                    grow(&mut writes[dst.0 as usize], K_BOOL, &mut changed);
                }
                Instruction::Lt { dst, lhs, rhs }
                | Instruction::Le { dst, lhs, rhs }
                | Instruction::Gt { dst, lhs, rhs }
                | Instruction::Ge { dst, lhs, rhs } => {
                    narrow(&mut allowed, &mut was_read, lhs.0, K_NUM, &mut changed);
                    narrow(&mut allowed, &mut was_read, rhs.0, K_NUM, &mut changed);
                    grow(&mut writes[dst.0 as usize], K_BOOL, &mut changed);
                }
                Instruction::And { dst, lhs, rhs } | Instruction::Or { dst, lhs, rhs } => {
                    narrow(&mut allowed, &mut was_read, lhs.0, K_BOOL, &mut changed);
                    narrow(&mut allowed, &mut was_read, rhs.0, K_BOOL, &mut changed);
                    grow(&mut writes[dst.0 as usize], K_BOOL, &mut changed);
                }
                Instruction::Not { dst, src } => {
                    narrow(&mut allowed, &mut was_read, src.0, K_BOOL, &mut changed);
                    grow(&mut writes[dst.0 as usize], K_BOOL, &mut changed);
                }
                Instruction::BinImm { op, dst, lhs, imm } => {
                    let imm_mask = match imm.data {
                        ValueData::Integer(_) => K_INT,
                        ValueData::Float(_) => K_FLOAT,
                        _ => return None,
                    };
                    match op {
                        BinaryOp::Add
                        | BinaryOp::Subtract
                        | BinaryOp::Multiply
                        | BinaryOp::Divide
                        | BinaryOp::Modulo => {
                            narrow(&mut allowed, &mut was_read, lhs.0, K_NUM, &mut changed);
                            let l = writes[lhs.0 as usize];
                            if (l | imm_mask) & K_FLOAT != 0 {
                                grow(&mut writes[dst.0 as usize], K_FLOAT, &mut changed);
                            }
                            if l & K_INT != 0 && imm_mask == K_INT {
                                grow(&mut writes[dst.0 as usize], K_INT, &mut changed);
                            }
                        }
                        BinaryOp::Equal
                        | BinaryOp::NotEqual
                        | BinaryOp::LessThan
                        | BinaryOp::LessThanEqual
                        | BinaryOp::GreaterThan
                        | BinaryOp::GreaterThanEqual => {
                            narrow(&mut allowed, &mut was_read, lhs.0, K_NUM, &mut changed);
                            grow(&mut writes[dst.0 as usize], K_BOOL, &mut changed);
                        }
                        _ => return None,
                    }
                }
                Instruction::Jump { .. } | Instruction::MatchFail => {}
                Instruction::JumpIfTrue { condition, .. }
                | Instruction::JumpIfFalse { condition, .. } => {
                    narrow(
                        &mut allowed,
                        &mut was_read,
                        condition.0,
                        K_BOOL,
                        &mut changed,
                    );
                }
                Instruction::CallFn { dst, args, .. } => {
                    // whitelist_ok proved func_id == self and arity.
                    for (i, a) in args.iter().enumerate() {
                        narrow(
                            &mut allowed,
                            &mut was_read,
                            a.0,
                            kind_mask(param_kinds[i]),
                            &mut changed,
                        );
                    }
                    grow(&mut writes[dst.0 as usize], ret_mask, &mut changed);
                }
                Instruction::Return { value } => {
                    let reg = (*value)?;
                    narrow(
                        &mut allowed,
                        &mut was_read,
                        reg.0,
                        K_NUM | K_BOOL,
                        &mut changed,
                    );
                    return_regs.push(reg.0);
                    grow(&mut ret_mask, writes[reg.0 as usize], &mut changed);
                }
                _ => return None,
            }
        }
        if !changed {
            break;
        }
        eq_pairs.clear();
        return_regs.clear();
    }

    let singleton = |mask: u8| -> Option<Kind> {
        match mask {
            K_INT => Some(Kind::Int),
            K_BOOL => Some(Kind::Bool),
            K_FLOAT => Some(Kind::Float),
            _ => None,
        }
    };

    // Every read register: writes must be a singleton within the allowed
    // set. Unread registers are dead — codegen skips their stores.
    let mut reg_kind: Vec<Option<Kind>> = vec![None; nregs];
    for r in 0..nregs {
        if was_read[r] {
            let Some(k) = singleton(writes[r]) else {
                if jit_debug() {
                    eprintln!(
                        "[jit]   r{} read but writes mask {:#b} not singleton (allowed {:#b})",
                        r, writes[r], allowed[r]
                    );
                }
                return None;
            };
            if kind_mask(k) & allowed[r] == 0 {
                if jit_debug() {
                    eprintln!(
                        "[jit]   r{} kind {:?} outside allowed {:#b}",
                        r, k, allowed[r]
                    );
                }
                return None;
            }
            reg_kind[r] = Some(k);
        }
    }

    // Equality pairs: both numeric (any mix) or both bool.
    for (l, r) in eq_pairs {
        let lk = reg_kind[l as usize]?;
        let rk = reg_kind[r as usize]?;
        let both_num =
            matches!(lk, Kind::Int | Kind::Float) && matches!(rk, Kind::Int | Kind::Float);
        let both_bool = lk == Kind::Bool && rk == Kind::Bool;
        if !both_num && !both_bool {
            return None;
        }
    }

    // All returns agree on one kind.
    let ret_kind = singleton(ret_mask)?;
    for r in &return_regs {
        if reg_kind[*r as usize] != Some(ret_kind) {
            return None;
        }
    }

    Some(Inference {
        reg_kind,
        param_kinds: param_kinds.to_vec(),
        ret_kind,
    })
}

fn instruction_name(inst: &Instruction) -> &'static str {
    match inst {
        Instruction::LoadConst { .. } => "LoadConst",
        Instruction::LoadLocal { .. } => "LoadLocal",
        Instruction::StoreLocal { .. } => "StoreLocal",
        Instruction::Move { .. } => "Move",
        Instruction::Add { .. } => "Add",
        Instruction::Sub { .. } => "Sub",
        Instruction::Mul { .. } => "Mul",
        Instruction::Div { .. } => "Div",
        Instruction::Mod { .. } => "Mod",
        Instruction::Neg { .. } => "Neg",
        Instruction::Eq { .. } => "Eq",
        Instruction::Ne { .. } => "Ne",
        Instruction::Lt { .. } => "Lt",
        Instruction::Le { .. } => "Le",
        Instruction::Gt { .. } => "Gt",
        Instruction::Ge { .. } => "Ge",
        Instruction::And { .. } => "And",
        Instruction::Or { .. } => "Or",
        Instruction::Not { .. } => "Not",
        Instruction::Jump { .. } => "Jump",
        Instruction::JumpIfTrue { .. } => "JumpIfTrue",
        Instruction::JumpIfFalse { .. } => "JumpIfFalse",
        Instruction::Call { .. } => "Call",
        Instruction::CallFn { .. } => "CallFn",
        Instruction::CallValue { .. } => "CallValue",
        Instruction::CallBuiltin { .. } => "CallBuiltin",
        Instruction::CallNamed { .. } => "CallNamed",
        Instruction::CallMethod { .. } => "CallMethod",
        Instruction::BinImm { .. } => "BinImm",
        Instruction::Return { .. } => "Return",
        Instruction::MatchFail => "MatchFail",
        _ => "other",
    }
}

// ── codegen ────────────────────────────────────────────────────────────

struct Gen<'a> {
    inference: &'a Inference,
    deopt_block: cranelift_codegen::ir::Block,
}

impl Gen<'_> {
    fn kind(&self, reg: u32) -> Option<Kind> {
        self.inference.reg_kind.get(reg as usize).copied().flatten()
    }

    /// use_var for a live register; None if the register is dead (the
    /// caller decides whether that makes the instruction skippable).
    fn read(&self, builder: &mut FunctionBuilder, reg: u32) -> Option<ClifValue> {
        self.kind(reg)?;
        Some(builder.use_var(Variable::from_u32(reg)))
    }

    /// def_var unless the register is dead.
    fn write(&self, builder: &mut FunctionBuilder, reg: u32, value: ClifValue) {
        if self.kind(reg).is_some() {
            builder.def_var(Variable::from_u32(reg), value);
        }
    }

    /// Promote a value to f64 if its register kind is Int.
    fn to_float(&self, builder: &mut FunctionBuilder, value: ClifValue, k: Kind) -> ClifValue {
        match k {
            Kind::Float => value,
            _ => builder.ins().fcvt_from_sint(types::F64, value),
        }
    }
}

fn translate_body(
    builder: &mut FunctionBuilder,
    module: &mut JITModule,
    self_id: cranelift_module::FuncId,
    bytecode: &CompiledBytecode,
    inference: &Inference,
) -> Option<()> {
    let n = bytecode.instructions.len();
    let param_count = inference.param_kinds.len();

    // Block leaders: entry, every jump target, every instruction after a
    // conditional jump (the fallthrough edge needs a block).
    let mut is_leader = vec![false; n];
    is_leader[bytecode.entry_point] = true;
    is_leader[0] = true;
    for (i, inst) in bytecode.instructions.iter().enumerate() {
        match inst {
            Instruction::Jump { target } => {
                let t = target.0 as usize;
                if t >= n {
                    return None;
                }
                is_leader[t] = true;
            }
            Instruction::JumpIfTrue { target, .. } | Instruction::JumpIfFalse { target, .. } => {
                let t = target.0 as usize;
                if t >= n {
                    return None;
                }
                is_leader[t] = true;
                if i + 1 < n {
                    is_leader[i + 1] = true;
                }
            }
            _ => {}
        }
    }

    let entry_block = builder.create_block();
    builder.append_block_params_for_function_params(entry_block);
    let deopt_block = builder.create_block();
    let gen = Gen {
        inference,
        deopt_block,
    };

    let mut blocks = vec![None; n];
    for (i, leader) in is_leader.iter().enumerate() {
        if *leader {
            blocks[i] = Some(builder.create_block());
        }
    }

    // One typed variable per LIVE register, plus the depth budget.
    let nregs = bytecode.register_count as usize;
    for r in 0..nregs {
        if let Some(k) = gen.kind(r as u32) {
            builder.declare_var(Variable::from_u32(r as u32), k.clif_type());
        }
    }
    let depth_var = Variable::from_u32(nregs as u32);
    builder.declare_var(depth_var, types::I64);

    builder.switch_to_block(entry_block);
    let params: Vec<ClifValue> = builder.block_params(entry_block).to_vec();
    for (i, p) in params.iter().take(param_count).enumerate() {
        gen.write(builder, i as u32, *p);
    }
    // Initialize every other live register so use before first def can't
    // trip the SSA builder (bytecode never actually reads uninitialized
    // registers, but proving that is the verifier's job, not ours).
    for r in param_count..nregs {
        if let Some(k) = gen.kind(r as u32) {
            let zero = match k {
                Kind::Float => builder.ins().f64const(0.0),
                _ => builder.ins().iconst(types::I64, 0),
            };
            builder.def_var(Variable::from_u32(r as u32), zero);
        }
    }
    builder.def_var(depth_var, params[param_count]);
    let first = blocks[bytecode.entry_point]
        .or(blocks[0])
        .expect("entry leader");
    builder.ins().jump(first, &[]);

    // The entry block just terminated; the loop's leader handling switches
    // into the first real block without adding a second jump.
    let mut terminated = true;

    for (i, inst) in bytecode.instructions.iter().enumerate() {
        if let Some(block) = blocks[i] {
            if !terminated {
                builder.ins().jump(block, &[]);
            }
            builder.switch_to_block(block);
            terminated = false;
        }
        if terminated {
            // Dead code between a terminator and the next leader.
            continue;
        }

        match inst {
            Instruction::LoadConst { dst, const_idx } => {
                let Some(dk) = gen.kind(dst.0) else { continue };
                let val = match (&bytecode.constants[*const_idx as usize].data, dk) {
                    (ValueData::Integer(x), Kind::Int) => builder.ins().iconst(types::I64, *x),
                    (ValueData::Boolean(b), Kind::Bool) => {
                        builder.ins().iconst(types::I64, *b as i64)
                    }
                    (ValueData::Float(f), Kind::Float) => builder.ins().f64const(*f),
                    _ => return None,
                };
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::MatchFail => {
                // Unreachable on real paths; if control ever got here the
                // deopt re-run raises the canonical match-failure error.
                builder.ins().jump(deopt_block, &[]);
                terminated = true;
            }
            Instruction::Move { dst, src } => {
                if gen.kind(dst.0).is_none() {
                    continue; // dead store, no observable effect
                }
                let val = gen.read(builder, src.0)?;
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::Add { dst, lhs, rhs }
            | Instruction::Sub { dst, lhs, rhs }
            | Instruction::Mul { dst, lhs, rhs }
            | Instruction::Div { dst, lhs, rhs }
            | Instruction::Mod { dst, lhs, rhs } => {
                let lk = gen.kind(lhs.0)?;
                let rk = gen.kind(rhs.0)?;
                let a = builder.use_var(Variable::from_u32(lhs.0));
                let b = builder.use_var(Variable::from_u32(rhs.0));
                let op = arith_op(inst);
                let val = emit_arith(builder, &gen, op, a, lk, b, rk)?;
                gen.write(builder, dst.0, val);
            }
            Instruction::Neg { dst, src } => {
                let sk = gen.kind(src.0)?;
                let a = builder.use_var(Variable::from_u32(src.0));
                let val = match sk {
                    Kind::Float => builder.ins().fneg(a),
                    Kind::Int => {
                        // checked_neg: only i64::MIN overflows.
                        let min = builder.ins().iconst(types::I64, i64::MIN);
                        let is_min = builder.ins().icmp(IntCC::Equal, a, min);
                        let cont = builder.create_block();
                        builder.ins().brif(is_min, deopt_block, &[], cont, &[]);
                        builder.switch_to_block(cont);
                        builder.ins().ineg(a)
                    }
                    Kind::Bool => return None,
                };
                gen.write(builder, dst.0, val);
            }
            Instruction::Eq { dst, lhs, rhs }
            | Instruction::Ne { dst, lhs, rhs }
            | Instruction::Lt { dst, lhs, rhs }
            | Instruction::Le { dst, lhs, rhs }
            | Instruction::Gt { dst, lhs, rhs }
            | Instruction::Ge { dst, lhs, rhs } => {
                let lk = gen.kind(lhs.0)?;
                let rk = gen.kind(rhs.0)?;
                let a = builder.use_var(Variable::from_u32(lhs.0));
                let b = builder.use_var(Variable::from_u32(rhs.0));
                let flag = if lk == Kind::Float || rk == Kind::Float {
                    let fa = gen.to_float(builder, a, lk);
                    let fb = gen.to_float(builder, b, rk);
                    builder.ins().fcmp(float_cc(inst), fa, fb)
                } else {
                    builder.ins().icmp(compare_cc(inst), a, b)
                };
                let val = builder.ins().uextend(types::I64, flag);
                gen.write(builder, dst.0, val);
            }
            Instruction::And { dst, lhs, rhs } => {
                let a = gen.read(builder, lhs.0)?;
                let b = gen.read(builder, rhs.0)?;
                let val = builder.ins().band(a, b);
                gen.write(builder, dst.0, val);
            }
            Instruction::Or { dst, lhs, rhs } => {
                let a = gen.read(builder, lhs.0)?;
                let b = gen.read(builder, rhs.0)?;
                let val = builder.ins().bor(a, b);
                gen.write(builder, dst.0, val);
            }
            Instruction::Not { dst, src } => {
                let a = gen.read(builder, src.0)?;
                let one = builder.ins().iconst(types::I64, 1);
                let val = builder.ins().bxor(a, one);
                gen.write(builder, dst.0, val);
            }
            Instruction::BinImm { op, dst, lhs, imm } => {
                let lk = gen.kind(lhs.0)?;
                let a = builder.use_var(Variable::from_u32(lhs.0));
                let (b, bk) = match imm.data {
                    ValueData::Integer(x) => {
                        if lk == Kind::Float {
                            // The VM promotes the int immediate to float.
                            (builder.ins().f64const(x as f64), Kind::Float)
                        } else {
                            (builder.ins().iconst(types::I64, x), Kind::Int)
                        }
                    }
                    ValueData::Float(f) => (builder.ins().f64const(f), Kind::Float),
                    _ => return None,
                };
                match op {
                    BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Multiply
                    | BinaryOp::Divide
                    | BinaryOp::Modulo => {
                        let val = emit_arith(builder, &gen, imm_arith(op)?, a, lk, b, bk)?;
                        gen.write(builder, dst.0, val);
                    }
                    BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::LessThan
                    | BinaryOp::LessThanEqual
                    | BinaryOp::GreaterThan
                    | BinaryOp::GreaterThanEqual => {
                        let flag = if lk == Kind::Float || bk == Kind::Float {
                            let fa = gen.to_float(builder, a, lk);
                            let fb = gen.to_float(builder, b, bk);
                            builder.ins().fcmp(imm_float_cc(op)?, fa, fb)
                        } else {
                            builder.ins().icmp(imm_compare(op)?, a, b)
                        };
                        let val = builder.ins().uextend(types::I64, flag);
                        gen.write(builder, dst.0, val);
                    }
                    _ => return None,
                }
            }
            Instruction::Jump { target } => {
                let block = blocks[target.0 as usize]?;
                builder.ins().jump(block, &[]);
                terminated = true;
            }
            Instruction::JumpIfTrue { condition, target } => {
                let cond = gen.read(builder, condition.0)?;
                let then_block = blocks[target.0 as usize]?;
                let else_block = blocks.get(i + 1).copied().flatten()?;
                builder.ins().brif(cond, then_block, &[], else_block, &[]);
                terminated = true;
            }
            Instruction::JumpIfFalse { condition, target } => {
                let cond = gen.read(builder, condition.0)?;
                let else_block = blocks[target.0 as usize]?;
                let then_block = blocks.get(i + 1).copied().flatten()?;
                builder.ins().brif(cond, then_block, &[], else_block, &[]);
                terminated = true;
            }
            Instruction::CallFn { dst, args, .. } => {
                // Self-recursion (whitelist guaranteed func_id == self):
                // spend a unit of depth budget, deopt when exhausted.
                let depth = builder.use_var(depth_var);
                let one = builder.ins().iconst(types::I64, 1);
                let new_depth = builder.ins().isub(depth, one);
                let exhausted = builder
                    .ins()
                    .icmp_imm(IntCC::SignedLessThanOrEqual, new_depth, 0);
                let cont = builder.create_block();
                builder.ins().brif(exhausted, deopt_block, &[], cont, &[]);
                builder.switch_to_block(cont);

                let mut call_args = Vec::with_capacity(args.len() + 1);
                for a in args {
                    call_args.push(gen.read(builder, a.0)?);
                }
                call_args.push(new_depth);
                let self_ref = module.declare_func_in_func(self_id, builder.func);
                let call = builder.ins().call(self_ref, &call_args);
                let value = builder.inst_results(call)[0];
                let status = builder.inst_results(call)[1];

                // A deopt anywhere below unwinds the whole native call.
                let ok_block = builder.create_block();
                builder.ins().brif(status, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                gen.write(builder, dst.0, value);
            }
            Instruction::Return { value } => {
                let reg = (*value)?;
                let val = gen.read(builder, reg.0)?;
                let ok = builder.ins().iconst(types::I64, STATUS_OK);
                builder.ins().return_(&[val, ok]);
                terminated = true;
            }
            _ => return None,
        }
    }

    if !terminated {
        // Bytecode always ends in Return; be safe anyway.
        builder.ins().jump(deopt_block, &[]);
    }

    builder.switch_to_block(deopt_block);
    let zero = match inference.ret_kind {
        Kind::Float => builder.ins().f64const(0.0),
        _ => builder.ins().iconst(types::I64, 0),
    };
    let one = builder.ins().iconst(types::I64, 1);
    builder.ins().return_(&[zero, one]);

    builder.seal_all_blocks();
    Some(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

fn arith_op(inst: &Instruction) -> Arith {
    match inst {
        Instruction::Add { .. } => Arith::Add,
        Instruction::Sub { .. } => Arith::Sub,
        Instruction::Mul { .. } => Arith::Mul,
        Instruction::Div { .. } => Arith::Div,
        Instruction::Mod { .. } => Arith::Mod,
        _ => unreachable!("arith_op on non-arith instruction"),
    }
}

fn imm_arith(op: &BinaryOp) -> Option<Arith> {
    Some(match op {
        BinaryOp::Add => Arith::Add,
        BinaryOp::Subtract => Arith::Sub,
        BinaryOp::Multiply => Arith::Mul,
        BinaryOp::Divide => Arith::Div,
        BinaryOp::Modulo => Arith::Mod,
        _ => return None,
    })
}

fn compare_cc(inst: &Instruction) -> IntCC {
    match inst {
        Instruction::Eq { .. } => IntCC::Equal,
        Instruction::Ne { .. } => IntCC::NotEqual,
        Instruction::Lt { .. } => IntCC::SignedLessThan,
        Instruction::Le { .. } => IntCC::SignedLessThanOrEqual,
        Instruction::Gt { .. } => IntCC::SignedGreaterThan,
        Instruction::Ge { .. } => IntCC::SignedGreaterThanOrEqual,
        _ => unreachable!("compare_cc on non-compare instruction"),
    }
}

fn float_cc(inst: &Instruction) -> FloatCC {
    match inst {
        Instruction::Eq { .. } => FloatCC::Equal,
        Instruction::Ne { .. } => FloatCC::NotEqual,
        Instruction::Lt { .. } => FloatCC::LessThan,
        Instruction::Le { .. } => FloatCC::LessThanOrEqual,
        Instruction::Gt { .. } => FloatCC::GreaterThan,
        Instruction::Ge { .. } => FloatCC::GreaterThanOrEqual,
        _ => unreachable!("float_cc on non-compare instruction"),
    }
}

fn imm_compare(op: &BinaryOp) -> Option<IntCC> {
    Some(match op {
        BinaryOp::Equal => IntCC::Equal,
        BinaryOp::NotEqual => IntCC::NotEqual,
        BinaryOp::LessThan => IntCC::SignedLessThan,
        BinaryOp::LessThanEqual => IntCC::SignedLessThanOrEqual,
        BinaryOp::GreaterThan => IntCC::SignedGreaterThan,
        BinaryOp::GreaterThanEqual => IntCC::SignedGreaterThanOrEqual,
        _ => return None,
    })
}

fn imm_float_cc(op: &BinaryOp) -> Option<FloatCC> {
    Some(match op {
        BinaryOp::Equal => FloatCC::Equal,
        BinaryOp::NotEqual => FloatCC::NotEqual,
        BinaryOp::LessThan => FloatCC::LessThan,
        BinaryOp::LessThanEqual => FloatCC::LessThanOrEqual,
        BinaryOp::GreaterThan => FloatCC::GreaterThan,
        BinaryOp::GreaterThanEqual => FloatCC::GreaterThanOrEqual,
        _ => return None,
    })
}

/// Arithmetic matching the VM's semantics exactly. Integer paths carry
/// overflow/zero guards that branch to deopt; float add/sub/mul are plain
/// IEEE; float division and modulo guard b == 0.0 (olang errors there).
/// Float modulo itself is refused — Rust's `%` is fmod, which has no
/// exact IR equivalent, and guessing is how divergence starts.
fn emit_arith(
    builder: &mut FunctionBuilder,
    gen: &Gen,
    op: Arith,
    a: ClifValue,
    ak: Kind,
    b: ClifValue,
    bk: Kind,
) -> Option<ClifValue> {
    if ak == Kind::Bool || bk == Kind::Bool {
        return None;
    }
    let float = ak == Kind::Float || bk == Kind::Float;
    if float {
        if op == Arith::Mod {
            return None;
        }
        let fa = gen.to_float(builder, a, ak);
        let fb = gen.to_float(builder, b, bk);
        let val = match op {
            Arith::Add => builder.ins().fadd(fa, fb),
            Arith::Sub => builder.ins().fsub(fa, fb),
            Arith::Mul => builder.ins().fmul(fa, fb),
            Arith::Div => {
                // olang: float division by zero is an error, not inf.
                let zero = builder.ins().f64const(0.0);
                let is_zero = builder.ins().fcmp(FloatCC::Equal, fb, zero);
                let cont = builder.create_block();
                builder.ins().brif(is_zero, gen.deopt_block, &[], cont, &[]);
                builder.switch_to_block(cont);
                builder.ins().fdiv(fa, fb)
            }
            Arith::Mod => unreachable!(),
        };
        return Some(val);
    }

    let deopt = gen.deopt_block;
    let val = match op {
        Arith::Add => {
            let r = builder.ins().iadd(a, b);
            // signed overflow iff sign(a)==sign(b) and sign(r)!=sign(a):
            // ((a ^ r) & (b ^ r)) < 0
            let ax = builder.ins().bxor(a, r);
            let bx = builder.ins().bxor(b, r);
            let both = builder.ins().band(ax, bx);
            let overflow = builder.ins().icmp_imm(IntCC::SignedLessThan, both, 0);
            let cont = builder.create_block();
            builder.ins().brif(overflow, deopt, &[], cont, &[]);
            builder.switch_to_block(cont);
            r
        }
        Arith::Sub => {
            let r = builder.ins().isub(a, b);
            // overflow iff sign(a)!=sign(b) and sign(r)!=sign(a):
            // ((a ^ b) & (a ^ r)) < 0
            let ab = builder.ins().bxor(a, b);
            let ar = builder.ins().bxor(a, r);
            let both = builder.ins().band(ab, ar);
            let overflow = builder.ins().icmp_imm(IntCC::SignedLessThan, both, 0);
            let cont = builder.create_block();
            builder.ins().brif(overflow, deopt, &[], cont, &[]);
            builder.switch_to_block(cont);
            r
        }
        Arith::Mul => {
            let r = builder.ins().imul(a, b);
            // overflow iff the high 64 bits disagree with the sign
            // extension of the low 64: smulhi(a,b) != r >> 63
            let hi = builder.ins().smulhi(a, b);
            let sign = builder.ins().sshr_imm(r, 63);
            let overflow = builder.ins().icmp(IntCC::NotEqual, hi, sign);
            let cont = builder.create_block();
            builder.ins().brif(overflow, deopt, &[], cont, &[]);
            builder.switch_to_block(cont);
            r
        }
        Arith::Div | Arith::Mod => {
            // b == 0 and (a == i64::MIN && b == -1) both deopt (the VM's
            // zero-division and checked-overflow errors respectively).
            let zero_div = builder.ins().icmp_imm(IntCC::Equal, b, 0);
            let cont1 = builder.create_block();
            builder.ins().brif(zero_div, deopt, &[], cont1, &[]);
            builder.switch_to_block(cont1);

            let min = builder.ins().iconst(types::I64, i64::MIN);
            let a_min = builder.ins().icmp(IntCC::Equal, a, min);
            let b_neg1 = builder.ins().icmp_imm(IntCC::Equal, b, -1);
            let both = builder.ins().band(a_min, b_neg1);
            let cont2 = builder.create_block();
            builder.ins().brif(both, deopt, &[], cont2, &[]);
            builder.switch_to_block(cont2);

            match op {
                Arith::Div => builder.ins().sdiv(a, b),
                _ => builder.ins().srem(a, b),
            }
        }
    };
    Some(val)
}
