//! The baseline JIT: hot bytecode compiled to native machine code.
//!
//! This is the third tier, and it extends the correctness story unchanged:
//! interpreter → bytecode ("can't compile identically → stay interpreted")
//! → native ("can't compile natively → stay on bytecode"). A function
//! prequalifies at promotion time when every instruction falls in a pure
//! numeric/boolean whitelist — arithmetic, comparisons, branches, calls
//! to other olang functions, return. The actual compilation is
//! **type-specialized, lazy, and call-graph aware**: on a function's
//! first call, the JIT plans every function reachable through its CallFn
//! sites, runs kind inference to a global fixpoint across the group
//! (callee return kinds feed caller registers; masks only grow, so it
//! converges), and compiles the whole group with direct native-to-native
//! calls — helpers, chains, and mutual recursion all stay native. Each
//! compiled function guards its entry on the exact argument kinds it was
//! specialized for (one specialization per function); any other shape
//! runs on bytecode as before.
//!
//! Pure is the load-bearing word: a qualifying group has no side
//! effects, so *any* guard failure (argument-kind mismatch, integer
//! overflow, division by zero, depth exhaustion — anywhere in the native
//! call chain) simply abandons the native run and re-executes the
//! original call on bytecode, which produces the exact result or error
//! the VM would have produced anyway. The JIT never reproduces an error
//! message; it only ever declines.
//!
//! Codegen notes:
//! - Registers are typed by inference: i64 (integers, booleans as 0/1)
//!   or f64. Mixed int/float arithmetic promotes the integer side with
//!   fcvt_from_sint, exactly as the VM's execute_binary_op does.
//! - Checked integer arithmetic is hand-rolled flag math; float add/sub/
//!   mul are plain IEEE (as in the VM), while float division guards
//!   b == 0.0 because olang errors there rather than producing inf.
//!   Float modulo is refused — fmod has no exact IR equivalent.
//! - A register written with conflicting kinds is fine as long as nothing
//!   reads it (an `if` statement's dead result slot); liveness flows
//!   backwards through copies. Dead stores are skipped, but operation
//!   *guards* still run, because the VM would still error on an
//!   overflowing dead computation.
//! - Every call carries a depth budget clamped to the VM's own
//!   max_call_depth; exhaustion deopts, so runaway recursion errors the
//!   canonical way instead of smashing the native stack. Each internal
//!   function returns (value, status); status != 0 deopts the caller
//!   too, unwinding the native stack to the entry wrapper.

use crate::ast::BinaryOp;
use crate::ovm::bytecode::{CompiledBytecode, Instruction};
use crate::ovm::value::{OvmValue, ValueData};
use crate::ovm::FunctionId;

use std::collections::HashMap;
use std::sync::Arc;

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

/// How the VM hands the JIT other functions' bytecode when planning a
/// call graph.
pub type BytecodeLookup<'a> = dyn Fn(FunctionId) -> Option<Arc<CompiledBytecode>> + 'a;

