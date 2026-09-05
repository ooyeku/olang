//! On-stack replacement (Campaign 7, T3): a hot loop inside a function
//! the whole-function JIT refused — a `println` in the prologue, string
//! formatting in the epilogue — no longer strands its iterations on the
//! VM dispatch loop. When a frame's back-edge counter crosses the
//! threshold, the loop region alone is synthesized into a standalone
//! function (parameters = the registers the loop reads, return = the
//! registers it writes that outlive it) and compiled through the
//! ordinary JIT pipeline; the dispatch loop calls it mid-frame and
//! resumes at the loop's exit with the returned state.
//!
//! Soundness rests on the JIT's own qualification: every instruction it
//! accepts is pure with respect to caller-visible state (allocation
//! builds fresh values; `map_set` returns a new map), so a region that
//! fails natively — refusal, deopt, runtime error — is simply discarded
//! and the VM resumes at the loop head with its original registers,
//! which re-raises any real error at the right instruction. The two
//! frame-semantic instructions a region must never contain (`Return`,
//! `TailCallSelf`) are refused here; everything else is the JIT's call.

use super::FunctionId;
use super::bytecode::{
    BytecodeDebugInfo, BytecodeOptimizer, CompiledBytecode, Instruction, Label, Register,
};
use super::jit;
use std::sync::Arc;

/// A synthesized loop region, ready for the JIT and the dispatch loop.
pub struct OsrRegion {
    /// The synthetic function's id (a fresh `FunctionId`, same allocator
    /// as lambdas — no collisions).
    pub region_id: FunctionId,
    pub synth: Arc<CompiledBytecode>,
    /// The loop head pc in the original function — the only pc an OSR
    /// entry may replace.
    pub head: usize,
    /// Where the VM resumes after a successful native run: the loop's
    /// single exit target.
    pub exit_pc: usize,
    /// Original registers passed in, in parameter order.
    pub live_in: Vec<Register>,
    /// Original registers the native run's result writes back, in tuple
    /// element order (or the single return value when length is 1).
    pub live_out: Vec<Register>,
}

/// The raw entry marshals at most 16 arguments; tuple returns carry at
/// most `MAX_TUPLE` (8) elements. Loop state beyond that stays on the VM.
const MAX_LIVE_IN: usize = crate::ovm::jit::MAX_PARAMS;
const MAX_LIVE_OUT: usize = crate::ovm::jit::MAX_TUPLE;

fn osr_debug() -> bool {
    std::env::var_os("OLANG_OSR_DEBUG").is_some()
}

/// Build the standalone loop function for the maximal loop region
/// containing the hot back edge — for a nested loop, the outermost
/// enclosing loop, so the whole nest runs native. None when the shape
/// is unsupported: jumps into the region's interior, more than one
/// exit target, frame-semantic instructions inside, or live state past
/// the marshal caps. The JIT's own qualification runs later, on the
/// synthesized function — a region it refuses costs one attempt, ever.
pub fn synthesize(bytecode: &CompiledBytecode, head: usize) -> Option<OsrRegion> {
    // The outermost enclosing loop is the most profitable region — the
    // whole nest runs native. But an outer loop may carry constructs
    // the tier cannot take (a map with a fresh closure per epoch, say)
    // while the hot inner loop is a clean counter loop: when the merged
    // region refuses, retry with the innermost loop containing the hot
    // back edge before giving up.
    let outer = enclosing_loop_region(bytecode, head)?;
    let mut candidates: Vec<(usize, usize)> = vec![outer];
    for iv in raw_intervals_containing(bytecode, head) {
        if !candidates.contains(&iv) {
            candidates.push(iv);
        }
    }
    // Largest first: the widest region that both synthesizes and passes
    // the JIT's syntactic whitelist wins — a tiny inner loop would
    // otherwise be picked over a compilable middle loop and spend its
    // native entry overhead on two iterations.
    candidates.sort_by_key(|(s, e)| std::cmp::Reverse(e - s));
    for (h, e) in candidates {
        if let Some(r) = synthesize_region(bytecode, h, e) {
            if crate::ovm::jit::whitelist_ok(&r.synth) {
                return Some(r);
            }
            if osr_debug() {
                eprintln!(
                    "[osr] region {}..={} synthesized but fails the JIT whitelist; trying a smaller region",
                    h, e
                );
            }
        }
    }
    None
}

