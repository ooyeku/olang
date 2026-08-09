//! The baseline JIT: hot bytecode compiled to native machine code.
//!
//! This is the third tier, and it extends the correctness story unchanged:
//! interpreter → bytecode ("can't compile identically → stay interpreted")
//! → native ("can't compile natively → stay on bytecode"). A function
//! qualifies only when every instruction falls in a pure integer/boolean
//! whitelist — arithmetic, comparisons, branches, self-recursion, return —
//! with operand kinds proven by a fixpoint inference over its registers.
//! Pure is the load-bearing word: a qualifying function has no side
//! effects, so *any* guard failure (non-integer argument, overflow,
//! division by zero, depth exhaustion) can simply abandon the native run
//! and re-execute the same call on bytecode, which produces the exact
//! result or error the VM would have produced anyway. The JIT never
//! reproduces an error message; it only ever declines.
//!
//! Codegen notes:
//! - Values are raw i64s; booleans are 0/1. The interpreter's checked
//!   arithmetic becomes native flag math (hand-rolled overflow checks, so
//!   no dependence on unstable IR forms): any trip jumps to a deopt block.
//! - Self-recursion is a direct native call carrying a depth budget; when
//!   it reaches zero the whole call deopts, and the bytecode re-run hits
//!   the VM's own max-call-depth error if the recursion really is runaway.
//! - Each internal function returns (value, status); status != 0 deopts
//!   the caller too, unwinding the native stack to the entry wrapper.

use crate::ast::BinaryOp;
use crate::ovm::bytecode::{CompiledBytecode, Instruction, TplPart};
use crate::ovm::value::{OvmValue, ValueData};
use crate::ovm::FunctionId;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags, Value as ClifValue};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

/// The C-ABI entry: (args pointer, depth budget, out pointer) -> status.
/// Status 0 means `*out` holds the result; anything else means deopt.
type NativeEntry = unsafe extern "C" fn(*const i64, i64, *mut i64) -> i64;

const STATUS_OK: i64 = 0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Int,
    Bool,
}

struct JittedFn {
    entry: NativeEntry,
    param_count: usize,
    ret_kind: Kind,
}

/// Per-VM JIT state. The module owns the executable memory; function
/// pointers stay valid until the module (and thus the VM) drops.
pub struct JitCache {
    module: Option<JITModule>,
    table: Vec<Option<JittedFn>>,
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

    /// Try to native-compile freshly registered bytecode. Failure of any
    /// kind is absorbed: the function simply stays on bytecode.
    pub fn try_compile(&mut self, func_id: FunctionId, bytecode: &CompiledBytecode) {
        let idx = func_id.index();
        if self.table.len() <= idx {
            self.table.resize_with(idx + 1, || None);
        }
        if self.table[idx].is_some() {
            return;
        }
        let debug = std::env::var_os("OLANG_JIT_DEBUG").is_some();
        let Some(kinds) = infer_kinds(func_id, bytecode) else {
            if debug {
                eprintln!(
                    "[jit] fn#{} does not qualify; instructions: {:?}",
                    idx,
                    bytecode
                        .instructions
                        .iter()
                        .map(instruction_name)
                        .collect::<Vec<_>>()
                );
                if bytecode.instructions.len() <= 48 {
                    for (i, inst) in bytecode.instructions.iter().enumerate() {
                        eprintln!("[jit]   {:3}: {:?}", i, inst);
                    }
                }
            }
            return;
        };
        let Some(jitted) = self.compile(func_id, bytecode, &kinds) else {
            if debug {
                eprintln!("[jit] fn#{} qualified but codegen failed", idx);
            }
            return;
        };
        if debug {
            eprintln!("[jit] fn#{} compiled to native", idx);
        }
        self.table[idx] = Some(jitted);
        self.compiled += 1;
    }