const STATUS_OK: i64 = 0;
const MAX_PARAMS: usize = 16;
/// Sanity bound on how many functions one group may pull in.
const MAX_GROUP: usize = 32;

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
    /// The inner (fast-convention) function, callable from later groups.
    clif_id: cranelift_module::FuncId,
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
        if whitelist_ok(bytecode) {
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
    /// specialization group on first use). None means "no native run
    /// happened" — the caller proceeds on bytecode exactly as before.
    #[inline]
    pub fn try_call(
        &mut self,
        func_id: FunctionId,
        bytecode: &Arc<CompiledBytecode>,
        args: &[OvmValue],
        remaining_depth: u32,
        lookup: &BytecodeLookup,
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
            lookup,
        )
    }

    /// Like try_call, but the caller already extracted raw bits and kinds.
    pub fn try_call_raw(
        &mut self,
        func_id: FunctionId,
        bytecode: &Arc<CompiledBytecode>,
        bits: &[i64],
        kinds: &[Kind],
        remaining_depth: u32,
        lookup: &BytecodeLookup,
    ) -> Option<OvmValue> {
        let idx = func_id.index();
        match self.table.get(idx)? {
            Some(Slot::Ready(_)) => {}
            Some(Slot::Pending) => {
                // First call: specialize the whole reachable group on the
                // kinds this call carries.
                if self
                    .specialize_group(func_id, bytecode, kinds, lookup)
                    .is_none()
                {
                    if jit_debug() {
                        eprintln!("[jit] fn#{} refused (inference or codegen)", idx);
                    }
                    self.table[idx] = Some(Slot::Refused);
                    return None;
                }
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

    /// Plan, infer, and compile the call graph reachable from `entry_id`.
    /// On success every group member becomes Ready. On failure the entry
    /// alone becomes Refused (a helper may still compile later from its
    /// own first call, with its own kinds).
    fn specialize_group(
        &mut self,
        entry_id: FunctionId,
        entry_bytecode: &Arc<CompiledBytecode>,
        entry_kinds: &[Kind],
        lookup: &BytecodeLookup,
    ) -> Option<()> {
        if entry_kinds.len() != entry_bytecode.param_count {
            return None;
        }

        // ── plan + global inference fixpoint ──
        let mut plans: Vec<PlanFn> = vec![PlanFn::new(
            entry_id,
            Arc::clone(entry_bytecode),
            entry_kinds.to_vec(),
        )];
        let mut plan_pos: HashMap<usize, usize> = HashMap::new();
        plan_pos.insert(entry_id.index(), 0);

        loop {
            let mut changed = false;

            // Signature snapshot: kinds + current ret mask per function the
            // group can call (plans lag one iteration; monotone, converges).
            let mut sigs: HashMap<usize, (Vec<Kind>, u8)> = HashMap::new();
            for p in &plans {
                sigs.insert(p.func_id.index(), (p.param_kinds.clone(), p.ret_mask));
            }
            for (i, slot) in self.table.iter().enumerate() {
                if let Some(Slot::Ready(j)) = slot {
                    sigs.insert(i, (j.param_kinds.clone(), kind_mask(j.ret_kind)));
                }
            }

            let mut requests: Vec<(FunctionId, Vec<Kind>)> = Vec::new();
            for plan in plans.iter_mut() {
                plan.infer_pass(&sigs, &mut requests, &mut changed)?;
            }

            for (fid, kinds) in requests {
                let idx = fid.index();
                if let Some(pos) = plan_pos.get(&idx) {
                    if plans[*pos].param_kinds != kinds {
                        return None; // one specialization per function
                    }
                    continue;
                }
                match self.table.get(idx) {
                    Some(Some(Slot::Ready(j))) => {
                        if j.param_kinds != kinds {
                            return None;
                        }
                        continue; // already native; sigs covers it
                    }
                    Some(Some(Slot::Refused)) => return None,
                    _ => {}
                }
                if plans.len() >= MAX_GROUP {
                    return None;
                }
                let bytecode = lookup(fid)?;
                if !whitelist_ok(&bytecode) || bytecode.param_count != kinds.len() {
                    return None;
                }
                plan_pos.insert(idx, plans.len());
                plans.push(PlanFn::new(fid, bytecode, kinds));
                changed = true;
            }

            if !changed {
                break;
            }
        }

        // ── finalize each member's inference ──
        let mut inferences: Vec<Inference> = Vec::with_capacity(plans.len());
        for plan in &plans {
            inferences.push(plan.finalize()?);
        }

        // ── codegen: declare everything, then define everything ──
        // Snapshot previously compiled call targets before borrowing the
        // module (both live in self).
        let mut targets: HashMap<usize, (cranelift_module::FuncId, Kind)> = HashMap::new();
        for (i, slot) in self.table.iter().enumerate() {
            if let Some(Slot::Ready(j)) = slot {
                targets.insert(i, (j.clif_id, j.ret_kind));
            }
        }
        let module = self.module()?;
        let mut clif_ids = Vec::with_capacity(plans.len());
        for (plan, inf) in plans.iter().zip(&inferences) {
            let mut sig = module.make_signature();
            for k in &plan.param_kinds {
                sig.params.push(AbiParam::new(k.clif_type()));
            }
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(inf.ret_kind.clif_type()));
            sig.returns.push(AbiParam::new(types::I64));
            let name = format!("olang_jit_{}", plan.func_id.index());
            let id = module.declare_function(&name, Linkage::Local, &sig).ok()?;
            clif_ids.push(id);
        }

        // Call-site resolver: group members (by plan position) override
        // any snapshot entry.
        for ((plan, inf), clif_id) in plans.iter().zip(&inferences).zip(&clif_ids) {
            targets.insert(plan.func_id.index(), (*clif_id, inf.ret_kind));
        }

        let mut fbc = FunctionBuilderContext::new();
        for ((plan, inf), clif_id) in plans.iter().zip(&inferences).zip(&clif_ids) {
            let mut ctx = module.make_context();
            ctx.func.signature = {
                let mut sig = module.make_signature();
                for k in &plan.param_kinds {
                    sig.params.push(AbiParam::new(k.clif_type()));
                }
                sig.params.push(AbiParam::new(types::I64));
                sig.returns.push(AbiParam::new(inf.ret_kind.clif_type()));
                sig.returns.push(AbiParam::new(types::I64));
                sig
            };
            {
                let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
                translate_body(&mut builder, module, &targets, &plan.bytecode, inf)?;
                builder.finalize();
            }
            if let Err(e) = module.define_function(*clif_id, &mut ctx) {
                if jit_debug() {
                    eprintln!(
                        "[jit] define fn#{} failed: {:?}\nIR:\n{}",
                        plan.func_id.index(),
                        e,
                        ctx.func
                    );
                }
                return None;
            }
            module.clear_context(&mut ctx);
        }

        // Entry wrappers (C ABI) for every member, so each is directly
        // callable from the VM later. Float params/results travel as raw
        // bits in the i64 slots — same bytes, no conversion.
        let mut entries = Vec::with_capacity(plans.len());
        for ((plan, inf), clif_id) in plans.iter().zip(&inferences).zip(&clif_ids) {
            let mut entry_sig = module.make_signature();
            entry_sig.params.push(AbiParam::new(types::I64));
            entry_sig.params.push(AbiParam::new(types::I64));
            entry_sig.params.push(AbiParam::new(types::I64));
            entry_sig.returns.push(AbiParam::new(types::I64));
            let entry_name = format!("olang_jit_{}_entry", plan.func_id.index());
            let entry_fid = module
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

                let mut call_args = Vec::with_capacity(plan.param_kinds.len() + 1);
                for (i, k) in plan.param_kinds.iter().enumerate() {
                    call_args.push(builder.ins().load(
                        k.clif_type(),
                        MemFlags::trusted(),
                        args_ptr,
                        (i * 8) as i32,
                    ));
                }
                call_args.push(depth);

                let inner_ref = module.declare_func_in_func(*clif_id, builder.func);
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
            module.define_function(entry_fid, &mut ctx).ok()?;
            module.clear_context(&mut ctx);
            entries.push(entry_fid);
            let _ = inf; // ret kind used via targets
        }

        module.finalize_definitions().ok()?;

        for (((plan, inf), clif_id), entry_fid) in
            plans.iter().zip(&inferences).zip(&clif_ids).zip(&entries)
        {
            let code = self.module.as_ref()?.get_finalized_function(*entry_fid);
            // SAFETY: signature matches the entry ABI; memory lives as
            // long as the module (owned by this cache).
            let entry: NativeEntry = unsafe { std::mem::transmute(code) };
            let idx = plan.func_id.index();
            if self.table.len() <= idx {
                self.table.resize_with(idx + 1, || None);
            }
            if jit_debug() {
                eprintln!(
                    "[jit] fn#{} compiled to native for {:?}",
                    idx, plan.param_kinds
                );
            }
            self.table[idx] = Some(Slot::Ready(JittedFn {
                entry,
                clif_id: *clif_id,
                param_kinds: plan.param_kinds.clone(),
                ret_kind: inf.ret_kind,
            }));
            self.compiled += 1;
        }
        Some(())
    }
}

// ── prequalification: the syntactic whitelist ──────────────────────────

/// Every instruction the JIT knows how to translate, checked without any
/// type information — cheap enough to run at registration for every
/// promoted function. CallFn targets are resolved (and arity-checked)
/// later, at group-planning time.
fn whitelist_ok(bytecode: &CompiledBytecode) -> bool {
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
        | Instruction::MatchFail
        | Instruction::CallFn { .. } => true,
        Instruction::BinImm { imm, .. } => {
            matches!(imm.data, ValueData::Integer(_) | ValueData::Float(_))
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

fn mask_singleton(mask: u8) -> Option<Kind> {
    match mask {
        K_INT => Some(Kind::Int),
        K_BOOL => Some(Kind::Bool),
        K_FLOAT => Some(Kind::Float),
        _ => None,
    }
}

/// One group member's inference state, carried across global fixpoint
/// iterations.
struct PlanFn {
    func_id: FunctionId,
    bytecode: Arc<CompiledBytecode>,
    param_kinds: Vec<Kind>,
    writes: Vec<u8>,
    allowed: Vec<u8>,
    was_read: Vec<bool>,
    ret_mask: u8,
    eq_pairs: Vec<(u32, u32)>,
    return_regs: Vec<u32>,
}

/// The finalized result codegen consumes.
struct Inference {
    reg_kind: Vec<Option<Kind>>,
    param_kinds: Vec<Kind>,
    ret_kind: Kind,
}

impl PlanFn {
    fn new(func_id: FunctionId, bytecode: Arc<CompiledBytecode>, param_kinds: Vec<Kind>) -> Self {
        let nregs = bytecode.register_count as usize;
        let mut writes = vec![0u8; nregs];
        for (i, k) in param_kinds.iter().enumerate() {
            writes[i] = kind_mask(*k);
        }
        Self {
            func_id,
            bytecode,
            param_kinds,
            writes,
            allowed: vec![K_ANY; nregs],
            was_read: vec![false; nregs],
            ret_mask: 0,
            eq_pairs: Vec::new(),
            return_regs: Vec::new(),
        }
    }

    /// One inference pass. `sigs` maps callable functions to their
    /// (param kinds, current ret mask); unknown callees whose argument
    /// kinds have resolved are pushed onto `requests` for planning.
    /// Returns None on a hard refusal.
    fn infer_pass(
        &mut self,
        sigs: &HashMap<usize, (Vec<Kind>, u8)>,
        requests: &mut Vec<(FunctionId, Vec<Kind>)>,
        global_changed: &mut bool,
    ) -> Option<()> {
        let bytecode = self.bytecode.clone();
        let mut changed = false;

        let const_mask = |idx: u32| -> u8 {
            match bytecode.constants.get(idx as usize).map(|c| &c.data) {
                Some(ValueData::Integer(_)) => K_INT,
                Some(ValueData::Boolean(_)) => K_BOOL,
                Some(ValueData::Unit) => K_UNIT,
                Some(ValueData::Float(_)) => K_FLOAT,
                _ => 0,
            }
        };

        self.eq_pairs.clear();
        self.return_regs.clear();

        macro_rules! grow {
            ($slot:expr, $bits:expr) => {{
                let bits = $bits;
                let slot = &mut $slot;
                if *slot | bits != *slot {
                    *slot |= bits;
                    changed = true;
                }
            }};
        }
        macro_rules! narrow {
            ($r:expr, $mask:expr) => {{
                let r = $r as usize;
                let mask = $mask;
                if self.allowed[r] & mask != self.allowed[r] {
                    self.allowed[r] &= mask;
                    changed = true;
                }
                if !self.was_read[r] {
                    self.was_read[r] = true;
                    changed = true;
                }
            }};
        }

        for inst in &bytecode.instructions {
            match inst {
                Instruction::LoadConst { dst, const_idx } => {
                    grow!(self.writes[dst.0 as usize], const_mask(*const_idx));
                }
                Instruction::Move { dst, src } => {
                    let src_mask = self.writes[src.0 as usize];
                    grow!(self.writes[dst.0 as usize], src_mask);
                    // Liveness flows backwards through copies: the move
                    // reads src only if someone reads dst, and the copied
                    // value must satisfy dst's constraint.
                    if self.was_read[dst.0 as usize] {
                        let dst_allowed = self.allowed[dst.0 as usize];
                        narrow!(src.0, dst_allowed);
                    }
                }
                Instruction::Add { dst, lhs, rhs }
                | Instruction::Sub { dst, lhs, rhs }
                | Instruction::Mul { dst, lhs, rhs }
                | Instruction::Div { dst, lhs, rhs }
                | Instruction::Mod { dst, lhs, rhs } => {
                    narrow!(lhs.0, K_NUM);
                    narrow!(rhs.0, K_NUM);
                    let l = self.writes[lhs.0 as usize];
                    let r = self.writes[rhs.0 as usize];
                    if (l | r) & K_FLOAT != 0 {
                        grow!(self.writes[dst.0 as usize], K_FLOAT);
                    }
                    if l & K_INT != 0 && r & K_INT != 0 {
                        grow!(self.writes[dst.0 as usize], K_INT);
                    }
                }
                Instruction::Neg { dst, src } => {
                    narrow!(src.0, K_NUM);
                    let s = self.writes[src.0 as usize];
                    grow!(self.writes[dst.0 as usize], s & K_NUM);
                }
                Instruction::Eq { dst, lhs, rhs } | Instruction::Ne { dst, lhs, rhs } => {
                    narrow!(lhs.0, K_NUM | K_BOOL);
                    narrow!(rhs.0, K_NUM | K_BOOL);
                    self.eq_pairs.push((lhs.0, rhs.0));
                    grow!(self.writes[dst.0 as usize], K_BOOL);
                }
                Instruction::Lt { dst, lhs, rhs }
                | Instruction::Le { dst, lhs, rhs }
                | Instruction::Gt { dst, lhs, rhs }
                | Instruction::Ge { dst, lhs, rhs } => {
                    narrow!(lhs.0, K_NUM);
                    narrow!(rhs.0, K_NUM);
                    grow!(self.writes[dst.0 as usize], K_BOOL);
                }
                Instruction::And { dst, lhs, rhs } | Instruction::Or { dst, lhs, rhs } => {
                    narrow!(lhs.0, K_BOOL);
                    narrow!(rhs.0, K_BOOL);
                    grow!(self.writes[dst.0 as usize], K_BOOL);
                }
                Instruction::Not { dst, src } => {
                    narrow!(src.0, K_BOOL);
                    grow!(self.writes[dst.0 as usize], K_BOOL);
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
                            narrow!(lhs.0, K_NUM);
                            let l = self.writes[lhs.0 as usize];
                            if (l | imm_mask) & K_FLOAT != 0 {
                                grow!(self.writes[dst.0 as usize], K_FLOAT);
                            }
                            if l & K_INT != 0 && imm_mask == K_INT {
                                grow!(self.writes[dst.0 as usize], K_INT);
                            }
                        }
                        BinaryOp::Equal
                        | BinaryOp::NotEqual
                        | BinaryOp::LessThan
                        | BinaryOp::LessThanEqual
                        | BinaryOp::GreaterThan
                        | BinaryOp::GreaterThanEqual => {
                            narrow!(lhs.0, K_NUM);
                            grow!(self.writes[dst.0 as usize], K_BOOL);
                        }
                        _ => return None,
                    }
                }
                Instruction::Jump { .. } | Instruction::MatchFail => {}
                Instruction::JumpIfTrue { condition, .. }
                | Instruction::JumpIfFalse { condition, .. } => {
                    narrow!(condition.0, K_BOOL);
                }
                Instruction::CallFn { dst, func_id, args } => {
                    if let Some((param_kinds, ret_mask)) = sigs.get(&func_id.index()) {
                        if args.len() != param_kinds.len() {
                            return None;
                        }
                        for (i, a) in args.iter().enumerate() {
                            narrow!(a.0, kind_mask(param_kinds[i]));
                        }
                        let rm = *ret_mask;
                        grow!(self.writes[dst.0 as usize], rm);
                    } else {
                        // Unknown callee: once every argument register has
                        // resolved to a single numeric/bool kind, request it
                        // for planning. Until then, keep iterating.
                        let mut kinds = Vec::with_capacity(args.len());
                        let mut resolved = true;
                        for a in args {
                            match mask_singleton(self.writes[a.0 as usize]) {
                                Some(k) => kinds.push(k),
                                None => {
                                    resolved = false;
                                    break;
                                }
                            }
                        }
                        if resolved {
                            requests.push((*func_id, kinds));
                        }
                    }
                }
                Instruction::Return { value } => {
                    let reg = (*value)?;
                    narrow!(reg.0, K_NUM | K_BOOL);
                    self.return_regs.push(reg.0);
                    grow!(self.ret_mask, self.writes[reg.0 as usize]);
                }
                _ => return None,
            }
        }

        if changed {
            *global_changed = true;
        }
        Some(())
    }

    /// After the global fixpoint: check singletons and produce the
    /// codegen-facing result.
    fn finalize(&self) -> Option<Inference> {
        let nregs = self.writes.len();
        let mut reg_kind: Vec<Option<Kind>> = vec![None; nregs];
        for (r, slot) in reg_kind.iter_mut().enumerate() {
            if self.was_read[r] {
                let k = mask_singleton(self.writes[r])?;
                if kind_mask(k) & self.allowed[r] == 0 {
                    return None;
                }
                *slot = Some(k);
            }
        }

        for (l, r) in &self.eq_pairs {
            let lk = reg_kind[*l as usize]?;
            let rk = reg_kind[*r as usize]?;
            let both_num =
                matches!(lk, Kind::Int | Kind::Float) && matches!(rk, Kind::Int | Kind::Float);
            let both_bool = lk == Kind::Bool && rk == Kind::Bool;
            if !both_num && !both_bool {
                return None;
            }
        }

        let ret_kind = mask_singleton(self.ret_mask)?;
        for r in &self.return_regs {
            if reg_kind[*r as usize] != Some(ret_kind) {
                return None;
            }
        }

        Some(Inference {
            reg_kind,
            param_kinds: self.param_kinds.clone(),
            ret_kind,
        })
    }
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
    targets: &HashMap<usize, (cranelift_module::FuncId, Kind)>,
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
            Instruction::CallFn { dst, func_id, args } => {
                // A native-to-native call (group member or previously
                // compiled function): spend a unit of depth budget, deopt
                // when exhausted.
                let (callee_clif, _callee_ret) = *targets.get(&func_id.index())?;
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
                let callee_ref = module.declare_func_in_func(callee_clif, builder.func);
                let call = builder.ins().call(callee_ref, &call_args);
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
/// IEEE; float division guards b == 0.0 (olang errors there). Float
/// modulo itself is refused — Rust's `%` is fmod, which has no exact IR
/// equivalent, and guessing is how divergence starts.
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
