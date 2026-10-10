//! The compiler's side of the compile cache (`crate::compile_cache`):
//! the questions a compile asks of its scope, recorded with their
//! answers' hashes as it runs; an entry made from a finished compile;
//! and an entry used — every question asked again of this run's scope,
//! the constants looked up again, the ids mapped to this run's.
//!
//! Every read the compiler makes of a registry, the closure, the
//! function's run, the module's scope or the capability table goes
//! through a `q_*` method here, which records it. The answer's hash
//! covers what the compile does with the answer and no more: a name's
//! kind (a function, and of which name; a builtin, and which), a
//! scalar's value (a constant condition folds), a known function's
//! parameters — so an entry survives what does not change its code (a
//! module value's contents, a function's body elsewhere) and never
//! survives what does.

use super::{BytecodeCompiler, BytecodeVm, CompiledBytecode, Instruction};
use crate::ast::Value;
use crate::compile_cache::{self as cc, ConstSrc, Entry, IdRef, LambdaSrc, Member, Rec};
use crate::ovm::{FunctionId, OvmValue};
use std::collections::HashMap;
use std::sync::Arc;

// The questions. Numbers are part of the entry format.
pub(super) const T_SEG: u8 = 0;
pub(super) const T_LAZY: u8 = 1;
const T_REG: u8 = 2;
const T_BUILTIN: u8 = 3;
const T_AMBIG: u8 = 4;
const T_BRIDGED: u8 = 5;
const T_REFUSED: u8 = 6;
const T_UNIT_VARIANT: u8 = 7;
const T_KNOWN: u8 = 8;
const T_LEXICAL: u8 = 9;
const T_CALLEE: u8 = 10;
const T_IDENT: u8 = 11;
const T_FREE: u8 = 12;
const T_MODFN: u8 = 13;
const T_STRUCT: u8 = 14;
const T_FIELDS: u8 = 15;
const T_ALIASES: u8 = 16;
const T_GRANT: u8 = 17;

/// What one compile (a function and its lambdas) has recorded so far.
#[derive(Default)]
pub(crate) struct Recording {
    pub(super) log: Vec<Rec>,
    /// (id, name) for each direct call the compile resolved by name.
    pub(super) ids: Vec<(u64, String)>,
    /// Each compiled member's constants' origins, by its id.
    pub(super) srcs: Vec<(u64, Vec<Option<ConstSrc>>)>,
    /// The members as installed: lambdas in compile order, the function
    /// last.
    pub(super) group: Vec<Arc<CompiledBytecode>>,
    /// The questions this member has recorded, by a hash of question
    /// and answer (each compared whole): a question asked again with the
    /// same answer is not recorded again. (Between the two, the scope can
    /// have changed only by ids given to callees — replayed the same way
    /// — so the second answer follows from the first.)
    asked: HashMap<u64, Vec<usize>>,
}

impl BytecodeCompiler {
    /// Record a question and its answer's hash (nothing when not
    /// recording).
    #[inline]
    fn note(&self, tag: u8, a: &str, b: &str, h: impl FnOnce() -> u64) {
        if !self.rec_on {
            return;
        }
        // the answer first: working it out asks nothing that records
        let h = h();
        if let Some(r) = self.rec.borrow_mut().as_mut() {
            if tag == T_SEG {
                r.asked.clear();
            } else if tag != T_LAZY {
                let at = cc::answer(&(tag, a, b, h));
                let seen = r.asked.entry(at).or_default();
                if seen.iter().any(|&i| {
                    let q = &r.log[i];
                    q.tag == tag && q.a == a && q.b == b && q.h == h
                }) {
                    return;
                }
                seen.push(r.log.len());
            }
            r.log.push(Rec {
                tag,
                a: a.to_string(),
                b: b.to_string(),
                h,
            });
        }
    }

    pub(super) fn note_segment(&self, member: usize, id: FunctionId) {
        self.note(T_SEG, &member.to_string(), "", || id.raw());
    }