/// Every raw back-edge interval containing `head`, unmerged — the
/// nesting levels around the hot loop.
fn raw_intervals_containing(bytecode: &CompiledBytecode, head: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (pc, inst) in bytecode.instructions.iter().enumerate() {
        let target = match inst {
            Instruction::Jump { target }
            | Instruction::JumpIfTrue { target, .. }
            | Instruction::JumpIfFalse { target, .. } => Some(target.0 as usize),
            Instruction::TailCallSelf { .. } => Some(bytecode.entry_point),
            _ => None,
        };
        if let Some(t) = target
            && t <= pc
            && (t..=pc).contains(&head)
        {
            out.push((t, pc));
        }
    }
    out
}

fn synthesize_region(bytecode: &CompiledBytecode, h: usize, e: usize) -> Option<OsrRegion> {
    if e < h {
        return None;
    }
    let insts = &bytecode.instructions;
    let region = &insts[h..=e];

    // Frame semantics stay with the frame.
    if region.iter().any(|i| {
        matches!(
            i,
            Instruction::Return { .. } | Instruction::TailCallSelf { .. }
        )
    }) {
        refuse(bytecode, "return or tail-call inside the region");
        return None;
    }

    // Single entry: nothing outside the region may jump past the head.
    for (pc, inst) in insts.iter().enumerate() {
        if (h..=e).contains(&pc) {
            continue;
        }
        for t in jump_targets(inst) {
            if t > h && t <= e {
                refuse(bytecode, "a jump enters the loop mid-body");
                return None;
            }
        }
    }

    // Exits: in-region jumps landing outside, plus the fall-through when
    // the region's last instruction is a conditional back-edge.
    let mut exits: Vec<usize> = Vec::new();
    for (i, inst) in region.iter().enumerate() {
        for t in jump_targets(inst) {
            if !(h..=e).contains(&t) && !exits.contains(&t) {
                exits.push(t);
            }
        }
        if i == e - h
            && matches!(
                inst,
                Instruction::JumpIfTrue { .. } | Instruction::JumpIfFalse { .. }
            )
            && !exits.contains(&(e + 1))
        {
            exits.push(e + 1);
        }
    }
    if exits.len() != 1 {
        refuse(bytecode, "the loop has several exit targets");
        return None;
    }
    let exit_pc = exits[0];

    // Reads and writes inside the region, from the optimizer's total
    // model. Compilability is not judged here — the JIT's whitelist
    // gates the synthesized function; a refused region falls to the
    // next-smaller candidate.
    let nregs = bytecode.register_count as usize;
    let mut reads = vec![false; nregs];
    let mut writes = vec![false; nregs];
    for inst in region {
        let (uses, defs) = BytecodeOptimizer::uses_defs(inst);
        for u in uses {
            if (u as usize) < nregs {
                reads[u as usize] = true;
            }
        }
        for d in defs {
            if (d as usize) < nregs {
                writes[d as usize] = true;
            }
        }
    }

    // Arguments: registers the region reads that carry a value at the
    // head. Results: registers it writes that something after the exit
    // still reads.
    let live_head = jit::live_in_at(bytecode, nregs, h);
    let live_in: Vec<Register> = (0..nregs as u32)
        .filter(|r| reads[*r as usize] && live_head[*r as usize])
        .map(Register)
        .collect();
    let live_exit = jit::live_in_at(bytecode, nregs, exit_pc);
    let live_out: Vec<Register> = (0..nregs as u32)
        .filter(|r| writes[*r as usize] && live_exit[*r as usize])
        .map(Register)
        .collect();
    if live_in.len() > MAX_LIVE_IN || live_out.is_empty() || live_out.len() > MAX_LIVE_OUT {
        refuse(
            bytecode,
            &format!(
                "live state past the marshal caps ({} in / {} out; caps {} / {})",
                live_in.len(),
                live_out.len(),
                MAX_LIVE_IN,
                MAX_LIVE_OUT
            ),
        );
        return None;
    }

    // Renumber: parameters take 0..k in order, every other register the
    // region touches gets the next fresh slot.
    let mut map: Vec<Option<u32>> = vec![None; nregs];
    for (i, r) in live_in.iter().enumerate() {
        map[r.0 as usize] = Some(i as u32);
    }
    let mut next = live_in.len() as u32;
    let mut renumber = |r: &mut Register, map: &mut Vec<Option<u32>>| {
        let slot = &mut map[r.0 as usize];
        if slot.is_none() {
            *slot = Some(next);
            next += 1;
        }
        r.0 = slot.unwrap();
    };
    let region_len = region.len();
    let mut out: Vec<Instruction> = Vec::with_capacity(region_len + 2);
    for inst in region {
        let mut inst = inst.clone();
        let mut ok = true;
        if !jit::for_each_reg(&mut inst, |r| renumber(r, &mut map)) {
            ok = false;
        }
        if !ok {
            refuse(bytecode, "an instruction without a register model");
            return None;
        }
        // Interior targets shift to region-relative; the one exit lands
        // on the epilogue.
        jit::remap_targets(&mut inst, &|t| {
            let t = t as usize;
            if (h..=e).contains(&t) {
                (t - h) as u32
            } else {
                region_len as u32
            }
        });
        out.push(inst);
    }

    // Epilogue: hand the surviving state back — one value directly, a
    // few as a tuple (the JIT returns tuples through out slots, no
    // allocation).
    let live_out_new: Vec<Register> = live_out
        .iter()
        .map(|r| Register(map[r.0 as usize].expect("live-out is written in the region")))
        .collect();
    if live_out_new.len() == 1 {
        out.push(Instruction::Return {
            value: Some(live_out_new[0]),
        });
    } else {
        let dst = Register(next);
        next += 1;
        out.push(Instruction::MakeTuple {
            dst,
            elements: live_out_new,
        });
        out.push(Instruction::Return { value: Some(dst) });
    }

    let region_id = FunctionId::new();
    let name = bytecode
        .debug_info
        .function_name
        .as_deref()
        .unwrap_or("<anon>");
    if osr_debug() {
        eprintln!(
            "[osr] '{}' loop [{}, {}] -> fn#{}: {} in, {} out, exit pc {}",
            name,
            h,
            e,
            region_id.index(),
            live_in.len(),
            live_out.len(),
            exit_pc
        );
    }
    let synth = CompiledBytecode {
        function_id: region_id,
        callee_names: Vec::new(),
        instructions: out,
        register_count: next,
        local_count: 0,
        param_count: live_in.len(),
        def_file: bytecode.def_file.clone(),
        param_names: Arc::from(Vec::<String>::new()),
        param_checks: Arc::from(Vec::<Option<crate::ast::FieldTypeCheck>>::new()),
        return_check: None,
        constants: bytecode.constants.clone(),
        // No spans: a native failure resumes the VM at the loop head,
        // which re-raises any real error with the original function's
        // spans.
        span_table: Vec::new(),
        debug_info: BytecodeDebugInfo {
            function_name: Some(format!("{}::osr", name)),
            ..Default::default()
        },
        optimization_level: bytecode.optimization_level,
        entry_point: 0,
    };
    Some(OsrRegion {
        region_id,
        synth: Arc::new(synth),
        head: h,
        exit_pc,
        live_in,
        live_out,
    })
}

