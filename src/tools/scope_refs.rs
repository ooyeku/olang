//! Names by binding, not spelling: which occurrences of a name in a
//! program read the same binding as the one at a place — what the
//! language server's references, highlights and rename follow.
//!
//! The program is read by the parser's grammar (every name with its
//! place) and walked as olang scopes it: a `let` binds after its value
//! (`let x = x + 1` reads the outer `x`), a function's and a lambda's
//! parameters, a `for`'s binding, a match arm's pattern and a block's
//! `let`s are their body's own and shadow what is outside; functions are
//! known throughout the block that declares them. A module's top-level
//! name is one name however many times it is bound (a notebook's cells
//! bind and read the module's names, a later cell's `let` included), and
//! a function's body reads the module's names whenever they are bound.
//! A name bound nowhere in the program (a builtin, a module) is one name
//! by its spelling. Text that does not parse answers nothing: the caller
//! falls back to words.

use crate::parser::{OlangParser, Rule};
use pest::Parser as _;
use pest::iterators::Pair;
use std::collections::HashMap;

/// One name's place: byte offsets in the program and its binding (`None`:
/// bound nowhere in the program).
#[derive(Debug, Clone)]
struct Occ {
    start: usize,
    end: usize,
    name: String,
    binding: Option<usize>,
}

/// What a name at a place reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub name: String,
    /// Every place reading the same binding: 0-based line, the start and
    /// end column in characters.
    pub places: Vec<(u32, usize, usize)>,
    /// A top-level name of the module (what a notebook's prose reads).
    pub top_level: bool,
    /// Bound in this program (a parameter, a `let`, a function, an import).
    pub bound: bool,
}

struct Walker<'a> {
    src: &'a str,
    base: usize,
    occs: Vec<Occ>,
    /// Each binding's name and whether it is the module's.
    binds: Vec<(String, bool)>,
    scopes: Vec<HashMap<String, usize>>,
    /// Names read in a function's body that nothing visible bound when it
    /// was read: the module's, if the module binds them later.
    pending: Vec<usize>,
    /// Inside a function's (or a lambda's) body.
    deferred: u32,
}