    pub(super) fn note_lazy(&self, name: &str, id: FunctionId) {
        self.note(T_LAZY, name, "", || id.raw());
    }

    pub(super) fn note_call_id(&self, id: FunctionId, name: &str) {
        if self.rec_on
            && let Ok(mut rec) = self.rec.try_borrow_mut()
            && let Some(r) = rec.as_mut()
        {
            r.ids.push((id.raw(), name.to_string()));
        }
    }

    pub(super) fn note_constant_srcs(&mut self, id: FunctionId) {
        if !self.rec_on {
            return;
        }
        let srcs = std::mem::take(&mut self.emitter.const_srcs);
        if let Ok(mut rec) = self.rec.try_borrow_mut()
            && let Some(r) = rec.as_mut()
        {
            r.srcs.push((id.raw(), srcs));
        }
    }

    // --- the questions -------------------------------------------------

    pub(super) fn q_registry(&self, name: &str) -> Option<FunctionId> {
        let v = self.function_registry.get(name).copied();
        self.note(T_REG, name, "", || cc::answer(&v.is_some()));
        v
    }

    pub(super) fn q_builtin(&self, name: &str) -> bool {
        let v = self.builtin_names.contains(name);
        self.note(T_BUILTIN, name, "", || cc::answer(&v));
        v
    }

    pub(super) fn q_ambiguous(&self, name: &str) -> bool {
        let v = self.ambiguous_names.contains(name);
        self.note(T_AMBIG, name, "", || cc::answer(&v));
        v
    }

    pub(super) fn q_bridged(&self, name: &str) -> bool {
        let v = self.bridged_callees.contains(name);
        self.note(T_BRIDGED, name, "", || cc::answer(&v));
        v
    }

    pub(super) fn q_refused(&self, name: &str) -> bool {
        let v = self.refused_names.contains(name);
        self.note(T_REFUSED, name, "", || cc::answer(&v));
        v
    }

    pub(super) fn q_unit_variant(&self, name: &str) -> bool {
        let v = self.unit_variant_names.contains(name);
        self.note(T_UNIT_VARIANT, name, "", || cc::answer(&v));
        v
    }

    pub(super) fn q_known(&self, name: &str) -> Option<&Arc<crate::ast::Function>> {
        let v = self.known_function_values.get(name);
        self.note(T_KNOWN, name, "", || self.known_class(v));
        v
    }

    /// What the function sees lexically (`lexical`), as far as a compile
    /// looks: a function's name and whether it is the known one, a
    /// builtin's name, a constructor's type, variant and arity, whether
    /// an enum is a unit variant.
    pub(super) fn q_lexical(&self, name: &str) -> Option<Value> {
        let v = self.lexical(name);
        self.note(T_LEXICAL, name, "", || self.lexical_class(name, v.as_ref()));
        v
    }

    /// A called name's value (`callee_in_scope`): a function, and of
    /// which name, or anything else.
    pub(super) fn q_callee(&self, name: &str) -> Option<Value> {
        let v = self.callee_in_scope(name);
        self.note(T_CALLEE, name, "", || Self::callee_class(v.as_ref()));
        v
    }

    /// A free identifier's value: a function, a scalar and its value, a
    /// value that crosses to the VM, or one that does not.
    pub(super) fn q_ident(&self, name: &str) -> Option<Value> {
        let v = self.ident_value(name);
        self.note(T_IDENT, name, "", || Self::ident_class(v.as_ref()));
        v
    }

    /// Whether a lambda's free name is in the closure, the run or the
    /// module's scope.
    pub(super) fn q_free_present(&self, name: &str) -> bool {
        let v = self.free_present(name);
        self.note(T_FREE, name, "", || cc::answer(&v));
        v
    }

    /// The builtin `module.field` names, when the closure's `module` is a
    /// module whose `field` is a builtin.
    pub(super) fn q_module_builtin(&self, module: &str, field: &str) -> Option<String> {
        let v = self.module_builtin(module, field);
        self.note(T_MODFN, module, field, || cc::answer(&v));
        v
    }