    /// True when a native body exists for this function — callers use it
    /// to decide whether extracting argument values is worth it.
    #[inline]
    pub fn has(&self, func_id: FunctionId) -> bool {
        self.table
            .get(func_id.index())
            .is_some_and(|slot| slot.is_some())
    }

    /// Run the native body if one exists and every argument is an Integer.
    /// None means "no native run happened" — the caller proceeds on
    /// bytecode exactly as before. `remaining_depth` bounds native
    /// self-recursion at the VM's own limit.
    #[inline]
    pub fn try_call(
        &mut self,
        func_id: FunctionId,
        args: &[OvmValue],
        remaining_depth: u32,
    ) -> Option<OvmValue> {
        let mut raw = [0i64; 16];
        if args.len() > raw.len() {
            return None;
        }
        for (slot, arg) in raw.iter_mut().zip(args) {
            match arg.data {
                ValueData::Integer(i) => *slot = i,
                _ => return None,
            }
        }
        self.try_call_ints(func_id, &raw[..args.len()], remaining_depth)
    }

    /// Like try_call, but the caller already extracted raw integers.
    #[inline]
    pub fn try_call_ints(
        &mut self,
        func_id: FunctionId,
        ints: &[i64],
        remaining_depth: u32,
    ) -> Option<OvmValue> {
        let jitted = self.table.get(func_id.index())?.as_ref()?;
        if ints.len() != jitted.param_count {
            return None;
        }
        let mut out = 0i64;
        let status =
            unsafe { (jitted.entry)(ints.as_ptr(), remaining_depth as i64, &mut out as *mut i64) };
        if status != STATUS_OK {
            return None;
        }
        self.native_calls += 1;
        Some(match jitted.ret_kind {
            Kind::Int => OvmValue::new_integer(out),
            Kind::Bool => OvmValue::new_boolean(out != 0),
        })
    }

    fn compile(
        &mut self,
        func_id: FunctionId,
        bytecode: &CompiledBytecode,
        kinds: &Inference,
    ) -> Option<JittedFn> {
        let debug = std::env::var_os("OLANG_JIT_DEBUG").is_some();
        macro_rules! step {
            ($e:expr, $what:literal) => {
                match $e {
                    Some(v) => v,
                    None => {
                        if debug {
                            eprintln!("[jit] codegen step failed: {}", $what);
                        }
                        return None;
                    }
                }
            };
        }

        let param_count = bytecode.param_count;
        let ret_kind = step!(kinds.ret_kind, "ret_kind");
        let module = step!(self.module(), "module creation");

        // Internal function: (params..., depth) -> (value, status), using
        // cranelift's own fast calling convention so self-recursion stays
        // in registers.
        let mut inner_sig = module.make_signature();
        for _ in 0..param_count + 1 {
            inner_sig.params.push(AbiParam::new(types::I64));
        }
        inner_sig.returns.push(AbiParam::new(types::I64));
        inner_sig.returns.push(AbiParam::new(types::I64));

        let inner_name = format!("olang_jit_{}", func_id.index());
        let inner_id = step!(
            module
                .declare_function(&inner_name, Linkage::Local, &inner_sig)
                .ok(),
            "declare inner"
        );

        let mut ctx = module.make_context();
        ctx.func.signature = inner_sig.clone();
        let mut fbc = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            step!(
                translate_body(&mut builder, module, inner_id, bytecode, param_count),
                "translate body"
            );
            builder.finalize();
        }
        if let Err(e) = module.define_function(inner_id, &mut ctx) {
            if debug {
                eprintln!("[jit] define inner failed: {:?}\nIR:\n{}", e, ctx.func);
            }
            return None;
        }
        module.clear_context(&mut ctx);

        // Entry wrapper: C ABI (args_ptr, depth, out_ptr) -> status.
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

            let mut call_args = Vec::with_capacity(param_count + 1);
            for i in 0..param_count {
                call_args.push(builder.ins().load(
                    types::I64,
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
            param_count,
            ret_kind,
        })
    }
}

