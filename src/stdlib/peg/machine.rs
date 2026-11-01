//! The matcher: recursive descent over the flattened grammar, in the
//! three families `peg.ol` writes for every rule — building (`b`),
//! recognising (`k`) and recognising while noting failures (`tk`) — as
//! two const parameters, so each family is its own compiled code with
//! the tests of the others folded away.
//!
//! What it shares with the olang engine, exactly: where skip runs (a
//! lower-case rule's elements skip unless the position is already past
//! it; tokens and `skip` never), what an ordered choice tries when it can
//! look at the next character first, the one-entry memo of a rule tried
//! twice at a place (a slot pair a rule, its recognising half shared by
//! `k` and `tk`), seed growing for left recursion, the precedence
//! climber, and the bookkeeping of the noting family — the furthest
//! failure, what was expected there, a token or a described rule said as
//! itself where it could not begin, and the piece matched before each
//! position — so a failed parse says what the olang engine says.
//!
//! Nodes are built compact (kind, span, children in a shared store,
//! labels) into an arena; backtracking truncates a stack of children,
//! which frees and allocates nothing. Only the final tree becomes olang
//! values (values.rs).

use super::grammar::{Expr, Fix, Grammar, Id, NONE};

/// A child: a node, and the label it carries (NONE for none) — `peg.ol`'s
/// `(name, node)` tuples.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Child {
    pub node: u32,
    pub label: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Extra {
    None,
    /// Index into `Machine::fields`.
    Fields(u32),
    Prec {
        op_start: i64,
        op_end: i64,
        fix: Fix,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Node {
    /// NONE for a node without a kind (a silent rule's).
    pub kind: u32,
    pub start: i64,
    pub end: i64,
    pub kids: u32,
    pub nkids: u32,
    pub extra: Extra,
}

#[derive(Clone, Debug)]
pub(crate) enum Field {
    One(u32),
    Many(Vec<u32>),
}

/// The context of an expression: whether it skips between elements (a
/// lower-case rule), whether it is inside a token (where rule calls only
/// recognise and successes are not noted), and in a noting token the
/// position it began at (its own failures there are not noted).
#[derive(Clone, Copy)]
struct Cx {
    ws: bool,
    inner: bool,
    tok: i64,
}

const CX_RULE: Cx = Cx {
    ws: true,
    inner: false,
    tok: -1,
};
const CX_TOKEN: Cx = Cx {
    ws: false,
    inner: true,
    tok: -1,
};

/// How deep rule calls may nest before a parse gives up: the
/// interpreter's own limit on calls (its stack grows past the thread's
/// as it goes, `with_stack_headroom`), and in the browser, whose stack
/// does not grow, a bound that fits it.
#[cfg(feature = "native")]
pub(crate) const MAX_DEPTH: u32 = 100_000;
#[cfg(not(feature = "native"))]
pub(crate) const MAX_DEPTH: u32 = 1_000;

pub(crate) struct Machine<'a> {
    pub g: &'a Grammar,
    /// The input's code points, -1 after the last.
    pub cs: &'a [i32],
    pub n: i64,
    stack: Vec<Child>,
    pub nodes: Vec<Node>,
    pub store: Vec<Child>,
    pub fields: Vec<Vec<(u32, Field)>>,
    /// (position, answer) a slot; position -1 is empty.
    cache: Vec<(i64, i64)>,
    /// (slot, position, seed) of the growths under way.
    seeds: Vec<(u32, i64, i64)>,
    pub far: i64,
    pub exp: Vec<u32>,
    pub oks: Vec<i64>,
    depth: u32,
    pub too_deep: bool,
}

impl<'a> Machine<'a> {
    pub fn new(g: &'a Grammar, cs: &'a [i32]) -> Machine<'a> {
        Machine {
            g,
            cs,
            n: cs.len() as i64 - 1,
            stack: Vec::new(),
            nodes: Vec::new(),
            store: Vec::new(),
            fields: Vec::new(),
            cache: vec![(-1, -1); g.nslots],
            seeds: Vec::new(),
            far: -1,
            exp: Vec::new(),
            oks: Vec::new(),
            depth: 0,
            too_deep: false,
        }
    }

    /// A fresh parse's state (the arena stays: its nodes are still read).
    pub fn reset(&mut self, track: bool) {
        self.cache.iter_mut().for_each(|c| *c = (-1, -1));
        self.seeds.clear();
        self.stack.clear();
        self.far = -1;
        self.exp.clear();
        self.oks.clear();
        if track {
            self.oks.resize(self.cs.len(), -1);
        }
    }

    #[inline(always)]
    fn at(&self, p: i64) -> i32 {
        // p never passes n: every test stops at the -1 there
        self.cs[p as usize]
    }

    // ── skip ───────────────────────────────────────────────────────────

    /// Past whatever `skip` matches at `p` (repeating it while it moves).
    #[inline]
    pub fn skip(&mut self, p: i64) -> i64 {
        let g = self.g;
        match (&g.skip_class, g.skip) {
            (Some(set), _) => {
                let mut q = p;
                while set.has(self.at(q)) {
                    q += 1;
                }
                q
            }
            (None, Some(r)) => {
                let mut q = p;
                loop {
                    let r = self.rule_k::<false>(r, q);
                    if r <= q {
                        break;
                    }
                    q = r;
                }
                q
            }
            (None, None) => p,
        }
    }

    #[inline(always)]
    fn at_pos(&mut self, cx: Cx, p: i64, skd: bool) -> i64 {
        if cx.ws && !skd && self.g.skip.is_some() {
            self.skip(p)
        } else {
            p
        }
    }

    #[inline(always)]
    fn sk(&mut self, cx: Cx, v: i64) -> i64 {
        if cx.ws && self.g.skip.is_some() {
            self.skip(v)
        } else {
            v
        }
    }

    // ── noting failures ────────────────────────────────────────────────

    fn fail(&mut self, x: i64, i: u32) {
        if x > self.far {
            self.far = x;
            self.exp.clear();
            self.exp.push(i);
        } else if x == self.far && !self.exp.contains(&i) {
            self.exp.push(i);
        }
    }

    #[inline]
    fn fail_cx(&mut self, cx: Cx, x: i64, i: u32) {
        if cx.tok >= 0 {
            if x > cx.tok {
                self.fail(x, i);
            }
        } else {
            self.fail(x, i);
        }
    }

    #[inline]
    fn okk(&mut self, a: i64, b: i64) -> i64 {
        let slot = &mut self.oks[b as usize];
        if *slot < 0 {
            *slot = a;
        }
        b
    }

    /// A token, or a rule with words of its own, ends: failing where it
    /// began, its words replace what its insides expected there; a
    /// token that matched forgets what its insides noted.
    fn mute_end(&mut self, saved: (i64, Vec<u32>), p: i64, r: i64, i: u32, tok: bool) -> i64 {
        if r < 0 {
            if self.far == p {
                let mut before = if saved.0 == p { saved.1 } else { Vec::new() };
                if !before.contains(&i) {
                    before.push(i);
                }
                self.far = p;
                self.exp = before;
            } else if self.far < p {
                self.fail(p, i);
            }
        } else if tok || self.far == p {
            self.far = saved.0;
            self.exp = saved.1;
        }
        r
    }

    // ── nodes ──────────────────────────────────────────────────────────

    fn node(&mut self, kind: u32, start: i64, end: i64, base: usize, extra: Extra) -> u32 {
        let kids = self.store.len() as u32;
        self.store.extend_from_slice(&self.stack[base..]);
        self.nodes.push(Node {
            kind,
            start,
            end,
            kids,
            nkids: (self.stack.len() - base) as u32,
            extra,
        });
        (self.nodes.len() - 1) as u32
    }

    fn leaf(&mut self, kind: u32, start: i64, end: i64) -> u32 {
        self.nodes.push(Node {
            kind,
            start,
            end,
            kids: self.store.len() as u32,
            nkids: 0,
            extra: Extra::None,
        });
        (self.nodes.len() - 1) as u32
    }

    /// A node whose labelled children become its fields (`pg_lnode`).
    fn lnode(&mut self, kind: u32, start: i64, end: i64, base: usize, lists: &[u32]) -> u32 {
        let kids = self.store.len() as u32;
        let mut fields: Vec<(u32, Field)> = lists
            .iter()
            .map(|&l| (l, Field::Many(Vec::new())))
            .collect();
        for (i, k) in (base..self.stack.len()).enumerate() {
            let c = self.stack[k];
            self.store.push(Child {
                node: c.node,
                label: NONE,
            });
            if c.label != NONE {
                match fields.iter_mut().find(|f| f.0 == c.label) {
                    Some((_, Field::Many(v))) => v.push(i as u32),
                    Some((_, one)) => *one = Field::One(i as u32),
                    None => fields.push((c.label, Field::One(i as u32))),
                }
            }
        }
        self.fields.push(fields);
        self.nodes.push(Node {
            kind,
            start,
            end,
            kids,
            nkids: (self.stack.len() - base) as u32,
            extra: Extra::Fields((self.fields.len() - 1) as u32),
        });
        (self.nodes.len() - 1) as u32
    }

    #[inline(always)]
    pub fn end_of(&self, n: i64) -> i64 {
        self.nodes[n as usize].end
    }

    #[inline(always)]
    fn push(&mut self, node: u32, label: u32) {
        self.stack.push(Child { node, label });
    }

    /// The children from `base` as one node (`pg_grp`): the only child
    /// itself, or a node of the table's kind holding them.
    fn grp(&mut self, base: usize, kind: u32, p: i64) -> Child {
        if self.stack.len() - base == 1 {
            self.stack[base]
        } else {
            let e = if self.stack.len() == base {
                p
            } else {
                self.end_of(self.stack[self.stack.len() - 1].node as i64)
            };
            let n = self.node(kind, p, e, base, Extra::None);
            Child {
                node: n,
                label: NONE,
            }
        }
    }

    // ── rules ──────────────────────────────────────────────────────────

    #[inline]
    fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.too_deep = true;
        }
        !self.too_deep
    }

    /// A rule, recognising (`T` false: `k_`) or noting (`T`: `tk_`): the
    /// end of its match, or -1.
    pub fn rule_k<const T: bool>(&mut self, r: Id, p: i64) -> i64 {
        if !self.enter() {
            self.depth -= 1;
            return -1;
        }
        let out = if self.depth % 64 == 0 {
            crate::interpreter::with_stack_headroom(|| self.rule_k_wrapped::<T>(r, p))
        } else {
            self.rule_k_wrapped::<T>(r, p)
        };
        self.depth -= 1;
        out
    }

    fn rule_k_wrapped<const T: bool>(&mut self, r: Id, p: i64) -> i64 {
        let rule = &self.g.rules[r as usize];
        if rule.is_skip {
            // `tk_skip` is `k_skip`
            return self.ev::<false, false>(rule.body, p, true, CX_TOKEN);
        }
        match rule.slot {
            None => self.rule_k_body::<T>(r, p),
            Some(s) => {
                let slot = s * 2 + 1;
                if rule.lr_head {
                    self.grow_k::<T>(slot, r, p)
                } else {
                    let (cp, cv) = self.cache[slot as usize];
                    if cp == p {
                        return cv;
                    }
                    let v = self.rule_k_body::<T>(r, p);
                    self.cache[slot as usize] = (p, v);
                    v
                }
            }
        }
    }

    fn rule_k_body<const T: bool>(&mut self, r: Id, p: i64) -> i64 {
        let rule = &self.g.rules[r as usize];
        if rule.token {
            if !T {
                self.ev::<false, false>(rule.body, p, true, CX_TOKEN)
            } else {
                let saved = (self.far, self.exp.clone());
                let cx = Cx {
                    ws: false,
                    inner: true,
                    tok: p,
                };
                let res = self.ev::<false, true>(rule.body, p, true, cx);
                let res = self.mute_end(saved, p, res, rule.desc, true);
                if res < 0 { -1 } else { self.okk(p, res) }
            }
        } else if T && rule.has_desc {
            let saved = (self.far, self.exp.clone());
            let res = self.ev::<false, true>(rule.body, p, true, CX_RULE);
            self.mute_end(saved, p, res, rule.desc, false)
        } else {
            self.ev::<false, T>(rule.body, p, true, CX_RULE)
        }
    }

    fn seed_find(&self, slot: u32, p: i64) -> Option<i64> {
        self.seeds
            .iter()
            .rev()
            .find(|e| e.0 == slot && e.1 == p)
            .map(|e| e.2)
    }

    fn seed_set(&mut self, slot: u32, p: i64, v: i64) {
        self.seeds.retain(|e| !(e.0 == slot && e.1 == p));
        self.seeds.push((slot, p, v));
    }

    fn seed_drop(&mut self, slot: u32, p: i64) {
        self.seeds.retain(|e| !(e.0 == slot && e.1 == p));
    }

    /// Left recursion, recognising: fail at first, then run the body
    /// again and again, each time letting the recursive call see the last
    /// answer, while the match grows.
    fn grow_k<const T: bool>(&mut self, slot: u32, r: Id, p: i64) -> i64 {
        if let Some(v) = self.seed_find(slot, p) {
            return v;
        }
        self.seed_set(slot, p, -1);
        let mut best = -1;
        loop {
            let v = self.rule_k_body::<T>(r, p);
            if v <= best || self.too_deep {
                break;
            }
            best = v;
            self.seed_set(slot, p, v);
        }
        self.seed_drop(slot, p);
        best
    }

    /// A rule, building: its node, or -1.
    pub fn rule_b(&mut self, r: Id, p: i64) -> i64 {
        if !self.enter() {
            self.depth -= 1;
            return -1;
        }
        let out = if self.depth % 64 == 0 {
            crate::interpreter::with_stack_headroom(|| self.rule_b_wrapped(r, p))
        } else {
            self.rule_b_wrapped(r, p)
        };
        self.depth -= 1;
        out
    }

    fn rule_b_wrapped(&mut self, r: Id, p: i64) -> i64 {
        let rule = &self.g.rules[r as usize];
        if rule.is_skip || rule.token {
            let kind = if rule.is_skip || rule.silent {
                NONE
            } else {
                rule.kind
            };
            let v = self.rule_k::<false>(r, p);
            return if v < 0 {
                -1
            } else {
                self.leaf(kind, p, v) as i64
            };
        }
        match rule.slot {
            None => self.rule_b_body(r, p),
            Some(s) => {
                let slot = s * 2;
                if rule.lr_head {
                    if let Some(v) = self.seed_find(slot, p) {
                        return v;
                    }
                    self.seed_set(slot, p, -1);
                    let mut best = -1;
                    let mut bend = -1;
                    loop {
                        let nd = self.rule_b_body(r, p);
                        if nd < 0 || self.too_deep {
                            break;
                        }
                        let e = self.end_of(nd);
                        if e <= bend {
                            break;
                        }
                        best = nd;
                        bend = e;
                        self.seed_set(slot, p, nd);
                    }
                    self.seed_drop(slot, p);
                    best
                } else {
                    let (cp, cv) = self.cache[slot as usize];
                    if cp == p {
                        return cv;
                    }
                    let v = self.rule_b_body(r, p);
                    self.cache[slot as usize] = (p, v);
                    v
                }
            }
        }
    }

    fn rule_b_body(&mut self, r: Id, p: i64) -> i64 {
        let g = self.g;
        let rule = &g.rules[r as usize];
        if let Some(pi) = rule.prec {
            return self.prec_b(pi, p, 0, CX_RULE);
        }
        let base = self.stack.len();
        let res = self.ev::<true, false>(rule.body, p, true, CX_RULE);
        let out = if res < 0 {
            -1
        } else if let Some(lists) = &rule.labels {
            self.lnode(rule.kind, p, res, base, lists) as i64
        } else {
            let kind = if rule.silent { NONE } else { rule.kind };
            self.node(kind, p, res, base, Extra::None) as i64
        };
        self.stack.truncate(base);
        out
    }

    // ── expressions ────────────────────────────────────────────────────

    #[inline]
    fn call<const B: bool, const T: bool>(&mut self, r: Id, x: i64, cx: Cx) -> i64 {
        if T && !cx.inner {
            self.rule_k::<true>(r, x)
        } else {
            self.rule_k::<false>(r, x)
        }
    }

    /// The end of `e`'s match from `p`, or -1. Building, its nodes are
    /// pushed on the stack of children.
    fn ev<const B: bool, const T: bool>(&mut self, e: Id, p: i64, skd: bool, cx: Cx) -> i64 {
        let g = self.g;
        match &g.exprs[e as usize] {
            Expr::Empty => p,
            Expr::Lit { codes, ci, desc } => {
                let x = self.at_pos(cx, p, skd);
                let mut ok = true;
                let mut q = x as usize;
                for &c in codes.iter() {
                    let have = self.cs[q];
                    let have = if *ci {
                        super::grammar::lower(have)
                    } else {
                        have
                    };
                    if have != c {
                        ok = false;
                        break;
                    }
                    q += 1;
                }
                if ok {
                    let end = x + codes.len() as i64;
                    if T && !cx.inner {
                        self.okk(x, end)
                    } else {
                        end
                    }
                } else {
                    if T {
                        self.fail_cx(cx, x, *desc);
                    }
                    -1
                }
            }
            Expr::Class { set, desc } => {
                let x = self.at_pos(cx, p, skd);
                if set.has(self.at(x)) {
                    if T && !cx.inner {
                        self.okk(x, x + 1)
                    } else {
                        x + 1
                    }
                } else {
                    if T {
                        self.fail_cx(cx, x, *desc);
                    }
                    -1
                }
            }
            Expr::Any => {
                let x = self.at_pos(cx, p, skd);
                if self.at(x) >= 0 {
                    if T && !cx.inner {
                        self.okk(x, x + 1)
                    } else {
                        x + 1
                    }
                } else {
                    if T {
                        let i = g.any_desc;
                        self.fail_cx(cx, x, i);
                    }
                    -1
                }
            }
            Expr::Ref(r) => {
                let x = self.at_pos(cx, p, skd);
                if !B {
                    return self.call::<B, T>(*r, x, cx);
                }
                let rule = &g.rules[*r as usize];
                if rule.token {
                    let v = self.rule_k::<false>(*r, x);
                    if rule.silent || v < 0 {
                        return v;
                    }
                    let leaf = self.leaf(rule.kind, x, v);
                    self.push(leaf, NONE);
                    v
                } else {
                    let n = self.rule_b(*r, x);
                    if n < 0 {
                        return -1;
                    }
                    let nd = self.nodes[n as usize];
                    if rule.silent && !rule.is_skip {
                        for k in nd.kids..nd.kids + nd.nkids {
                            let c = self.store[k as usize];
                            self.stack.push(c);
                        }
                    } else {
                        self.push(n as u32, NONE);
                    }
                    nd.end
                }
            }
            Expr::Seq(items) => {
                let mark = self.stack.len();
                let mut q = p;
                for (i, it) in items.iter().enumerate() {
                    q = self.ev::<B, T>(*it, q, i == 0 && skd, cx);
                    if q < 0 {
                        if B {
                            self.stack.truncate(mark);
                        }
                        return -1;
                    }
                }
                q
            }
            Expr::Alt { items, guards } => {
                if !T && let Some(guards) = guards {
                    let x = self.at_pos(cx, p, skd);
                    let c = self.at(x);
                    for (it, gd) in items.iter().zip(guards.iter()) {
                        if let Some(set) = gd
                            && !set.has(c)
                        {
                            continue;
                        }
                        let r = self.ev::<B, T>(*it, x, true, cx);
                        if r >= 0 {
                            return r;
                        }
                    }
                    -1
                } else {
                    for it in items.iter() {
                        let r = self.ev::<B, T>(*it, p, skd, cx);
                        if r >= 0 {
                            return r;
                        }
                    }
                    -1
                }
            }
            Expr::Star(x) => self.star::<B, T>(*x, p, cx),
            Expr::StarClass { set, desc } => {
                let mut v = p;
                while set.has(self.at(v)) {
                    v += 1;
                }
                if T {
                    self.fail_cx(cx, v, *desc);
                }
                v
            }
            Expr::StarLead {
                lead,
                rest,
                general,
            } => {
                if T {
                    return self.star::<B, T>(*general, p, cx);
                }
                let mut v = p;
                loop {
                    if lead.has(self.at(v)) {
                        v += 1;
                        continue;
                    }
                    let r = self.ev::<B, T>(*rest, v, false, cx);
                    if r <= v {
                        break;
                    }
                    v = r;
                }
                v
            }
            Expr::Plus(x) => {
                let f = self.ev::<B, T>(*x, p, skd, cx);
                if f < 0 {
                    -1
                } else {
                    self.star::<B, T>(*x, f, cx)
                }
            }
            Expr::Opt(x) => {
                let r = self.ev::<B, T>(*x, p, skd, cx);
                if r >= 0 { r } else { p }
            }
            Expr::And(x) => {
                let r = self.ev::<false, false>(*x, p, skd, cx);
                if r >= 0 { p } else { -1 }
            }
            Expr::Not { e: x, any } => {
                let r = self.ev::<false, false>(*x, p, skd, cx);
                if r >= 0 {
                    if T && *any {
                        let at = self.at_pos(cx, p, skd);
                        let end = g.end_desc;
                        self.fail(at, end);
                    }
                    -1
                } else {
                    p
                }
            }
            Expr::Label {
                kind,
                e: inner,
                wraps,
                lists,
            } => {
                if !B {
                    return self.ev::<B, T>(*inner, p, skd, cx);
                }
                let x = self.at_pos(cx, p, skd);
                let m = self.stack.len();
                let r = self.ev::<B, T>(*inner, x, true, cx);
                if r < 0 {
                    return -1;
                }
                if *wraps {
                    for c in &mut self.stack[m..] {
                        if c.label == NONE {
                            c.label = *kind;
                        }
                    }
                } else if r > x || self.stack.len() > m {
                    let n = self.lnode(*kind, x, r, m, lists);
                    self.stack.truncate(m);
                    self.push(n, *kind);
                }
                r
            }
            Expr::Prec(pi) => {
                let x = self.at_pos(cx, p, skd);
                if B {
                    let n = self.prec_b(*pi, x, 0, cx);
                    if n < 0 {
                        -1
                    } else {
                        self.push(n as u32, NONE);
                        self.end_of(n)
                    }
                } else {
                    self.prec_k::<T>(*pi, x, 0, cx)
                }
            }
        }
    }

    #[inline]
    fn star<const B: bool, const T: bool>(&mut self, x: Id, p: i64, cx: Cx) -> i64 {
        let mut v = p;
        loop {
            let r = self.ev::<B, T>(x, v, false, cx);
            if r <= v {
                break;
            }
            v = r;
        }
        v
    }

    // ── precedence tables ──────────────────────────────────────────────

    /// A table, building, from `p` with operators at `ml` or tighter: the
    /// node (the operand alone, or an operator's node), or -1.
    fn prec_b(&mut self, pi: Id, p: i64, ml: u32, cx: Cx) -> i64 {
        if !self.enter() {
            self.depth -= 1;
            return -1;
        }
        let g = self.g;
        let pr = &g.precs[pi as usize];
        let kind = pr.kind;
        let base = self.stack.len();
        let mut q = -1;
        let mut pl: i64 = -1;
        let mut pe = -1;
        for op in pr.ops.iter().filter(|o| o.fix == Fix::Prefix) {
            if pl < 0 {
                let r = self.ev::<false, false>(op.e, p, true, cx);
                if r >= 0 {
                    pl = op.lev as i64;
                    pe = r;
                }
            }
        }
        if pl >= 0 {
            let from = self.sk(cx, pe);
            let o = self.prec_b(pi, from, pl as u32, cx);
            if o >= 0 {
                let oe = self.end_of(o);
                self.push(o as u32, NONE);
                let nn = self.node(
                    kind,
                    p,
                    oe,
                    self.stack.len() - 1,
                    Extra::Prec {
                        op_start: p,
                        op_end: pe,
                        fix: Fix::Prefix,
                    },
                );
                self.stack.pop();
                self.push(nn, NONE);
                q = oe;
            }
        } else {
            q = self.ev::<true, false>(pr.atom, p, true, cx);
        }
        if q >= 0 {
            'outer: loop {
                let x = self.sk(cx, q);
                for op in pr.ops.iter().filter(|o| o.fix == Fix::Postfix) {
                    if ml <= op.lev {
                        let r = self.ev::<false, false>(op.e, x, true, cx);
                        if r >= 0 {
                            let a = self.grp(base, kind, p);
                            self.stack.truncate(base);
                            self.stack.push(a);
                            let nn = self.node(
                                kind,
                                p,
                                r,
                                base,
                                Extra::Prec {
                                    op_start: x,
                                    op_end: r,
                                    fix: Fix::Postfix,
                                },
                            );
                            self.stack.truncate(base);
                            self.push(nn, NONE);
                            q = r;
                            continue 'outer;
                        }
                    }
                }
                for op in pr
                    .ops
                    .iter()
                    .filter(|o| o.fix == Fix::Left || o.fix == Fix::Right)
                {
                    if ml <= op.lev {
                        let r = self.ev::<false, false>(op.e, x, true, cx);
                        if r >= 0 {
                            let next = if op.fix == Fix::Left {
                                op.lev + 1
                            } else {
                                op.lev
                            };
                            let from = self.sk(cx, r);
                            let o = self.prec_b(pi, from, next, cx);
                            if o >= 0 {
                                let oe = self.end_of(o);
                                let a = self.grp(base, kind, p);
                                self.stack.truncate(base);
                                self.stack.push(a);
                                self.push(o as u32, NONE);
                                let nn = self.node(
                                    kind,
                                    p,
                                    oe,
                                    base,
                                    Extra::Prec {
                                        op_start: x,
                                        op_end: r,
                                        fix: Fix::Left,
                                    },
                                );
                                self.stack.truncate(base);
                                self.push(nn, NONE);
                                q = oe;
                                continue 'outer;
                            }
                        }
                    }
                }
                break;
            }
        }
        let out = if q < 0 {
            -1
        } else {
            self.grp(base, kind, p).node as i64
        };
        self.stack.truncate(base);
        self.depth -= 1;
        out
    }

    /// A table, recognising or noting: the end, or -1.
    fn prec_k<const T: bool>(&mut self, pi: Id, p: i64, ml: u32, cx: Cx) -> i64 {
        if !self.enter() {
            self.depth -= 1;
            return -1;
        }
        let g = self.g;
        let pr = &g.precs[pi as usize];
        let mut q;
        let mut pl: i64 = -1;
        let mut pe = -1;
        for op in pr.ops.iter().filter(|o| o.fix == Fix::Prefix) {
            if pl < 0 {
                let r = self.ev::<false, T>(op.e, p, true, cx);
                if r >= 0 {
                    pl = op.lev as i64;
                    pe = r;
                }
            }
        }
        if pl >= 0 {
            let from = self.sk(cx, pe);
            q = self.prec_k::<T>(pi, from, pl as u32, cx);
        } else {
            q = self.ev::<false, T>(pr.atom, p, true, cx);
        }
        if q >= 0 {
            'outer: loop {
                let x = self.sk(cx, q);
                for op in pr.ops.iter().filter(|o| o.fix == Fix::Postfix) {
                    if ml <= op.lev {
                        let r = self.ev::<false, T>(op.e, x, true, cx);
                        if r >= 0 {
                            q = r;
                            continue 'outer;
                        }
                    }
                }
                for op in pr
                    .ops
                    .iter()
                    .filter(|o| o.fix == Fix::Left || o.fix == Fix::Right)
                {
                    if ml <= op.lev {
                        let r = self.ev::<false, T>(op.e, x, true, cx);
                        if r >= 0 {
                            let next = if op.fix == Fix::Left {
                                op.lev + 1
                            } else {
                                op.lev
                            };
                            let from = self.sk(cx, r);
                            let o = self.prec_k::<T>(pi, from, next, cx);
                            if o >= 0 {
                                q = o;
                                continue 'outer;
                            }
                        }
                    }
                }
                break;
            }
        }
        self.depth -= 1;
        q
    }

    // ── a whole parse ──────────────────────────────────────────────────

    /// Past `skip` from `p`, where the grammar has one.
    pub fn skip_at(&mut self, p: i64) -> i64 {
        if self.g.skip.is_some() {
            self.skip(p)
        } else {
            p
        }
    }

    /// The start rule's node over the whole input (skip around it), or
    /// None.
    pub fn build(&mut self, start: Id, p: i64) -> Option<u32> {
        self.reset(false);
        let from = self.skip_at(p);
        let nd = self.rule_b(start, from);
        if nd >= 0 {
            let e = self.end_of(nd);
            if self.skip_at(e) == self.n {
                return Some(nd as u32);
            }
        }
        None
    }

    /// Whether the whole input matches the start rule.
    pub fn accepts(&mut self, start: Id) -> bool {
        self.reset(false);
        let from = self.skip_at(0);
        let r = self.rule_k::<false>(start, from);
        r >= 0 && self.skip_at(r) == self.n
    }

    /// Parse again from `p` noting every failure: the furthest position
    /// and what was expected there (the table's indices).
    pub fn failure(&mut self, start: Id, p: i64) -> (i64, Vec<u32>) {
        self.reset(true);
        let from = self.skip_at(p);
        let mut r = self.rule_k::<true>(start, from);
        if r >= 0 {
            r = self.skip_at(r);
        }
        if r >= 0 && r < self.n {
            let end = self.g.end_desc;
            self.fail(r, end);
        }
        let far = if self.far >= 0 {
            self.far
        } else if r >= 0 {
            r
        } else {
            p
        };
        (far, self.exp.clone())
    }

    /// The root of a tree from the start rule's node: itself, or one made
    /// for it when the start rule is silent.
    pub fn root_of(&mut self, nd: u32, start: Id) -> u32 {
        let n = self.nodes[nd as usize];
        if n.kind != NONE {
            return nd;
        }
        let kind = self.g.rules[start as usize].kind;
        let kids = self.store.len() as u32;
        for k in n.kids..n.kids + n.nkids {
            let c = self.store[k as usize];
            self.store.push(Child {
                node: c.node,
                label: NONE,
            });
        }
        self.nodes.push(Node {
            kind,
            start: n.start,
            end: n.end,
            kids,
            nkids: n.nkids,
            extra: Extra::None,
        });
        (self.nodes.len() - 1) as u32
    }

    /// A node of `kind` over [a, b) with these children.
    pub fn make(&mut self, kind: u32, a: i64, b: i64, kids: &[u32]) -> u32 {
        let at = self.store.len() as u32;
        for &k in kids {
            self.store.push(Child {
                node: k,
                label: NONE,
            });
        }
        self.nodes.push(Node {
            kind,
            start: a,
            end: b,
            kids: at,
            nkids: kids.len() as u32,
            extra: Extra::None,
        });
        (self.nodes.len() - 1) as u32
    }
}