    pub(super) fn q_struct_def(&self, type_name: &str) -> Option<&Vec<String>> {
        let v = self.struct_defs.get(type_name);
        self.note(T_STRUCT, type_name, "", || cc::answer(&v));
        v
    }

    pub(super) fn q_struct_checks(
        &self,
        type_name: &str,
    ) -> Option<&HashMap<String, crate::ast::FieldTypeCheck>> {
        let v = self.struct_field_checks.get(type_name);
        self.note(T_FIELDS, type_name, "", || Self::checks_class(v));
        v
    }

    pub(super) fn q_aliases(&self) -> &HashMap<String, crate::ast::TypeAnnotation> {
        self.note(T_ALIASES, "", "", || self.aliases_class());
        &self.type_aliases
    }

    /// The grant governing the function (`static_grant`).
    pub(super) fn q_grant(&self) -> Option<crate::caps::Caps> {
        let v = self.static_grant();
        self.note(T_GRANT, "", "", || cc::answer(&format!("{:?}", v)));
        v
    }

    /// A recorded question asked again of this run's scope.
    fn answer_again(&self, rec: &Rec) -> Option<u64> {
        let name = rec.a.as_str();
        Some(match rec.tag {
            T_REG => cc::answer(&self.function_registry.contains_key(name)),
            T_BUILTIN => cc::answer(&self.builtin_names.contains(name)),
            T_AMBIG => cc::answer(&self.ambiguous_names.contains(name)),
            T_BRIDGED => cc::answer(&self.bridged_callees.contains(name)),
            T_REFUSED => cc::answer(&self.refused_names.contains(name)),
            T_UNIT_VARIANT => cc::answer(&self.unit_variant_names.contains(name)),
            T_KNOWN => self.known_class(self.known_function_values.get(name)),
            T_LEXICAL => self.lexical_class(name, self.lexical(name).as_ref()),
            T_CALLEE => Self::callee_class(self.callee_in_scope(name).as_ref()),
            T_IDENT => Self::ident_class(self.ident_value(name).as_ref()),
            T_FREE => cc::answer(&self.free_present(name)),
            T_MODFN => cc::answer(&self.module_builtin(name, &rec.b)),
            T_STRUCT => cc::answer(&self.struct_defs.get(name)),
            T_FIELDS => Self::checks_class(self.struct_field_checks.get(name)),
            T_ALIASES => self.aliases_class(),
            T_GRANT => cc::answer(&format!("{:?}", self.static_grant())),
            _ => return None,
        })
    }

    // --- the answers' hashes --------------------------------------------

    fn known_class(&self, f: Option<&Arc<crate::ast::Function>>) -> u64 {
        let Some(f) = f else {
            return cc::answer(&0u8);
        };
        let ptr = Arc::as_ptr(f) as usize;
        if let Ok(memo) = self.known_digests.try_borrow()
            && let Some((held, h)) = memo.get(&ptr)
            && Arc::ptr_eq(held, f)
        {
            return *h;
        }
        let params = postcard::to_stdvec(&f.parameters).unwrap_or_default();
        let h = cc::answer(&(1u8, &f.name, params, &f.param_bounds));
        if let Ok(mut memo) = self.known_digests.try_borrow_mut() {
            // functions a session has replaced are let go now and then
            if memo.len() >= 65_536 {
                memo.clear();
            }
            memo.insert(ptr, (f.clone(), h));
        }
        h
    }