// ── qualification: register-kind inference ─────────────────────────────

// Kinds as a bitmask: a register may be WRITTEN with several kinds over
// its lifetime (e.g. the dead result slot of an `if` statement that is
// Unit on one path and Int on the other). What must be single-kinded is
// every register an instruction READS — reads carry requirements, writes
// accumulate possibilities, and qualification demands they line up.
const K_INT: u8 = 1;
const K_BOOL: u8 = 2;
const K_UNIT: u8 = 4;

struct Inference {
    ret_kind: Option<Kind>,
}

/// Prove the function whitelisted and kind-sound. None = stay on bytecode.
fn infer_kinds(self_id: FunctionId, bytecode: &CompiledBytecode) -> Option<Inference> {
    if bytecode.param_count > 16 || bytecode.instructions.is_empty() {
        return None;
    }

    let nregs = bytecode.register_count as usize;
    let mut writes: Vec<u8> = vec![0; nregs];
    let mut reads: Vec<u8> = vec![0; nregs];
    let mut eq_pairs: Vec<(u32, u32)> = Vec::new();
    let mut return_regs: Vec<u32> = Vec::new();
    let mut ret_mask: u8 = 0;

    for i in 0..bytecode.param_count {
        writes[i] = K_INT;
    }

    let const_kind = |idx: u32| -> Option<u8> {
        match bytecode.constants.get(idx as usize).map(|c| &c.data) {
            Some(ValueData::Integer(_)) => Some(K_INT),
            Some(ValueData::Boolean(_)) => Some(K_BOOL),
            Some(ValueData::Unit) => Some(K_UNIT),
            _ => None,
        }
    };

    // Fixpoint over monotone bitmask growth (Move propagation, self-call
    // return kinds); terminates because masks only ever gain bits.
    loop {
        let mut changed = false;
        let grow = |slot: &mut u8, bits: u8, changed: &mut bool| {
            if *slot | bits != *slot {
                *slot |= bits;
                *changed = true;
            }
        };

        for inst in &bytecode.instructions {
            match inst {
                Instruction::LoadConst { dst, const_idx } => {
                    let k = const_kind(*const_idx)?;
                    grow(&mut writes[dst.0 as usize], k, &mut changed);
                }
                Instruction::Move { dst, src } => {
                    let src_mask = writes[src.0 as usize];
                    grow(&mut writes[dst.0 as usize], src_mask, &mut changed);
                }
                Instruction::Add { dst, lhs, rhs }
                | Instruction::Sub { dst, lhs, rhs }
                | Instruction::Mul { dst, lhs, rhs }
                | Instruction::Div { dst, lhs, rhs }
                | Instruction::Mod { dst, lhs, rhs } => {
                    grow(&mut reads[lhs.0 as usize], K_INT, &mut changed);
                    grow(&mut reads[rhs.0 as usize], K_INT, &mut changed);
                    grow(&mut writes[dst.0 as usize], K_INT, &mut changed);
                }
                Instruction::Neg { dst, src } => {
                    grow(&mut reads[src.0 as usize], K_INT, &mut changed);
                    grow(&mut writes[dst.0 as usize], K_INT, &mut changed);
                }
                Instruction::Eq { dst, lhs, rhs } | Instruction::Ne { dst, lhs, rhs } => {
                    eq_pairs.push((lhs.0, rhs.0));
                    grow(&mut writes[dst.0 as usize], K_BOOL, &mut changed);
                }
                Instruction::Lt { dst, lhs, rhs }
                | Instruction::Le { dst, lhs, rhs }
                | Instruction::Gt { dst, lhs, rhs }
                | Instruction::Ge { dst, lhs, rhs } => {
                    grow(&mut reads[lhs.0 as usize], K_INT, &mut changed);
                    grow(&mut reads[rhs.0 as usize], K_INT, &mut changed);
                    grow(&mut writes[dst.0 as usize], K_BOOL, &mut changed);
                }
                Instruction::And { dst, lhs, rhs } | Instruction::Or { dst, lhs, rhs } => {
                    grow(&mut reads[lhs.0 as usize], K_BOOL, &mut changed);
                    grow(&mut reads[rhs.0 as usize], K_BOOL, &mut changed);
                    grow(&mut writes[dst.0 as usize], K_BOOL, &mut changed);
                }
                Instruction::Not { dst, src } => {
                    grow(&mut reads[src.0 as usize], K_BOOL, &mut changed);
                    grow(&mut writes[dst.0 as usize], K_BOOL, &mut changed);
                }
                Instruction::BinImm { op, dst, lhs, imm } => {
                    if !matches!(imm.data, ValueData::Integer(_)) {
                        return None;
                    }
                    let dst_kind = match op {
                        BinaryOp::Add
                        | BinaryOp::Subtract
                        | BinaryOp::Multiply
                        | BinaryOp::Divide
                        | BinaryOp::Modulo => K_INT,
                        BinaryOp::Equal
                        | BinaryOp::NotEqual
                        | BinaryOp::LessThan
                        | BinaryOp::LessThanEqual
                        | BinaryOp::GreaterThan
                        | BinaryOp::GreaterThanEqual => K_BOOL,
                        _ => return None,
                    };
                    grow(&mut reads[lhs.0 as usize], K_INT, &mut changed);
                    grow(&mut writes[dst.0 as usize], dst_kind, &mut changed);
                }
                Instruction::Jump { .. } | Instruction::MatchFail => {}
                Instruction::JumpIfTrue { condition, .. }
                | Instruction::JumpIfFalse { condition, .. } => {
                    grow(&mut reads[condition.0 as usize], K_BOOL, &mut changed);
                }
                Instruction::CallFn { dst, func_id, args } => {
                    if *func_id != self_id || args.len() != bytecode.param_count {
                        return None;
                    }
                    for a in args {
                        grow(&mut reads[a.0 as usize], K_INT, &mut changed);
                    }
                    grow(&mut writes[dst.0 as usize], ret_mask, &mut changed);
                }
                Instruction::Return { value } => {
                    let reg = (*value)?;
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

    let singleton =
        |mask: u8| -> Option<u8> { matches!(mask, K_INT | K_BOOL | K_UNIT).then_some(mask) };

    // Every read register must have been written with exactly the kind
    // the reads require.
    for r in 0..nregs {
        if reads[r] != 0 {
            let req = singleton(reads[r])?;
            if writes[r] != req {
                return None;
            }
        }
    }

    // Eq/Ne: both operands the same single kind, Int or Bool.
    for (l, r) in eq_pairs {
        let lk = singleton(writes[l as usize])?;
        let rk = singleton(writes[r as usize])?;
        if lk != rk || lk == K_UNIT {
            return None;
        }
    }

    // Every return site yields the same single kind, Int or Bool.
    let ret_kind = match singleton(ret_mask)? {
        K_INT => Kind::Int,
        K_BOOL => Kind::Bool,
        _ => return None,
    };
    for r in &return_regs {
        if writes[*r as usize] != ret_mask {
            return None;
        }
    }

    Some(Inference {
        ret_kind: Some(ret_kind),
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

fn translate_body(
    builder: &mut FunctionBuilder,
    module: &mut JITModule,
    self_id: cranelift_module::FuncId,
    bytecode: &CompiledBytecode,
    param_count: usize,
) -> Option<()> {
    let n = bytecode.instructions.len();
    if bytecode.entry_point >= n {
        return None;
    }

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

    let mut blocks = vec![None; n];
    for (i, leader) in is_leader.iter().enumerate() {
        if *leader {
            blocks[i] = Some(builder.create_block());
        }
    }

    // One i64 variable per register, plus the depth budget.
    let nregs = bytecode.register_count as usize;
    for r in 0..nregs + 1 {
        builder.declare_var(Variable::from_u32(r as u32), types::I64);
    }
    let depth_var = Variable::from_u32(nregs as u32);

    builder.switch_to_block(entry_block);
    let params: Vec<ClifValue> = builder.block_params(entry_block).to_vec();
    for (i, p) in params.iter().take(param_count).enumerate() {
        builder.def_var(Variable::from_u32(i as u32), *p);
    }
    let zero = builder.ins().iconst(types::I64, 0);
    for r in param_count..nregs {
        builder.def_var(Variable::from_u32(r as u32), zero);
    }
    builder.def_var(depth_var, params[param_count]);
    let first = blocks[bytecode.entry_point]
        .or(blocks[0])
        .expect("entry leader");
    builder.ins().jump(first, &[]);

    // The entry block just terminated; the loop's leader handling switches
    // into the first real block without adding a second jump.
    let mut terminated = true;
    let mut current_reachable = true;

    for (i, inst) in bytecode.instructions.iter().enumerate() {
        if let Some(block) = blocks[i] {
            if !terminated {
                builder.ins().jump(block, &[]);
            }
            builder.switch_to_block(block);
            terminated = false;
            current_reachable = true;
        }
        if terminated || !current_reachable {
            // Dead code between a terminator and the next leader.
            continue;
        }

        let v = |b: &mut FunctionBuilder, r: u32| b.use_var(Variable::from_u32(r));

        match inst {
            Instruction::LoadConst { dst, const_idx } => {
                let c = match bytecode.constants[*const_idx as usize].data {
                    ValueData::Integer(x) => x,
                    ValueData::Boolean(b) => b as i64,
                    // Unit-kind registers are never operated on or returned
                    // (inference guarantees it); any value stands in.
                    ValueData::Unit => 0,
                    _ => return None,
                };
                let val = builder.ins().iconst(types::I64, c);
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::MatchFail => {
                // Unreachable on real paths; if control ever got here the
                // deopt re-run raises the canonical match-failure error.
                builder.ins().jump(deopt_block, &[]);
                terminated = true;
            }
            Instruction::Move { dst, src } => {
                let val = v(builder, src.0);
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::Add { dst, lhs, rhs }
            | Instruction::Sub { dst, lhs, rhs }
            | Instruction::Mul { dst, lhs, rhs }
            | Instruction::Div { dst, lhs, rhs }
            | Instruction::Mod { dst, lhs, rhs } => {
                let a = v(builder, lhs.0);
                let b = v(builder, rhs.0);
                let op = arith_op(inst);
                let val = emit_checked_arith(builder, op, a, b, deopt_block)?;
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::Neg { dst, src } => {
                let a = v(builder, src.0);
                // checked_neg: only i64::MIN overflows.
                let min = builder.ins().iconst(types::I64, i64::MIN);
                let is_min = builder.ins().icmp(IntCC::Equal, a, min);
                let cont = builder.create_block();
                builder.ins().brif(is_min, deopt_block, &[], cont, &[]);
                builder.switch_to_block(cont);
                let val = builder.ins().ineg(a);
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::Eq { dst, lhs, rhs }
            | Instruction::Ne { dst, lhs, rhs }
            | Instruction::Lt { dst, lhs, rhs }
            | Instruction::Le { dst, lhs, rhs }
            | Instruction::Gt { dst, lhs, rhs }
            | Instruction::Ge { dst, lhs, rhs } => {
                let a = v(builder, lhs.0);
                let b = v(builder, rhs.0);
                let cc = compare_cc(inst);
                let flag = builder.ins().icmp(cc, a, b);
                let val = builder.ins().uextend(types::I64, flag);
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::And { dst, lhs, rhs } => {
                let a = v(builder, lhs.0);
                let b = v(builder, rhs.0);
                let val = builder.ins().band(a, b);
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::Or { dst, lhs, rhs } => {
                let a = v(builder, lhs.0);
                let b = v(builder, rhs.0);
                let val = builder.ins().bor(a, b);
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::Not { dst, src } => {
                let a = v(builder, src.0);
                let one = builder.ins().iconst(types::I64, 1);
                let val = builder.ins().bxor(a, one);
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::BinImm { op, dst, lhs, imm } => {
                let a = v(builder, lhs.0);
                let imm_val = match imm.data {
                    ValueData::Integer(x) => x,
                    _ => return None,
                };
                let b = builder.ins().iconst(types::I64, imm_val);
                match op {
                    BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply => {
                        let val = emit_checked_arith(builder, imm_arith(op)?, a, b, deopt_block)?;
                        builder.def_var(Variable::from_u32(dst.0), val);
                    }
                    BinaryOp::Divide | BinaryOp::Modulo => {
                        let val = emit_checked_arith(builder, imm_arith(op)?, a, b, deopt_block)?;
                        builder.def_var(Variable::from_u32(dst.0), val);
                    }
                    BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::LessThan
                    | BinaryOp::LessThanEqual
                    | BinaryOp::GreaterThan
                    | BinaryOp::GreaterThanEqual => {
                        let cc = imm_compare(op)?;
                        let flag = builder.ins().icmp(cc, a, b);
                        let val = builder.ins().uextend(types::I64, flag);
                        builder.def_var(Variable::from_u32(dst.0), val);
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
                let cond = v(builder, condition.0);
                let then_block = blocks[target.0 as usize]?;
                let else_block = blocks.get(i + 1).copied().flatten()?;
                builder.ins().brif(cond, then_block, &[], else_block, &[]);
                terminated = true;
            }
            Instruction::JumpIfFalse { condition, target } => {
                let cond = v(builder, condition.0);
                let else_block = blocks[target.0 as usize]?;
                let then_block = blocks.get(i + 1).copied().flatten()?;
                builder.ins().brif(cond, then_block, &[], else_block, &[]);
                terminated = true;
            }
            Instruction::CallFn { dst, args, .. } => {
                // Self-recursion (inference guaranteed func_id == self):
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

                let mut call_args: Vec<ClifValue> = args.iter().map(|r| v(builder, r.0)).collect();
                call_args.push(new_depth);
                let self_ref = module.declare_func_in_func(self_id, builder.func);
                let call = builder.ins().call(self_ref, &call_args);
                let value = builder.inst_results(call)[0];
                let status = builder.inst_results(call)[1];

                // A deopt anywhere below unwinds the whole native call.
                let ok_block = builder.create_block();
                builder.ins().brif(status, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                builder.def_var(Variable::from_u32(dst.0), value);
            }
            Instruction::Return { value } => {
                let reg = (*value)?;
                let val = v(builder, reg.0);
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
    let zero = builder.ins().iconst(types::I64, 0);
    let one = builder.ins().iconst(types::I64, 1);
    builder.ins().return_(&[zero, one]);

    builder.seal_all_blocks();
    // finalize() checks the function is complete; a panic here would mean
    // a translation bug, which the ? paths above are meant to prevent.
    Some(())
}

#[derive(Clone, Copy)]
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

/// Checked integer arithmetic matching the VM's semantics: overflow,
/// division by zero, and i64::MIN edge cases all branch to `deopt` —
/// the bytecode re-run then raises the VM's own error.
fn emit_checked_arith(
    builder: &mut FunctionBuilder,
    op: Arith,
    a: ClifValue,
    b: ClifValue,
    deopt: cranelift_codegen::ir::Block,
) -> Option<ClifValue> {
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
            // DivisionByZero and checked-overflow errors respectively).
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

// Silence "unused" for TplPart, imported for exhaustiveness reasoning only.
#[allow(dead_code)]
fn _tplpart_witness(_: &TplPart) {}