impl<'a> Walker<'a> {
    fn bind(&mut self, name: &str, start: usize, end: usize) {
        let top = self.scopes.len() == 1;
        let id = match self.scopes.last().and_then(|s| s.get(name)) {
            Some(id) if top => *id,
            _ => {
                self.binds.push((name.to_string(), top));
                self.binds.len() - 1
            }
        };
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string(), id);
        }
        self.occs.push(Occ {
            start: self.base + start,
            end: self.base + end,
            name: name.to_string(),
            binding: Some(id),
        });
    }

    fn lookup(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn use_name(&mut self, p: &Pair<Rule>) {
        let span = p.as_span();
        let name = p.as_str().to_string();
        let binding = self.lookup(&name);
        if binding.is_none() && self.deferred > 0 {
            self.pending.push(self.occs.len());
        }
        self.occs.push(Occ {
            start: self.base + span.start(),
            end: self.base + span.end(),
            name,
            binding,
        });
    }

    fn bind_pair(&mut self, p: &Pair<Rule>) {
        let span = p.as_span();
        self.bind(p.as_str(), span.start(), span.end());
    }

    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    /// A sequence of statements in a scope of its own (the program's, a
    /// block's): its functions known throughout it.
    fn statements<'p>(&mut self, items: impl Iterator<Item = Pair<'p, Rule>> + Clone) {
        for s in items.clone() {
            if let Some(f) = declared_fn(s) {
                let span = f.as_span();
                self.hoist(f.as_str(), span.start(), span.end());
            }
        }
        for s in items {
            self.walk(s);
        }
    }

    /// A function's name known before its declaration is read.
    fn hoist(&mut self, name: &str, _start: usize, _end: usize) {
        let top = self.scopes.len() == 1;
        if self.scopes.last().is_some_and(|s| s.contains_key(name)) {
            return;
        }
        self.binds.push((name.to_string(), top));
        let id = self.binds.len() - 1;
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string(), id);
        }
    }

    fn walk(&mut self, p: Pair<Rule>) {
        match p.as_rule() {
            Rule::program | Rule::block | Rule::test_block => {
                let top = p.as_rule() == Rule::program;
                if !top {
                    self.push();
                }
                let items = p.into_inner();
                self.statements(items);
                if !top {
                    self.pop();
                }
            }
            Rule::function_decl | Rule::trait_method => self.function(p),
            Rule::lambda => {
                self.push();
                self.deferred += 1;
                for c in p.into_inner() {
                    match c.as_rule() {
                        Rule::param_list => self.params(c),
                        Rule::type_annotation => {}
                        _ => self.walk(c),
                    }
                }
                self.deferred -= 1;
                self.pop();
            }
            Rule::let_decl => {
                let mut pattern = None;
                for c in p.into_inner() {
                    match c.as_rule() {
                        Rule::pattern => pattern = Some(c),
                        Rule::type_annotation | Rule::let_kw | Rule::mut_kw => {}
                        _ => self.walk(c),
                    }
                }
                if let Some(pat) = pattern {
                    self.pattern(pat);
                }
            }
            Rule::match_arm => {
                self.push();
                for c in p.into_inner() {
                    match c.as_rule() {
                        Rule::pattern => self.pattern(c),
                        _ => self.walk(c),
                    }
                }
                self.pop();
            }
            Rule::for_loop | Rule::par_for_loop => {
                let kids: Vec<Pair<Rule>> = p.into_inner().collect();
                // the iterable first, outside the loop's scope
                for c in &kids {
                    if c.as_rule() == Rule::expr {
                        self.walk(c.clone());
                    }
                }
                self.push();
                for c in &kids {
                    match c.as_rule() {
                        Rule::identifier => self.bind_pair(c),
                        Rule::for_binding_tuple => {
                            for i in c.clone().into_inner() {
                                if i.as_rule() == Rule::identifier {
                                    self.bind_pair(&i);
                                }
                            }
                        }
                        Rule::block => self.walk(c.clone()),
                        _ => {}
                    }
                }
                self.pop();
            }
            Rule::assignment_expr => {
                for c in p.into_inner() {
                    match c.as_rule() {
                        Rule::identifier => self.use_name(&c),
                        Rule::index_target => {
                            // `.field[…]`: the field is a key, the index an expression
                            for i in c.into_inner() {
                                if i.as_rule() != Rule::identifier {
                                    self.walk(i);
                                }
                            }
                        }
                        _ => self.walk(c),
                    }
                }
            }
            Rule::use_decl => {
                let mut path = None;
                let mut listed = false;
                for c in p.into_inner() {
                    match c.as_rule() {
                        Rule::use_path => path = Some(c),
                        Rule::use_list => {
                            listed = true;
                            for item in c.into_inner() {
                                let inner: Vec<Pair<Rule>> = item.into_inner().collect();
                                match inner.as_slice() {
                                    [one] if one.as_rule() == Rule::identifier => {
                                        self.bind_pair(one)
                                    }
                                    [alias] if alias.as_rule() == Rule::use_alias => {
                                        let names: Vec<Pair<Rule>> = alias
                                            .clone()
                                            .into_inner()
                                            .filter(|n| n.as_rule() == Rule::identifier)
                                            .collect();
                                        if let Some(last) = names.last() {
                                            self.bind_pair(last);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
                // `use module`: the module's name
                if !listed
                    && let Some(path) = path
                    && let Some(last) = path.into_inner().next_back()
                {
                    self.bind_pair(&last);
                }
            }
            Rule::identifier => self.use_name(&p),
            // a key, never a name read
            Rule::named_arg | Rule::field_value => {
                for c in p.into_inner() {
                    if c.as_rule() != Rule::identifier {
                        self.walk(c);
                    }
                }
            }
            Rule::struct_literal => {
                for c in p.into_inner() {
                    if !matches!(c.as_rule(), Rule::identifier | Rule::type_name_upper) {
                        self.walk(c);
                    }
                }
            }
            Rule::macro_call => {
                for c in p.into_inner() {
                    if c.as_rule() != Rule::identifier {
                        self.walk(c);
                    }
                }
            }
            Rule::template_string => {
                let span = p.as_span();
                self.template(span.as_str(), span.start());
            }
            // what reads no name
            Rule::type_annotation
            | Rule::type_decl
            | Rule::type_params
            | Rule::error_type_decl
            | Rule::decorator
            | Rule::string
            | Rule::raw_string
            | Rule::member_name
            | Rule::meta_fn_decl => {}
            Rule::impl_decl | Rule::trait_decl => {
                for c in p.into_inner() {
                    if c.as_rule() != Rule::identifier {
                        // a method's name is the type's, not the module's
                        self.push();
                        self.walk(c);
                        self.pop();
                    }
                }
            }
            _ => {
                for c in p.into_inner() {
                    self.walk(c);
                }
            }
        }
    }

    fn function(&mut self, p: Pair<Rule>) {
        let kids: Vec<Pair<Rule>> = p.into_inner().collect();
        // the name: known already when its block hoisted it
        if let Some(name) = kids.iter().find(|c| c.as_rule() == Rule::identifier) {
            let n = name.as_str();
            let span = name.as_span();
            match self.scopes.last().and_then(|s| s.get(n)).copied() {
                Some(id) => self.occs.push(Occ {
                    start: self.base + span.start(),
                    end: self.base + span.end(),
                    name: n.to_string(),
                    binding: Some(id),
                }),
                None => self.bind(n, span.start(), span.end()),
            }
        }
        self.push();
        self.deferred += 1;
        for c in kids {
            match c.as_rule() {
                Rule::param_list => self.params(c),
                Rule::expr | Rule::block => self.walk(c),
                _ => {}
            }
        }
        self.deferred -= 1;
        self.pop();
    }

    fn params(&mut self, p: Pair<Rule>) {
        for param in p.into_inner() {
            if param.as_rule() != Rule::parameter {
                continue;
            }
            let mut name = None;
            for c in param.into_inner() {
                match c.as_rule() {
                    Rule::identifier if name.is_none() => name = Some(c),
                    Rule::type_annotation => {}
                    // a default reads the parameters before it
                    _ => self.walk(c),
                }
            }
            if let Some(n) = name {
                self.bind_pair(&n);
            }
        }
    }

    /// The names a pattern binds: a lower-case name (a capitalised one is
    /// a variant), a field's shorthand, a rest's name; a struct's or a
    /// variant's own name binds nothing.
    fn pattern(&mut self, p: Pair<Rule>) {
        match p.as_rule() {
            Rule::identifier => {
                if p.as_str().chars().next().is_some_and(|c| !c.is_uppercase()) {
                    self.bind_pair(&p);
                }
            }
            Rule::struct_pattern | Rule::enum_variant_pattern => {
                let mut first = true;
                for c in p.into_inner() {
                    if first && c.as_rule() == Rule::identifier {
                        first = false;
                        continue;
                    }
                    first = false;
                    self.pattern(c);
                }
            }
            Rule::struct_pattern_field => {
                let kids: Vec<Pair<Rule>> = p.into_inner().collect();
                match kids.as_slice() {
                    [one] => self.pattern(one.clone()),
                    [_, rest @ ..] => {
                        for c in rest {
                            self.pattern(c.clone());
                        }
                    }
                    [] => {}
                }
            }
            Rule::rest_pattern => {
                for c in p.into_inner() {
                    if c.as_rule() == Rule::identifier {
                        self.bind_pair(&c);
                    }
                }
            }
            Rule::string | Rule::integer | Rule::boolean | Rule::char_literal => {}
            _ => {
                for c in p.into_inner() {
                    self.pattern(c);
                }
            }
        }
    }

    /// A template's `${expr}`s, each read as an expression at its place.
    fn template(&mut self, text: &str, at: usize) {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'$' && bytes[i + 1] == b'{' {
                let mut depth = 0i32;
                let mut j = i + 1;
                let mut end = None;
                while j < bytes.len() {
                    match bytes[j] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(j);
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let Some(end) = end else { return };
                let inner = &text[i + 2..end];
                if let Ok(mut pairs) = OlangParser::parse(Rule::expr, inner) {
                    let saved = self.base;
                    self.base = saved + at + i + 2;
                    if let Some(e) = pairs.next() {
                        self.walk(e);
                    }
                    self.base = saved;
                }
                i = end + 1;
                continue;
            }
            i += 1;
        }
    }
}

/// The function a statement declares (its name), when it declares one.
fn declared_fn(s: Pair<Rule>) -> Option<Pair<Rule>> {
    let mut p = s;
    loop {
        match p.as_rule() {
            Rule::function_decl => {
                return p.into_inner().find(|c| c.as_rule() == Rule::identifier);
            }
            Rule::statement | Rule::share_decl | Rule::decorated_decl | Rule::test_statement => {
                p = p
                    .into_inner()
                    .find(|c| !matches!(c.as_rule(), Rule::share_kw | Rule::decorator))?;
            }
            _ => return None,
        }
    }
}

/// Every name's place in `src` with its binding, or `None` when it does
/// not parse.
/// Each binding's name and whether it is the module's.
type Binds = Vec<(String, bool)>;

fn occurrences(src: &str) -> Option<(Vec<Occ>, Binds)> {
    let program = OlangParser::parse(Rule::program, src).ok()?.next()?;
    let mut w = Walker {
        src,
        base: 0,
        occs: Vec::new(),
        binds: Vec::new(),
        scopes: vec![HashMap::new()],
        pending: Vec::new(),
        deferred: 0,
    };
    w.walk(program);
    // a body's name the module binds later is the module's
    let top = w.scopes.first().cloned().unwrap_or_default();
    for i in std::mem::take(&mut w.pending) {
        if let Some(id) = top.get(&w.occs[i].name) {
            w.occs[i].binding = Some(*id);
        }
    }
    let _ = w.src;
    Some((w.occs, w.binds))
}

/// The byte offset of 0-based `line`, character `col` in `src`.
fn offset_of(src: &str, line: u32, col: usize) -> Option<usize> {
    let mut at = 0;
    for (i, l) in src.split('\n').enumerate() {
        if i as u32 == line {
            let off = l.char_indices().nth(col).map(|(b, _)| b).unwrap_or(l.len());
            return Some(at + off);
        }
        at += l.len() + 1;
    }
    None
}

/// 0-based line and character column of byte `at`.
fn place_of(src: &str, at: usize) -> (u32, usize) {
    let before = &src[..at.min(src.len())];
    let line = before.matches('\n').count() as u32;
    let start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    (line, before[start..].chars().count())
}

/// What the name at 0-based `line`, character `col` of `src` reads, and
/// every place reading the same: `None` when the text does not parse or
/// no name is there.
pub fn resolve_at(src: &str, line: u32, col: usize) -> Option<Resolved> {
    let (occs, binds) = occurrences(src)?;
    let at = offset_of(src, line, col)?;
    let hit = occs.iter().find(|o| o.start <= at && at <= o.end)?;
    Some(resolved(src, &occs, &binds, hit))
}

/// What the module's top-level `name` is read as: every place reading
/// the module's binding of it (`None` when it does not parse or the
/// module binds no such name).
pub fn resolve_top(src: &str, name: &str) -> Option<Resolved> {
    let (occs, binds) = occurrences(src)?;
    let hit = occs
        .iter()
        .find(|o| o.name == name && o.binding.is_some_and(|b| binds[b].1))?;
    Some(resolved(src, &occs, &binds, hit))
}

fn resolved(src: &str, occs: &[Occ], binds: &[(String, bool)], hit: &Occ) -> Resolved {
    let places = occs
        .iter()
        .filter(|o| match hit.binding {
            Some(b) => o.binding == Some(b),
            None => o.binding.is_none() && o.name == hit.name,
        })
        .map(|o| {
            let (l, c) = place_of(src, o.start);
            (l, c, c + o.name.chars().count())
        })
        .collect();
    Resolved {
        name: hit.name.clone(),
        places,
        top_level: hit.binding.is_none_or(|b| binds[b].1),
        bound: hit.binding.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(r: &Resolved) -> Vec<(u32, usize)> {
        r.places.iter().map(|p| (p.0, p.1)).collect()
    }

    #[test]
    fn a_parameter_shadows_the_module_s_name() {
        let src = "let n = 3\nfn f(n) = n + 1\nlet m = n * 2\n";
        let outer = resolve_at(src, 0, 4).unwrap();
        assert_eq!(cols(&outer), vec![(0, 4), (2, 8)]);
        assert!(outer.top_level);
        let inner = resolve_at(src, 1, 10).unwrap();
        assert_eq!(cols(&inner), vec![(1, 5), (1, 10)]);
        assert!(!inner.top_level);
    }

    #[test]
    fn a_let_reads_the_outer_name_before_it_binds_its_own() {
        let src = "let x = 1\nlet g = () => {\n    let x = x + 1\n    x\n}\nx\n";
        // the inner x's value reads the outer one
        assert_eq!(
            cols(&resolve_at(src, 0, 4).unwrap()),
            vec![(0, 4), (2, 12), (5, 0)]
        );
        assert_eq!(cols(&resolve_at(src, 3, 4).unwrap()), vec![(2, 8), (3, 4)]);
    }

    #[test]
    fn lambdas_loops_and_match_arms_bind_their_own() {
        let src = "let total = 5\nlet add = (total) => total + 1\nfor total in [1] { println(total) }\nlet r = match 2 { total => total }\nprintln(total)\n";
        assert_eq!(cols(&resolve_at(src, 0, 4).unwrap()), vec![(0, 4), (4, 8)]);
        assert_eq!(
            cols(&resolve_at(src, 1, 21).unwrap()),
            vec![(1, 11), (1, 21)]
        );
        assert_eq!(cols(&resolve_at(src, 2, 4).unwrap()), vec![(2, 4), (2, 27)]);
        assert_eq!(
            cols(&resolve_at(src, 3, 27).unwrap()),
            vec![(3, 18), (3, 27)]
        );
    }

    #[test]
    fn a_function_reads_the_module_s_names_bound_after_it_and_keys_are_not_names() {
        let src = "fn show_it() = limit + 1\nlet limit = 4\nlet r = #{ \"limit\": limit }\nlet o = { limit: limit }\nlet s = `at ${limit}`\no.limit\n";
        assert_eq!(
            cols(&resolve_at(src, 1, 4).unwrap()),
            vec![(0, 15), (1, 4), (2, 20), (3, 17), (4, 14)]
        );
        // a builtin is one name by its spelling
        let b = resolve_at("println(1)\nprintln(2)\n", 0, 2).unwrap();
        assert!(!b.bound);
        assert_eq!(b.places.len(), 2);
    }

    #[test]
    fn text_that_does_not_parse_answers_nothing() {
        assert_eq!(resolve_at("let x = (\n", 0, 4), None);
    }
}