    fn lexical_class(&self, name: &str, v: Option<&Value>) -> u64 {
        match v {
            None => cc::answer(&0u8),
            Some(Value::Function(f)) => {
                let same = self
                    .known_function_values
                    .get(name)
                    .is_some_and(|k| Arc::ptr_eq(&k.body, &f.body));
                cc::answer(&(1u8, &f.name, same))
            }
            Some(Value::Builtin(b)) => cc::answer(&(2u8, &b.name)),
            Some(Value::EnumConstructor(c)) => {
                cc::answer(&(3u8, &c.type_name, &c.variant_name, c.arity))
            }
            Some(Value::Enum(e)) => cc::answer(&(
                4u8,
                matches!(e.variant_data, crate::ast::EnumVariantData::Unit),
            )),
            Some(_) => cc::answer(&5u8),
        }
    }

    fn callee_class(v: Option<&Value>) -> u64 {
        match v {
            None => cc::answer(&0u8),
            Some(Value::Function(f)) => cc::answer(&(1u8, &f.name)),
            Some(_) => cc::answer(&2u8),
        }
    }

    fn ident_class(v: Option<&Value>) -> u64 {
        match v {
            None => cc::answer(&0u8),
            Some(Value::Function(_)) => cc::answer(&1u8),
            // A value the VM takes is loaded as a constant, looked up
            // again when the entry is used; the compile looks at none but
            // a Boolean's (a constant condition folds, and its dead
            // branch is swept).
            Some(Value::Boolean(b)) => cc::answer(&(2u8, *b)),
            Some(v) if BytecodeVm::round_trips(v) => {
                cc::answer(&(3u8, std::mem::discriminant(v)))
            }
            Some(_) => cc::answer(&4u8),
        }
    }

    fn checks_class(v: Option<&HashMap<String, crate::ast::FieldTypeCheck>>) -> u64 {
        let Some(map) = v else {
            return cc::answer(&0u8);
        };
        let mut rows: Vec<(&String, &crate::ast::FieldTypeCheck)> = map.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        cc::answer(&(1u8, postcard::to_stdvec(&rows).unwrap_or_default()))
    }

    fn aliases_class(&self) -> u64 {
        let mut rows: Vec<(&String, &crate::ast::TypeAnnotation)> = self.type_aliases.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        cc::answer(&postcard::to_stdvec(&rows).unwrap_or_default())
    }

    // --- the lookups behind them ----------------------------------------

    /// A free identifier: the closure, then the run, the module's
    /// finished scope, then the known function (unless ambiguous).
    pub(super) fn ident_value(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.enclosing_closure.get(name) {
            return Some(v.clone());
        }
        if let Some(v) = self.enclosing_run.get(name) {
            return Some(v);
        }
        if let Some(v) = self.module_scope.as_ref().and_then(|m| m.get(name)) {
            return Some(v.clone());
        }
        (!self.ambiguous_names.contains(name))
            .then(|| self.known_function_values.get(name))
            .flatten()
            .map(|f| Value::Function(f.clone()))
    }

    fn free_present(&self, name: &str) -> bool {
        self.enclosing_closure.contains_key(name)
            || self.enclosing_run.get(name).is_some()
            || self
                .module_scope
                .as_ref()
                .is_some_and(|m| m.contains_key(name))
    }