/// The maximal loop region containing `head`. Every backward jump is an
/// interval `[target, pc]`; transitively merging the intervals that
/// overlap dissolves nesting — an inner loop's interval merges into its
/// enclosing loop's — while sequential loops stay separate regions. The
/// merged interval containing the hot head is the region worth
/// synthesizing: for a nested loop that is the *outermost* enclosing
/// loop, so the whole nest runs native, and its head (not the hot inner
/// head) is where the dispatch loop must enter — `OsrRegion::head`
/// records it, and the dispatch re-offers entry when execution next
/// takes a back edge to it.
fn enclosing_loop_region(bytecode: &CompiledBytecode, head: usize) -> Option<(usize, usize)> {
    let mut intervals: Vec<(usize, usize)> = Vec::new();
    for (pc, inst) in bytecode.instructions.iter().enumerate() {
        let target = match inst {
            Instruction::Jump { target }
            | Instruction::JumpIfTrue { target, .. }
            | Instruction::JumpIfFalse { target, .. } => Some(target.0 as usize),
            Instruction::TailCallSelf { .. } => Some(bytecode.entry_point),
            _ => None,
        };
        if let Some(t) = target
            && t <= pc
        {
            intervals.push((t, pc));
        }
    }
    intervals.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in intervals {
        match merged.last_mut() {
            Some((_, me)) if s <= *me => *me = (*me).max(e),
            _ => merged.push((s, e)),
        }
    }
    merged.into_iter().find(|(s, e)| (*s..=*e).contains(&head))
}

fn refuse(bytecode: &CompiledBytecode, why: &str) {
    if osr_debug() {
        eprintln!(
            "[osr] '{}' refused: {}",
            bytecode
                .debug_info
                .function_name
                .as_deref()
                .unwrap_or("<anon>"),
            why
        );
    }
}

fn jump_targets(inst: &Instruction) -> impl Iterator<Item = usize> {
    let t: Option<&Label> = match inst {
        Instruction::Jump { target }
        | Instruction::JumpIfTrue { target, .. }
        | Instruction::JumpIfFalse { target, .. } => Some(target),
        _ => None,
    };
    t.map(|l| l.0 as usize).into_iter()
}