    fn module_builtin(&self, module: &str, field: &str) -> Option<String> {
        match self.enclosing_closure.get(module) {
            Some(Value::Struct { type_name, fields }) if type_name == "Module" => {
                match fields.get(field) {
                    Some(Value::Builtin(b)) => Some(b.name.clone()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The value a lambda's closure takes for a free name: the closure,
    /// the run, the module's scope, a known function.
    pub(super) fn capture_value(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.enclosing_closure.get(name) {
            return Some(v.clone());
        }
        if let Some(v) = self.enclosing_run.get(name) {
            return Some(v);
        }
        if let Some(v) = self.module_scope.as_ref().and_then(|m| m.get(name)) {
            return Some(v.clone());
        }
        self.known_function_values
            .get(name)
            .map(|f| Value::Function(f.clone()))
    }

    /// A lambda's function value, as the compiler builds it.
    pub(super) fn lambda_function(&self, src: &LambdaSrc) -> Option<crate::ast::Function> {
        let mut closure = im::HashMap::new();
        for name in &src.captured {
            closure.insert(name.clone(), self.capture_value(name)?);
        }
        Some(crate::ast::Function {
            name: src.self_name.clone(),
            param_checks: crate::ast::param_checks_of(&src.params, &[]),
            return_check: None,
            parameters: src.params.clone(),
            body: Arc::new(src.body.clone()),
            closure: Arc::new(closure),
            param_bounds: Vec::new(),
            def_file: None,
            parent_scope: 0,
            run: self.enclosing_run.clone(),
        })
    }
}

impl BytecodeVm {
    /// The key of a compile: the build, the declaration, its checks and
    /// file, and whether callees compile at their first call.
    pub(super) fn cache_key(
        &self,
        func: &crate::ast::FunctionDecl,
        param_checks: &[Option<crate::ast::FieldTypeCheck>],
        return_check: &Option<crate::ast::FieldTypeCheck>,
        def_file: &Option<Arc<str>>,
    ) -> Option<cc::Key> {
        let mut h = cc::KeyHasher::new();
        h.bytes(def_file.as_deref().unwrap_or("\u{0}").as_bytes());
        h.bytes(&[self.lazy_callees as u8]);
        if !(h.value(func) && h.value(param_checks) && h.value(return_check)) {
            return None;
        }
        Some(h.finish())
    }

    /// Start recording the compile about to run.
    pub(super) fn record_start(&mut self) {
        self.compiler.rec_on = true;
        self.compiler.emitter.recording = true;
        *self.compiler.rec.borrow_mut() = Some(Recording::default());
    }

    /// Stop recording: what the compile recorded.
    pub(super) fn record_stop(&mut self) -> Option<Recording> {
        self.compiler.rec_on = false;
        self.compiler.emitter.recording = false;
        self.compiler.emitter.const_srcs.clear();
        self.compiler.rec.borrow_mut().take()
    }

    /// An entry from a finished compile, or `None` when it met anything
    /// an entry cannot express.
    pub(super) fn entry_of(rec: Recording, func_id: FunctionId) -> Option<Entry> {
        if rec.group.is_empty() {
            return None;
        }
        // the function first, then its lambdas in the order they compiled
        let mut group = rec.group;
        let main = group.pop()?;
        if main.function_id != func_id {
            return None;
        }
        let order: Vec<Arc<CompiledBytecode>> = std::iter::once(main).chain(group).collect();
        // the segments name the same members, in the same order
        let segs: Vec<u64> = rec.log.iter().filter(|r| r.tag == T_SEG).map(|r| r.h).collect();
        if segs.len() != order.len()
            || segs.iter().zip(&order).any(|(s, m)| *s != m.function_id.raw())
        {
            return None;
        }
        let member_of: HashMap<u64, u32> = order
            .iter()
            .enumerate()
            .map(|(i, m)| (m.function_id.raw(), i as u32))
            .collect();
        // an id reached under two names could be two ids in another run:
        // not kept
        let mut by_name: HashMap<u64, &String> = HashMap::new();
        for (id, n) in &rec.ids {
            if by_name.insert(*id, n).is_some_and(|was| was != n) {
                return None;
            }
        }
        let srcs: HashMap<u64, &Vec<Option<ConstSrc>>> =
            rec.srcs.iter().map(|(id, s)| (*id, s)).collect();
        let mut ids: Vec<(u64, IdRef)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut members = Vec::with_capacity(order.len());
        for code in &order {
            let d = &code.debug_info;
            if !d.instruction_to_source.is_empty()
                || !d.register_names.is_empty()
                || !d.local_variables.is_empty()
                || !d.line_table.is_empty()
            {
                return None;
            }
            for inst in &code.instructions {
                if let Instruction::CallFn { func_id, .. } = inst {
                    let raw = func_id.raw();
                    if !seen.insert(raw) {
                        continue;
                    }
                    let r = match member_of.get(&raw) {
                        Some(i) => IdRef::Member(*i),
                        None => IdRef::Registry((*by_name.get(&raw)?).clone()),
                    };
                    ids.push((raw, r));
                }
            }
            let src = srcs.get(&code.function_id.raw())?;
            if src.len() != code.constants.len() {
                return None;
            }
            let constants = code
                .constants
                .iter()
                .zip(src.iter())
                .map(|(value, s)| match s {
                    Some(s) => {
                        if let ConstSrc::Closure { func_id, .. } = s
                            && !member_of.contains_key(func_id)
                        {
                            return None;
                        }
                        Some(s.clone())
                    }
                    None => cc::plain_of(value).map(ConstSrc::Plain),
                })
                .collect::<Option<Vec<_>>>()?;
            members.push(Member {
                old_id: code.function_id.raw(),
                instructions: code.instructions.clone(),
                register_count: code.register_count,
                local_count: code.local_count,
                param_count: code.param_count,
                def_file: code.def_file.as_deref().map(str::to_string),
                param_names: code.param_names.to_vec(),
                param_checks: code.param_checks.to_vec(),
                return_check: code.return_check.clone(),
                constants,
                span_table: code.span_table.clone(),
                callee_names: code.callee_names.clone(),
                function_name: code.debug_info.function_name.clone(),
                optimization_level: code.optimization_level,
                entry_point: code.entry_point,
            });
        }
        Some(Entry {
            log: rec.log,
            members,
            ids,
        })
    }

    /// The group an entry holds, if every question it recorded gets the
    /// same answer from this run's scope: the code with this run's ids
    /// and constants, the function first. On a refusal nothing it did
    /// stays (the ids it gave callees are withdrawn).
    pub(super) fn cached_group(
        &mut self,
        entry: Arc<Entry>,
        func_id: FunctionId,
        closure: &Arc<im::HashMap<String, Value>>,
    ) -> Option<Vec<Arc<CompiledBytecode>>> {
        let mut lazy: Vec<(String, FunctionId)> = Vec::new();
        let group = self.replay(entry, func_id, closure, &mut lazy);
        if group.is_none() {
            for (name, id) in lazy {
                if self.compiler.function_registry.get(&name) == Some(&id) {
                    self.compiler.function_registry.remove(&name);
                }
            }
            return None;
        }
        self.compiler.lazy_new.extend(lazy);
        group
    }

    fn replay(
        &mut self,
        entry: Arc<Entry>,
        func_id: FunctionId,
        closure: &Arc<im::HashMap<String, Value>>,
        lazy: &mut Vec<(String, FunctionId)>,
    ) -> Option<Vec<Arc<CompiledBytecode>>> {
        if entry.members.is_empty() {
            return None;
        }
        let mut new_ids: HashMap<u64, FunctionId> = HashMap::new();
        for (i, m) in entry.members.iter().enumerate() {
            new_ids.insert(m.old_id, if i == 0 { func_id } else { FunctionId::new() });
        }
        // each member's questions, after its segment mark
        let mut segments: Vec<&[Rec]> = Vec::new();
        let mut start = None;
        for (i, r) in entry.log.iter().enumerate() {
            if r.tag == T_SEG {
                if let Some(s) = start {
                    segments.push(&entry.log[s..i]);
                }
                start = Some(i + 1);
            }
        }
        segments.push(&entry.log[start?..]);
        if segments.len() != entry.members.len() {
            return None;
        }
        // a lambda's closure, by its old id, made by its parent's template
        let mut contexts: HashMap<u64, Arc<im::HashMap<String, Value>>> = HashMap::new();
        let mut constants: Vec<Vec<OvmValue>> = Vec::with_capacity(entry.members.len());
        for (i, member) in entry.members.iter().enumerate() {
            self.compiler.enclosing_closure = if i == 0 {
                closure.clone()
            } else {
                contexts.get(&member.old_id)?.clone()
            };
            for rec in segments[i] {
                if rec.tag == T_LAZY {
                    if self.compiler.function_registry.contains_key(&rec.a) {
                        return None;
                    }
                    let id = FunctionId::new();
                    self.compiler.function_registry.insert(rec.a.clone(), id);
                    lazy.push((rec.a.clone(), id));
                    continue;
                }
                if self.compiler.answer_again(rec)? != rec.h {
                    return None;
                }
            }
            let mut values = Vec::with_capacity(member.constants.len());
            for src in &member.constants {
                values.push(self.rebuild_constant(src, &new_ids, &mut contexts)?);
            }
            constants.push(values);
        }
        // ids: the members', then the registry's (the owed ones given above)
        let mut id_map = new_ids;
        for (old, r) in &entry.ids {
            let id = match r {
                IdRef::Member(i) => *id_map.get(&entry.members.get(*i as usize)?.old_id)?,
                IdRef::Registry(name) => *self.compiler.function_registry.get(name)?,
            };
            id_map.insert(*old, id);
        }
        // the code moved out of a read entry, copied from a kept one
        let entry = Arc::try_unwrap(entry).unwrap_or_else(|kept| (*kept).clone());
        let mut group = Vec::with_capacity(entry.members.len());
        for (member, constants) in entry.members.into_iter().zip(constants) {
            let mut instructions = member.instructions;
            for inst in &mut instructions {
                if let Instruction::CallFn { func_id, .. } = inst {
                    *func_id = *id_map.get(&func_id.raw())?;
                }
            }
            group.push(Arc::new(CompiledBytecode {
                function_id: *id_map.get(&member.old_id)?,
                instructions,
                register_count: member.register_count,
                local_count: member.local_count,
                param_count: member.param_count,
                def_file: member.def_file.as_deref().map(Arc::from),
                param_names: member.param_names.into(),
                param_checks: member.param_checks.into(),
                return_check: member.return_check,
                constants,
                span_table: member.span_table,
                callee_names: member.callee_names,
                debug_info: super::BytecodeDebugInfo {
                    function_name: member.function_name,
                    ..Default::default()
                },
                optimization_level: member.optimization_level,
                entry_point: member.entry_point,
            }));
        }
        Some(group)
    }

    /// A constant looked up again where the compile looked it up.
    fn rebuild_constant(
        &self,
        src: &ConstSrc,
        new_ids: &HashMap<u64, FunctionId>,
        contexts: &mut HashMap<u64, Arc<im::HashMap<String, Value>>>,
    ) -> Option<OvmValue> {
        let c = &self.compiler;
        let value = match src {
            ConstSrc::Plain(p) => cc::value_of(p),
            ConstSrc::Ident(name) => match c.ident_value(name)? {
                Value::Function(ref f) => OvmValue::new_ast_function(f.clone()),
                v => OvmValue::from_ast(v),
            },
            ConstSrc::Callee(name) => match c.callee_in_scope(name)? {
                Value::Function(ref f) => OvmValue::new_ast_function(f.clone()),
                _ => return None,
            },
            ConstSrc::Known(name) => {
                OvmValue::from_ast(Value::Function(c.known_function_values.get(name)?.clone()))
            }
            ConstSrc::Lexical(name) => OvmValue::from_ast(c.lexical(name)?),
            ConstSrc::Lambda(src) => OvmValue::new_ast_function(Arc::new(c.lambda_function(src)?)),
            ConstSrc::Closure {
                src,
                capture_names,
                func_id,
            } => {
                let template = c.lambda_function(src)?;
                contexts.insert(*func_id, template.closure.clone());
                OvmValue::new_closure(Arc::new(crate::ovm::value::ClosureObject {
                    template,
                    capture_names: capture_names.clone(),
                    captured: Vec::new(),
                    func_id: *new_ids.get(func_id)?,
                    ast_closure: Default::default(),
                }))
            }
        };
        Some(super::InstructionEmitter::normalize_constant(value))
    }
}
