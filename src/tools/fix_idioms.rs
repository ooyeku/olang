//! `olang fix guards | defaults | first-last | units` — the 1.0 idioms.
//!
//! Four rewrites that 1.0.0-rc.1's rules make exact, built as `olang fix
//! maps` is (see `fix.rs`): the places come from the grammar's own parse
//! tree, the rewrite is made in the source text so layout and comments
//! stay byte for byte, parentheses are added only where precedence needs
//! them, and a file whose rewrite does not read back as the same program
//! (each rewritten form read back as the form it came from) is left as it
//! was. The rewrite runs to its fixed point, so a second run changes
//! nothing.
//!
//! Each rewrite means exactly what the original meant on every input; the
//! proofs are written beside each one, and `tests/fix_idioms_test.rs` runs
//! every form against `()`, empty and ordinary inputs on all three tiers.
//! The rules they rest on:
//!
//! - **R1** a field read of `()` is `()` (`().k`, and `s.pp.cur` with
//!   `s.pp` absent); a field read of any other value is as before — a
//!   record's value, or a raise.
//! - **R2** `a ?? b` is `b` exactly when `a` is `()`, else `a`, and `b`
//!   is evaluated only then.
//! - **R3** `first(xs)` and `last(xs)` answer the first and last element
//!   of a list or tuple (the character of a string), and `()` when it is
//!   empty; they raise on anything else.
//! - **R4** `math.floor`, `ceil`, `round` and `trunc` answer an Int (or
//!   raise), and `to_int` of an Int is that Int.
//! - **R5** `==` and `!=` never raise when one side is `()`, and `() ==
//!   v` is false for every `v` that is not `()`.
//! - **R6** reading a name or a field chain has no effect, so reading it
//!   once more, or once less, changes nothing.
//!
//! What a rewrite could not prove is left, and counted by reason.

use crate::parser::{OlangParser, Rule};
use pest::Parser as PestParser;
use pest::iterators::Pair;
use std::path::{Path, PathBuf};

/// Which rewrite to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// `if x == () => () else => x.f` → `x.f`; `x != () && x.f == v` →
    /// `x.f == v`
    Guards,
    /// `if x == () => d else => x` → `x ?? d`
    Defaults,
    /// `if len(xs) == 0 => () else => xs[len(xs) - 1]` → `last(xs)`; a
    /// length-guarded `xs[0]` → `first(xs)`
    FirstLast,
    /// `to_int(math.floor(e))` → `math.floor(e)`
    Units,
}

impl Target {
    pub fn from_name(name: &str) -> Option<Target> {
        Some(match name {
            "guards" => Target::Guards,
            "defaults" => Target::Defaults,
            "first-last" | "first_last" => Target::FirstLast,
            "units" => Target::Units,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Target::Guards => "guards",
            Target::Defaults => "defaults",
            Target::FirstLast => "first-last",
            Target::Units => "units",
        }
    }
}

/// What one file's fix did.
#[derive(Debug, Default, Clone)]
pub struct Report {
    /// Each rewrite made: (line, which form).
    pub rewritten: Vec<(usize, &'static str)>,
    /// Each candidate left: (line, why).
    pub skipped: Vec<(usize, &'static str)>,
}

impl Report {
    pub fn count(&self) -> usize {
        self.rewritten.len()
    }
}

// ── skip reasons, shared by the report and the docs ───────────────────

const SKIP_COMMENT: &str = "a comment would be lost";
const SKIP_STAGE: &str = "inside a pipeline stage's lambda, whose extent the parser decides";
const SKIP_CALL_ON_PATH: &str = "a call or an index on the guarded path";
const SKIP_BOOL_READ: &str = "the guarded read is used as a Bool (`&&` needs one; `()` is not)";
const SKIP_MAYBE_UNIT: &str = "compared with a value that may itself be ()";
const SKIP_OTHER_CMP: &str = "a comparison that () answers differently";
const SKIP_DEFAULT_BLOCK: &str = "the default is a block";
const SKIP_DEFAULT_JUMP: &str = "the default is a return, break, continue, loop or assignment";
const SKIP_DEFAULT_UNIT: &str = "the default is () (`olang fix guards` drops that test)";
const SKIP_UNGUARDED: &str =
    "not proven non-empty: xs[len(xs) - 1] raises on an empty list where last(xs) answers ()";
const SKIP_NOT_SEQUENCE: &str =
    "not proven a list, tuple or string (len and [i] take Bytes; first and last refuse it)";
const SKIP_SHADOW_FL: &str = "the file binds len, first or last itself";
const SKIP_SHADOW_UNITS: &str = "the file binds to_int or math itself";
const SKIP_ROUND_ARGS: &str = "not a one-argument math.floor/ceil/round/trunc";

/// Every skip reason, for the docs and the tests.
pub const SKIP_REASONS: &[&str] = &[
    SKIP_COMMENT,
    SKIP_STAGE,
    SKIP_CALL_ON_PATH,
    SKIP_BOOL_READ,
    SKIP_MAYBE_UNIT,
    SKIP_OTHER_CMP,
    SKIP_DEFAULT_BLOCK,
    SKIP_DEFAULT_JUMP,
    SKIP_DEFAULT_UNIT,
    SKIP_UNGUARDED,
    SKIP_NOT_SEQUENCE,
    SKIP_SHADOW_FL,
    SKIP_SHADOW_UNITS,
    SKIP_ROUND_ARGS,
];

/// The fixed text of `src` and what was done, or why the file was left.
pub fn fix(src: &str, target: Target) -> Result<(String, Report), String> {
    let (bom, body) = match src.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", src),
    };
    let shebang_end = if body.starts_with("#!") {
        body.find('\n').unwrap_or(body.len())
    } else {
        0
    };
    let masked = if shebang_end > 0 {
        format!("{}{}", " ".repeat(shebang_end), &body[shebang_end..])
    } else {
        body.to_string()
    };
    let parser = crate::Parser::new();
    let before = parser
        .parse_raw(&masked)
        .map_err(|e| format!("does not parse: {}", first_line(&e.to_string())))?;
    // To the fixed point: a rewrite can make another form exact (an
    // `if` whose else branch was itself a guarded read).
    let mut report = Report::default();
    let mut current = masked.clone();
    for _ in 0..8 {
        let (out, pass_report) = pass(&current, target).inspect_err(|_| {
            if let Some(dir) = std::env::var_os("OLANG_FIX_DEBUG") {
                let _ = std::fs::write(PathBuf::from(dir).join("after.ol"), &current);
            }
        })?;
        report.rewritten.extend(pass_report.rewritten);
        report.skipped = pass_report.skipped;
        if out == current {
            break;
        }
        current = out;
    }
    if report.rewritten.is_empty() {
        return Ok((src.to_string(), report));
    }
    let after = parser.parse_raw(&current).map_err(|e| {
        format!(
            "the rewrite does not parse ({}); left as it was",
            first_line(&e.to_string())
        )
    })?;
    let (b, a) = (normalized(&before, target), normalized(&after, target));
    if b != a {
        if let Some(dir) = std::env::var_os("OLANG_FIX_DEBUG") {
            let dir = PathBuf::from(dir);
            let _ = std::fs::write(
                dir.join("before.json"),
                serde_json::to_string_pretty(&b).unwrap_or_default(),
            );
            let _ = std::fs::write(
                dir.join("after.json"),
                serde_json::to_string_pretty(&a).unwrap_or_default(),
            );
            let _ = std::fs::write(dir.join("after.ol"), &current);
        }
        return Err("the rewrite would change the program; left as it was".to_string());
    }
    let restored = if shebang_end > 0 {
        format!("{}{}", &body[..shebang_end], &current[shebang_end..])
    } else {
        current
    };
    Ok((format!("{bom}{restored}"), report))
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

/// One pass over the text: every rewrite the tree as it stands allows.
fn pass(src: &str, target: Target) -> Result<(String, Report), String> {
    let program = OlangParser::parse(Rule::program, src)
        .map_err(|e| format!("does not parse: {}", first_line(&e.to_string())))?
        .next()
        .ok_or_else(|| "does not parse".to_string())?;
    // The names a rewrite would call, which the file may bind itself.
    let called: &[&str] = match target {
        Target::FirstLast => &["len", "first", "last"],
        Target::Units => &["to_int", "math"],
        _ => &[],
    };
    let statements: Vec<Pair<'_, Rule>> = program.clone().into_inner().collect();
    let top = statements
        .iter()
        .any(|st| called.iter().any(|n| declares_at_top(st, n)));
    let tracking = target == Target::FirstLast;
    let mut scopes = Scopes::default();
    if tracking {
        scopes.frames.push(top_frame(&statements));
        assigned_unproven(program.clone(), &mut scopes.assigned_anywhere);
    }
    let mut a = Analyzer {
        src,
        base: 0,
        file: src,
        target,
        edits: Vec::new(),
        skipped: Vec::new(),
        shadowed: top,
        scopes,
        tracking,
        dry: false,
        noted: Vec::new(),
    };
    // Each top-level statement on its own: a function that binds one of
    // the names (a `let last`, a parameter `first`) leaves its reads.
    for st in statements {
        a.shadowed = top || called.iter().any(|n| rebinds(&st, n));
        if tracking {
            a.scopes.assigned_here.clear();
            assigned_unproven(st.clone(), &mut a.scopes.assigned_here);
        }
        a.visit(st, &Ctx::default());
    }
    let mut edits = a.edits;
    let skipped = a.skipped;
    edits.sort_by(|x, y| x.start.cmp(&y.start).then(y.end.cmp(&x.end)));
    let mut r = Renderer {
        src,
        edits: &edits,
        done: Vec::new(),
    };
    let mut out = String::with_capacity(src.len());
    r.render(0, src.len(), None, &mut out);
    let mut report = Report::default();
    for i in r.done {
        report
            .rewritten
            .push((line_of(src, edits[i].start), edits[i].kind));
    }
    let mut skipped = skipped;
    skipped.sort();
    skipped.dedup();
    for (at, why) in skipped {
        report.skipped.push((line_of(src, at), why));
    }
    Ok((out, report))
}

fn line_of(src: &str, offset: usize) -> usize {
    let at = offset.min(src.len());
    src.as_bytes()[..at].iter().filter(|b| **b == b'\n').count() + 1
}

// ── edits and their rendering ─────────────────────────────────────────

/// A piece of a replacement: literal text, or a range of the source
/// rendered with the edits inside it made.
#[derive(Debug, Clone)]
enum Piece {
    Text(String),
    Src(usize, usize),
}

#[derive(Debug, Clone)]
struct Edit {
    start: usize,
    end: usize,
    pieces: Vec<Piece>,
    kind: &'static str,
    /// It replaces an `if` written as `else if`: where it stays an else
    /// branch it needs a `=> ` (not where an edit around it takes it in).
    else_branch: bool,
}

struct Renderer<'a> {
    src: &'a str,
    edits: &'a [Edit],
    /// The edits made, by index: an edit inside text another edit drops
    /// is not made, and not counted.
    done: Vec<usize>,
}

impl Renderer<'_> {
    /// The range with every edit inside it made, `owner` (the edit whose
    /// piece this range is) excepted. An edit inside one already made, or
    /// across its end, is left to that edit's pieces.
    fn render(&mut self, start: usize, end: usize, owner: Option<usize>, out: &mut String) {
        let mut pos = start;
        let mut i = self.edits.partition_point(|e| e.start < start);
        while i < self.edits.len() && self.edits[i].start < end {
            let e = &self.edits[i];
            if Some(i) == owner || e.end > end || e.start < pos {
                i += 1;
                continue;
            }
            out.push_str(&self.src[pos..e.start]);
            if e.else_branch && !(owner.is_some() && e.start == start) {
                out.push_str("=> ");
            }
            let pieces = e.pieces.clone();
            let next = e.end;
            for p in pieces {
                match p {
                    Piece::Text(t) => out.push_str(&t),
                    Piece::Src(a, b) => self.render(a, b, Some(i), out),
                }
            }
            self.done.push(i);
            pos = next;
            i += 1;
        }
        out.push_str(&self.src[pos..end]);
    }
}

// ── the binary-expression tree ────────────────────────────────────────

/// A flat `binary_expr`'s operands and operators as the parser's
/// precedence climbing builds them (`Parser::climb`), with spans.
enum Node<'i> {
    Leaf(Pair<'i, Rule>),
    Bin {
        op: Pair<'i, Rule>,
        left: Box<Node<'i>>,
        right: Box<Node<'i>>,
    },
}

impl<'i> Node<'i> {
    fn start(&self) -> usize {
        match self {
            Node::Leaf(p) => p.as_span().start(),
            Node::Bin { left, .. } => left.start(),
        }
    }

    /// `(left, operator text, right)` when this is a binary node.
    fn bin(&self) -> Option<(&Node<'i>, &str, &Node<'i>)> {
        match self {
            Node::Bin { op, left, right } => Some((left, op.as_str(), right)),
            Node::Leaf(_) => None,
        }
    }

    fn leaf(&self) -> Option<&Pair<'i, Rule>> {
        match self {
            Node::Leaf(p) => Some(p),
            Node::Bin { .. } => None,
        }
    }
}

/// `Parser::precedence`, mirrored.
fn precedence(rule: Rule, text: &str) -> u8 {
    match rule {
        Rule::or_op => 1,
        Rule::and_op => 2,
        Rule::comp_op => 3,
        Rule::coalesce_op => 4,
        Rule::pipe_op => 5,
        Rule::range_op => 6,
        Rule::bitwise_op => match text {
            "|" => 7,
            "^" => 8,
            "&" => 9,
            _ => 10,
        },
        Rule::add_op => 11,
        _ => 12,
    }
}

fn is_operator(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::or_op
            | Rule::and_op
            | Rule::comp_op
            | Rule::coalesce_op
            | Rule::pipe_op
            | Rule::range_op
            | Rule::bitwise_op
            | Rule::add_op
            | Rule::mul_op
    )
}

fn build_tree<'i>(children: Vec<Pair<'i, Rule>>) -> Option<Node<'i>> {
    let mut operands = Vec::new();
    let mut operators = Vec::new();
    for c in children {
        if is_operator(c.as_rule()) {
            operators.push(c);
        } else {
            operands.push(c);
        }
    }
    if operands.len() != operators.len() + 1 {
        return None;
    }
    let mut operands = operands.into_iter();
    let mut operators = operators.into_iter().peekable();
    let first = Node::Leaf(operands.next()?);
    Some(climb(first, &mut operands, &mut operators, 1))
}

fn climb<'i>(
    mut left: Node<'i>,
    operands: &mut std::vec::IntoIter<Pair<'i, Rule>>,
    operators: &mut std::iter::Peekable<std::vec::IntoIter<Pair<'i, Rule>>>,
    min: u8,
) -> Node<'i> {
    while let Some(op) = operators.peek() {
        let level = precedence(op.as_rule(), op.as_str());
        if level < min {
            break;
        }
        let op = operators.next().expect("peeked");
        let Some(next) = operands.next() else {
            break;
        };
        let right = climb(Node::Leaf(next), operands, operators, level + 1);
        left = Node::Bin {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    left
}

// ── views over the parse tree ─────────────────────────────────────────

/// The pair with the wrappers that are only their one child taken off
/// (`expr` around `binary_expr` around a sign-free `unary_expr` …, and a
/// one-expression `( … )`).
fn strip<'i>(p: Pair<'i, Rule>) -> Pair<'i, Rule> {
    let mut p = p;
    loop {
        let mut kids = p.clone().into_inner();
        let (Some(only), None) = (kids.next(), kids.next()) else {
            return p;
        };
        match p.as_rule() {
            Rule::expr
            | Rule::positional_arg
            | Rule::primary
            | Rule::cmp_rhs
            | Rule::cmp_group
            | Rule::binary_expr
            | Rule::unary_expr
            | Rule::call_expr
            | Rule::paren_expr => p = only,
            Rule::literal if only.as_rule() == Rule::unit => p = only,
            _ => return p,
        }
    }
}

/// The binary tree of an expression (a lone operand is a leaf).
fn tree_of(p: Pair<'_, Rule>) -> Option<Node<'_>> {
    let s = strip(p);
    if s.as_rule() == Rule::binary_expr {
        build_tree(s.into_inner().collect())
    } else {
        Some(Node::Leaf(s))
    }
}

fn is_unit(p: &Pair<'_, Rule>) -> bool {
    strip(p.clone()).as_rule() == Rule::unit
}

/// A name, or a name and its fields: `x`, `s.pp.cur` — no call, no
/// index. Its span is the pair's.
fn chain(p: &Pair<'_, Rule>) -> Option<Vec<String>> {
    let s = strip(p.clone());
    match s.as_rule() {
        Rule::identifier => Some(vec![s.as_str().to_string()]),
        Rule::call_expr => {
            let mut kids = s.into_inner();
            let head = strip(kids.next()?);
            if head.as_rule() != Rule::identifier {
                return None;
            }
            let mut out = vec![head.as_str().to_string()];
            for k in kids {
                if k.as_rule() != Rule::member_name {
                    return None;
                }
                out.push(k.as_str().to_string());
            }
            Some(out)
        }
        _ => None,
    }
}

fn node_chain(n: &Node<'_>) -> Option<Vec<String>> {
    n.leaf().and_then(chain)
}

fn node_is_unit(n: &Node<'_>) -> bool {
    n.leaf().is_some_and(is_unit)
}

/// The chain starts a read that goes on with a call or an index:
/// `x.f(…)`, `x.f[0]` — the root it starts at.
fn calls_on_path(p: &Pair<'_, Rule>) -> Option<String> {
    let s = strip(p.clone());
    if s.as_rule() != Rule::call_expr {
        return None;
    }
    let mut kids = s.into_inner();
    let head = strip(kids.next()?);
    if head.as_rule() != Rule::identifier {
        return None;
    }
    kids.any(|k| k.as_rule() != Rule::member_name)
        .then(|| head.as_str().to_string())
}

/// A literal that is never `()`: a number, a string without
/// interpolation, a Bool, a character — or a negated number.
fn literal_not_unit(p: &Pair<'_, Rule>) -> bool {
    let s = strip(p.clone());
    match s.as_rule() {
        Rule::literal => s.into_inner().next().is_some_and(|k| {
            matches!(
                k.as_rule(),
                Rule::integer
                    | Rule::float
                    | Rule::string
                    | Rule::raw_string
                    | Rule::boolean
                    | Rule::char_literal
                    | Rule::binary
                    | Rule::octal
                    | Rule::hex
            )
        }),
        Rule::unary_expr => {
            let kids: Vec<_> = s.into_inner().collect();
            kids.len() == 2
                && kids[0].as_str() == "-"
                && strip(kids[1].clone()).as_rule() == Rule::literal
                && strip(kids[1].clone())
                    .into_inner()
                    .next()
                    .is_some_and(|k| matches!(k.as_rule(), Rule::integer | Rule::float))
        }
        _ => false,
    }
}

fn node_literal(n: &Node<'_>) -> bool {
    n.leaf().is_some_and(literal_not_unit)
}

/// `n` when the pair is the integer literal `n`.
fn int_literal(p: &Pair<'_, Rule>) -> Option<i64> {
    let s = strip(p.clone());
    if s.as_rule() != Rule::literal {
        return None;
    }
    let k = s.into_inner().next()?;
    if k.as_rule() != Rule::integer {
        return None;
    }
    k.as_str().replace('_', "").parse().ok()
}

/// The callee name and the positional arguments of a plain call `f(a,
/// b)` with nothing after it.
fn plain_call<'i>(p: &Pair<'i, Rule>) -> Option<(String, Vec<Pair<'i, Rule>>)> {
    let s = strip(p.clone());
    if s.as_rule() != Rule::call_expr {
        return None;
    }
    let kids: Vec<_> = s.into_inner().collect();
    if kids.len() != 2 || kids[1].as_rule() != Rule::function_call {
        return None;
    }
    let head = strip(kids[0].clone());
    if head.as_rule() != Rule::identifier {
        return None;
    }
    Some((head.as_str().to_string(), call_args(&kids[1])?))
}

/// The positional arguments of a `function_call`; `None` when one is
/// named.
fn call_args<'i>(call: &Pair<'i, Rule>) -> Option<Vec<Pair<'i, Rule>>> {
    let mut out = Vec::new();
    for list in call.clone().into_inner() {
        if list.as_rule() != Rule::arg_list {
            continue;
        }
        for arg in list.into_inner() {
            if arg.as_rule() != Rule::argument {
                continue;
            }
            let inner = arg.into_inner().next()?;
            if inner.as_rule() != Rule::positional_arg {
                return None;
            }
            out.push(inner);
        }
    }
    Some(out)
}

/// `len(C)` → C.
fn len_of(n: &Node<'_>) -> Option<Vec<String>> {
    let (name, args) = plain_call(n.leaf()?)?;
    if name != "len" || args.len() != 1 {
        return None;
    }
    chain(&args[0])
}

fn node_int(n: &Node<'_>) -> Option<i64> {
    n.leaf().and_then(int_literal)
}

/// The chain a comparison proves non-empty when it is true: `len(C) >
/// 0`, `!= 0`, `>= 1`, and the same written the other way round.
fn nonempty_test(n: &Node<'_>) -> Option<Vec<String>> {
    let (l, op, r) = n.bin()?;
    if let Some(c) = len_of(l) {
        let k = node_int(r)?;
        return matches!((op, k), (">", 0) | ("!=", 0) | (">=", 1)).then_some(c);
    }
    let c = len_of(r)?;
    let k = node_int(l)?;
    matches!((op, k), ("<", 0) | ("!=", 0) | ("<=", 1)).then_some(c)
}

/// The chain a comparison proves non-empty when it is false: `len(C) ==
/// 0`, `< 1`, `<= 0`, either way round.
fn empty_test(n: &Node<'_>) -> Option<Vec<String>> {
    let (l, op, r) = n.bin()?;
    if let Some(c) = len_of(l) {
        let k = node_int(r)?;
        return matches!((op, k), ("==", 0) | ("<", 1) | ("<=", 0)).then_some(c);
    }
    let c = len_of(r)?;
    let k = node_int(l)?;
    matches!((op, k), ("==", 0) | (">", 1) | (">=", 0)).then_some(c)
}

/// Every chain the condition proves non-empty when it is true (its `&&`
/// conjuncts), or when it is false (its `||` disjuncts).
fn facts(n: &Node<'_>, when: bool, out: &mut Vec<Vec<String>>) {
    if let Some((l, op, r)) = n.bin()
        && ((when && op == "&&") || (!when && op == "||"))
    {
        facts(l, when, out);
        facts(r, when, out);
        return;
    }
    let found = if when {
        nonempty_test(n)
    } else {
        empty_test(n)
    };
    if let Some(c) = found {
        out.push(c);
    }
}

fn is_prefix(p: &[String], c: &[String]) -> bool {
    p.len() <= c.len() && c[..p.len()] == *p
}

/// `p != ()` (or `p == ()` with `eq`): the chain p.
fn unit_test(n: &Node<'_>, eq: bool) -> Option<Vec<String>> {
    let (l, op, r) = n.bin()?;
    if op != if eq { "==" } else { "!=" } {
        return None;
    }
    if node_is_unit(r) {
        node_chain(l)
    } else if node_is_unit(l) {
        node_chain(r)
    } else {
        None
    }
}

/// Every operand of a chain of one operator (`a && b && c`), left to
/// right; `None` when an operand is not of the form `f` takes.
fn all_of<T>(n: &Node<'_>, op: &str, f: &dyn Fn(&Node<'_>) -> Option<T>) -> Option<Vec<T>> {
    if let Some((l, o, r)) = n.bin()
        && o == op
    {
        let mut v = all_of(l, op, f)?;
        v.push(f(r)?);
        return Some(v);
    }
    Some(vec![f(n)?])
}

/// Whether any name in `names` is bound inside the pair: a `let`, a
/// parameter, a loop variable, a pattern, an assignment.
fn rebinds(p: &Pair<'_, Rule>, name: &str) -> bool {
    fn walk(p: Pair<'_, Rule>, name: &str, binding: bool) -> bool {
        let rule = p.as_rule();
        if rule == Rule::identifier {
            return binding && p.as_str() == name;
        }
        let kids: Vec<_> = p.into_inner().collect();
        for (i, k) in kids.into_iter().enumerate() {
            let b = match rule {
                Rule::pattern
                | Rule::or_pattern
                | Rule::base_pattern
                | Rule::struct_pattern_field
                | Rule::struct_pattern_fields
                | Rule::rest_pattern
                | Rule::list_pattern
                | Rule::list_pattern_inner
                | Rule::tuple_pattern
                | Rule::enum_variant_pattern
                | Rule::result_pattern
                | Rule::struct_pattern
                | Rule::anonymous_struct_pattern
                | Rule::for_binding_tuple => true,
                Rule::parameter => i == 0,
                Rule::for_loop | Rule::par_for_loop => k.as_rule() == Rule::identifier,
                Rule::assignment_expr => i == 0,
                Rule::function_decl => k.as_rule() == Rule::identifier,
                _ => false,
            };
            if walk(k, name, b || (binding && rule != Rule::expr)) {
                return true;
            }
        }
        false
    }
    walk(p.clone(), name, false)
}

// ── what the file binds ───────────────────────────────────────────────

/// The code of the text with strings, template text and comments blanked
/// to spaces (a template's `${…}` kept, the quotes kept), byte for byte,
/// so a word search finds only code and an offset into it is an offset
/// into the text.
fn code_only(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let chars: Vec<char> = src.chars().collect();
    let blank = |out: &mut String, c: char| {
        if c == '\n' {
            out.push('\n');
        } else {
            for _ in 0..c.len_utf8() {
                out.push(' ');
            }
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                blank(&mut out, chars[i]);
                i += 1;
            }
            continue;
        }
        if c == '"' {
            out.push('"');
            i += 1;
            while i < chars.len() {
                let d = chars[i];
                i += 1;
                if d == '\\' && i < chars.len() {
                    blank(&mut out, d);
                    blank(&mut out, chars[i]);
                    i += 1;
                } else if d == '"' {
                    out.push('"');
                    break;
                } else {
                    blank(&mut out, d);
                }
            }
            continue;
        }
        if c == '\'' && chars.get(i + 2) == Some(&'\'') {
            out.push('\'');
            blank(&mut out, chars[i + 1]);
            out.push('\'');
            i += 3;
            continue;
        }
        if c == '`' {
            out.push('`');
            i += 1;
            while i < chars.len() {
                let d = chars[i];
                if d == '\\' && i + 1 < chars.len() {
                    blank(&mut out, d);
                    blank(&mut out, chars[i + 1]);
                    i += 2;
                    continue;
                }
                if d == '`' {
                    out.push('`');
                    i += 1;
                    break;
                }
                if d == '$' && chars.get(i + 1) == Some(&'{') {
                    out.push_str("${");
                    i += 2;
                    let mut depth = 1;
                    while i < chars.len() {
                        let e = chars[i];
                        if e == '{' {
                            depth += 1;
                        } else if e == '}' {
                            depth -= 1;
                            if depth == 0 {
                                out.push('}');
                                i += 1;
                                break;
                            }
                        }
                        out.push(e);
                        i += 1;
                    }
                    continue;
                }
                blank(&mut out, d);
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Where the text's last token ends: trailing blanks and comments are
/// not the expression's (a pair's span can take them in).
fn token_end(text: &str) -> usize {
    code_only(text).trim_end().len()
}

/// Whether a top-level statement declares `name` for the whole file: a
/// function of that name, a top-level `let`, an import (a `*` import may
/// bring any name).
fn declares_at_top(st: &Pair<'_, Rule>, name: &str) -> bool {
    let mut p = st.clone();
    loop {
        let Some(inner) = p.clone().into_inner().find(|k| {
            matches!(
                k.as_rule(),
                Rule::function_decl
                    | Rule::let_decl
                    | Rule::use_decl
                    | Rule::share_decl
                    | Rule::decorated_decl
                    | Rule::meta_fn_decl
            )
        }) else {
            return false;
        };
        match inner.as_rule() {
            Rule::share_decl | Rule::decorated_decl | Rule::meta_fn_decl => p = inner,
            Rule::function_decl => {
                return inner
                    .into_inner()
                    .find(|k| k.as_rule() == Rule::identifier)
                    .is_some_and(|k| k.as_str() == name);
            }
            Rule::let_decl => return rebinds(&inner, name),
            Rule::use_decl => {
                return inner
                    .into_inner()
                    .filter(|k| k.as_rule() == Rule::use_list)
                    .flat_map(|l| l.into_inner())
                    .any(|item| {
                        let t = item.as_str().trim();
                        t == "*" || t == name || t.ends_with(&format!(" {name}"))
                    });
            }
            _ => return false,
        }
    }
}

/// Where a name stands, for `first-last`: whether the binding a read
/// reaches is proven never Bytes. `len` and `[i]` take Bytes and
/// `first`/`last` refuse it, so a rewrite needs that proof of the list it
/// reads: a `let` (or parameter annotation) that makes a list, a string or
/// another value that is not Bytes, and no assignment to the name that
/// might make it one.
#[derive(Clone, Default)]
struct Scopes {
    /// Innermost last; the first is the file's top level.
    frames: Vec<std::collections::HashMap<String, bool>>,
    /// Names assigned in this top-level statement a value not proven.
    assigned_here: std::collections::HashSet<String>,
    /// … anywhere in the file (what a top-level binding needs).
    assigned_anywhere: std::collections::HashSet<String>,
}

impl Scopes {
    fn bind(&mut self, name: &str, ok: bool) {
        if let Some(f) = self.frames.last_mut() {
            f.insert(name.to_string(), ok);
        }
    }

    /// Every name the pattern (or parameter list, loop binding) binds,
    /// bound to something unproven.
    fn bind_all(&mut self, p: Pair<'_, Rule>) {
        if p.as_rule() == Rule::identifier {
            self.bind(p.as_str(), false);
            return;
        }
        for k in p.into_inner() {
            self.bind_all(k);
        }
    }

    fn params(&mut self, list: Pair<'_, Rule>) {
        for param in list.into_inner().filter(|k| k.as_rule() == Rule::parameter) {
            let kids: Vec<_> = param.into_inner().collect();
            if let Some(name) = kids.first() {
                let ok = kids
                    .iter()
                    .find(|k| k.as_rule() == Rule::type_annotation)
                    .is_some_and(annotation_is_sequence);
                self.bind(name.as_str(), ok);
            }
        }
    }

    fn known(&self, name: &str) -> bool {
        for (i, f) in self.frames.iter().enumerate().rev() {
            if let Some(ok) = f.get(name) {
                let assigned = if i == 0 {
                    &self.assigned_anywhere
                } else {
                    &self.assigned_here
                };
                return *ok && !assigned.contains(name);
            }
        }
        false
    }
}

/// The names assigned (`x = e`, not `x[i] = e`) a value not proven never
/// Bytes, anywhere inside.
fn assigned_unproven(p: Pair<'_, Rule>, out: &mut std::collections::HashSet<String>) {
    if p.as_rule() == Rule::assignment_expr {
        let kids: Vec<_> = p.clone().into_inner().collect();
        let indexed = kids.iter().any(|k| k.as_rule() == Rule::index_target);
        if let Some(name) = kids.first()
            && !indexed
            && !kids.last().is_some_and(|v| not_bytes(v))
        {
            out.insert(name.as_str().to_string());
        }
    }
    for k in p.into_inner() {
        assigned_unproven(k, out);
    }
}

/// The file's top-level bindings: its functions, its `let`s, its imports.
fn top_frame(statements: &[Pair<'_, Rule>]) -> std::collections::HashMap<String, bool> {
    let mut frame: std::collections::HashMap<String, bool> = Default::default();
    let mut note = |name: &str, ok: bool| {
        let e = frame.entry(name.to_string()).or_insert(true);
        *e = *e && ok;
    };
    fn all(p: Pair<'_, Rule>, note: &mut dyn FnMut(&str, bool)) {
        if p.as_rule() == Rule::identifier {
            note(p.as_str(), false);
            return;
        }
        for k in p.into_inner() {
            all(k, note);
        }
    }
    for st in statements {
        let mut p = st.clone();
        while let Some(inner) = p.clone().into_inner().find(|k| {
            matches!(
                k.as_rule(),
                Rule::function_decl
                    | Rule::let_decl
                    | Rule::use_decl
                    | Rule::share_decl
                    | Rule::decorated_decl
                    | Rule::meta_fn_decl
            )
        }) {
            match inner.as_rule() {
                Rule::share_decl | Rule::decorated_decl | Rule::meta_fn_decl => p = inner,
                Rule::function_decl => {
                    if let Some(k) = inner.into_inner().find(|k| k.as_rule() == Rule::identifier) {
                        note(k.as_str(), false);
                    }
                    break;
                }
                Rule::let_decl => {
                    let kids: Vec<_> = inner.into_inner().collect();
                    if let Some(pat) = kids.iter().find(|k| k.as_rule() == Rule::pattern) {
                        match bare_pattern_name(pat) {
                            Some(name) => {
                                let ann =
                                    kids.iter().find(|k| k.as_rule() == Rule::type_annotation);
                                let value = kids.iter().find(|k| k.as_rule() == Rule::expr);
                                let ok = match (ann, value) {
                                    (Some(a), _) => annotation_is_sequence(a),
                                    (None, Some(v)) => not_bytes(v),
                                    _ => false,
                                };
                                note(&name, ok);
                            }
                            None => all(pat.clone(), &mut note),
                        }
                    }
                    break;
                }
                _ => {
                    all(inner, &mut note);
                    break;
                }
            }
        }
    }
    frame
}

/// `let x` — the name, when the pattern is a bare name.
fn bare_pattern_name(pat: &Pair<'_, Rule>) -> Option<String> {
    let mut p = pat.clone();
    loop {
        let mut kids = p.clone().into_inner();
        let (Some(only), None) = (kids.next(), kids.next()) else {
            return None;
        };
        if only.as_rule() == Rule::identifier {
            return Some(only.as_str().to_string());
        }
        if !matches!(
            only.as_rule(),
            Rule::or_pattern | Rule::base_pattern | Rule::pattern
        ) {
            return None;
        }
        p = only;
    }
}

/// `[T]` or `String`: the runtime refuses anything else at the binding.
fn annotation_is_sequence(a: &Pair<'_, Rule>) -> bool {
    let Some(k) = a.clone().into_inner().next() else {
        return false;
    };
    match k.as_rule() {
        Rule::list_type => true,
        Rule::basic_type => k.as_str() == "String",
        _ => false,
    }
}

/// Global builtins that answer a fresh List (or raise).
const LIST_FNS: &[&str] = &[
    "split",
    "map",
    "filter",
    "sort",
    "reverse",
    "take",
    "drop",
    "flatten",
    "zip",
    "enumerate",
    "concat",
    "entries",
    "map_keys",
    "map_values",
    "to_string",
    "show",
    "join",
];
/// `str.` functions that answer a List or a String (or raise).
const STR_FNS: &[&str] = &[
    "split",
    "lines",
    "chars",
    "trim",
    "trim_start",
    "trim_end",
    "lower",
    "upper",
    "replace",
    "substring",
    "join",
    "repeat",
    "pad_start",
    "pad_end",
];

/// Whether the expression's value can never be Bytes: a literal, an
/// arithmetic or logical result, a `+` with a string or list literal, a
/// fresh list or string from a builtin that makes one, or an `if` whose
/// branches are all of these.
fn not_bytes(e: &Pair<'_, Rule>) -> bool {
    let s = strip(e.clone());
    match s.as_rule() {
        Rule::literal | Rule::unit | Rule::map_literal | Rule::anonymous_object => true,
        Rule::template_string | Rule::string | Rule::raw_string => true,
        Rule::if_expr => {
            let kids: Vec<_> = s.into_inner().collect();
            let branches: Vec<_> = kids
                .iter()
                .skip(2)
                .filter(|k| k.as_rule() != Rule::else_kw)
                .collect();
            branches.len() == 2
                && branches.iter().all(|b| match b.as_rule() {
                    Rule::expr | Rule::if_expr => not_bytes(b),
                    _ => false,
                })
        }
        Rule::binary_expr => {
            let Some(t) = build_tree(s.into_inner().collect()) else {
                return false;
            };
            match t.bin() {
                Some((l, op, r)) => match op {
                    "+" => [l, r].iter().any(|n| {
                        n.leaf().is_some_and(|p| {
                            let s = strip(p.clone());
                            s.as_rule() == Rule::literal
                                && s.into_inner().next().is_some_and(|k| {
                                    matches!(
                                        k.as_rule(),
                                        Rule::string
                                            | Rule::raw_string
                                            | Rule::template_string
                                            | Rule::list
                                    )
                                })
                        })
                    }),
                    "??" | "|>" => false,
                    _ => true,
                },
                None => false,
            }
        }
        Rule::call_expr => {
            let kids: Vec<_> = s.into_inner().collect();
            let head = strip(kids[0].clone());
            if head.as_rule() != Rule::identifier {
                return false;
            }
            let name = head.as_str();
            match kids.len() {
                2 if kids[1].as_rule() == Rule::function_call => LIST_FNS.contains(&name),
                3 if name == "str"
                    && kids[1].as_rule() == Rule::member_name
                    && kids[2].as_rule() == Rule::function_call =>
                {
                    STR_FNS.contains(&kids[1].as_str())
                }
                _ => false,
            }
        }
        _ => false,
    }
}

// ── the analysis ──────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct Ctx {
    /// Something next to this position binds tighter than `??`: a `??`
    /// written here needs parentheses.
    tight: bool,
    /// The chains known non-empty here.
    nonempty: Vec<Vec<String>>,
    /// The tail of a pipeline stage's lambda body, which the parser cuts
    /// at its first `|>`: the tree here is not the parse tree.
    stage: bool,
    /// An `if` written as `else if`: what replaces it is an else branch,
    /// and needs its `=>`.
    else_if: bool,
}

impl Ctx {
    fn inner(&self) -> Ctx {
        Ctx {
            tight: false,
            nonempty: self.nonempty.clone(),
            stage: false,
            else_if: false,
        }
    }

    fn fresh(&self) -> Ctx {
        Ctx::default()
    }
}

struct Analyzer<'a> {
    /// The text the pairs index into (the file, or one interpolation).
    src: &'a str,
    /// Where `src` starts in the file.
    base: usize,
    file: &'a str,
    target: Target,
    edits: Vec<Edit>,
    skipped: Vec<(usize, &'static str)>,
    /// The file binds a name the rewrite would call.
    shadowed: bool,
    /// The bindings in scope (first-last).
    scopes: Scopes,
    tracking: bool,
    /// Looking only: a rewrite found is counted as left (the tree here
    /// is not the parse tree), and nothing else is counted.
    dry: bool,
    /// Reads already counted as left with their whole `if`.
    noted: Vec<usize>,
}

impl<'a> Analyzer<'a> {
    fn abs(&self, local: usize) -> usize {
        self.base + local
    }

    fn skip(&mut self, at: usize, why: &'static str) {
        if !self.dry {
            self.skipped.push((self.abs(at), why));
        }
    }

    /// An edit of a whole `if`: one written as `else if` keeps its place
    /// as an else branch.
    fn edit_if(
        &mut self,
        ctx: &Ctx,
        start: usize,
        end: usize,
        pieces: Vec<Piece>,
        kind: &'static str,
    ) {
        self.edit(start, end, pieces, kind);
        if ctx.else_if
            && let Some(e) = self.edits.last_mut()
            && e.start == self.base + start
        {
            e.else_branch = true;
        }
    }

    fn edit(&mut self, start: usize, end: usize, pieces: Vec<Piece>, kind: &'static str) {
        if self.dry {
            self.skipped.push((self.abs(start), SKIP_STAGE));
            return;
        }
        let start = self.abs(start);
        let end = self.abs(end);
        self.edits.push(Edit {
            start,
            end,
            pieces,
            kind,
            else_branch: false,
        });
    }

    fn src_piece(&self, start: usize, end: usize) -> Piece {
        Piece::Src(self.abs(start), self.abs(end))
    }

    fn has_comment(&self, start: usize, end: usize) -> bool {
        self.src[start..end].contains("//")
    }

    /// The pair's span, its trailing blanks and comments left out.
    fn span(&self, p: &Pair<'_, Rule>) -> (usize, usize) {
        let s = p.as_span();
        (
            s.start(),
            s.start() + token_end(&self.src[s.start()..s.end()]),
        )
    }

    fn visit(&mut self, p: Pair<'a, Rule>, ctx: &Ctx) {
        match p.as_rule() {
            Rule::binary_expr => self.visit_binary(p, ctx),
            Rule::unary_expr => {
                let has_op = p
                    .clone()
                    .into_inner()
                    .any(|k| k.as_rule() == Rule::unary_op);
                let mut c = ctx.clone();
                if has_op {
                    c.tight = true;
                    c.stage = ctx.stage;
                }
                for k in p.into_inner() {
                    self.visit(k, &c);
                }
            }
            Rule::call_expr => self.visit_call(p, ctx),
            Rule::if_expr => self.visit_if(p, ctx),
            Rule::lambda | Rule::spawn_expr => {
                let c = Ctx {
                    stage: ctx.stage,
                    ..Ctx::default()
                };
                let lambda = p.as_rule() == Rule::lambda;
                if lambda && self.tracking {
                    self.scopes.frames.push(Default::default());
                }
                for k in p.into_inner() {
                    let kc = if matches!(k.as_rule(), Rule::expr | Rule::block | Rule::call_expr) {
                        c.clone()
                    } else {
                        c.fresh()
                    };
                    let list = k.as_rule() == Rule::param_list;
                    self.visit(k.clone(), &kc);
                    if list && self.tracking {
                        self.scopes.params(k);
                    }
                }
                if lambda && self.tracking {
                    self.scopes.frames.pop();
                }
            }
            Rule::function_decl | Rule::trait_method if self.tracking => {
                let kids: Vec<_> = p.into_inner().collect();
                if self.scopes.frames.len() > 1
                    && let Some(name) = kids.iter().find(|k| k.as_rule() == Rule::identifier)
                {
                    self.scopes.bind(name.as_str(), false);
                }
                self.scopes.frames.push(Default::default());
                for k in kids {
                    match k.as_rule() {
                        Rule::param_list => {
                            self.visit(k.clone(), &Ctx::default());
                            self.scopes.params(k);
                        }
                        Rule::identifier => {}
                        _ => self.visit(k, &Ctx::default()),
                    }
                }
                self.scopes.frames.pop();
            }
            Rule::function_decl | Rule::test_decl | Rule::trait_method => {
                for k in p.into_inner() {
                    self.visit(k, &Ctx::default());
                }
            }
            Rule::block if self.tracking => {
                self.scopes.frames.push(Default::default());
                for k in p.into_inner() {
                    self.visit(k, &ctx.inner());
                }
                self.scopes.frames.pop();
            }
            Rule::let_decl if self.tracking => {
                let kids: Vec<_> = p.into_inner().collect();
                for k in &kids {
                    if k.as_rule() != Rule::pattern {
                        self.visit(k.clone(), &ctx.inner());
                    }
                }
                if self.scopes.frames.len() > 1
                    && let Some(pat) = kids.iter().find(|k| k.as_rule() == Rule::pattern)
                {
                    match bare_pattern_name(pat) {
                        Some(name) => {
                            let ann = kids.iter().find(|k| k.as_rule() == Rule::type_annotation);
                            let value = kids.iter().find(|k| k.as_rule() == Rule::expr);
                            let ok = match (ann, value) {
                                (Some(a), _) => annotation_is_sequence(a),
                                (None, Some(v)) => not_bytes(v),
                                _ => false,
                            };
                            self.scopes.bind(&name, ok);
                        }
                        None => self.scopes.bind_all(pat.clone()),
                    }
                }
            }
            Rule::for_loop | Rule::par_for_loop if self.tracking => {
                let kids: Vec<_> = p.into_inner().collect();
                for k in &kids {
                    if k.as_rule() == Rule::expr {
                        self.visit(k.clone(), &ctx.inner());
                    }
                }
                self.scopes.frames.push(Default::default());
                for k in kids {
                    match k.as_rule() {
                        Rule::identifier | Rule::for_binding_tuple => self.scopes.bind_all(k),
                        Rule::block => self.visit(k, &ctx.inner()),
                        _ => {}
                    }
                }
                self.scopes.frames.pop();
            }
            Rule::match_arm if self.tracking => {
                self.scopes.frames.push(Default::default());
                for k in p.into_inner() {
                    if k.as_rule() == Rule::pattern {
                        self.scopes.bind_all(k);
                    } else {
                        self.visit(k, &ctx.inner());
                    }
                }
                self.scopes.frames.pop();
            }
            Rule::template_string => self.visit_template(p, ctx),
            Rule::macro_call => {}
            Rule::expr | Rule::primary | Rule::positional_arg | Rule::cmp_rhs => {
                // one child standing for this pair: its context is ours
                let mut kids = p.clone().into_inner();
                if let (Some(only), None) = (kids.next(), kids.next()) {
                    self.visit(only, ctx);
                } else {
                    for k in p.into_inner() {
                        self.visit(k, &ctx.inner());
                    }
                }
            }
            _ => {
                for k in p.into_inner() {
                    self.visit(k, &ctx.inner());
                }
            }
        }
    }

    fn visit_template(&mut self, p: Pair<'a, Rule>, ctx: &Ctx) {
        let span = p.as_span();
        let text = &self.src[span.start()..span.end()];
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let mut i = 0;
        while i < chars.len() {
            let (_, c) = chars[i];
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '$' && chars.get(i + 1).map(|x| x.1) == Some('{') {
                let start = chars.get(i + 2).map(|x| x.0).unwrap_or(text.len());
                let mut depth = 1;
                let mut in_string = false;
                let mut escape = false;
                let mut j = i + 2;
                while j < chars.len() {
                    let d = chars[j].1;
                    if in_string {
                        if escape {
                            escape = false;
                        } else if d == '\\' {
                            escape = true;
                        } else if d == '"' {
                            in_string = false;
                        }
                    } else if d == '"' {
                        in_string = true;
                    } else if d == '{' {
                        depth += 1;
                    } else if d == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    j += 1;
                }
                let end = chars.get(j).map(|x| x.0).unwrap_or(text.len());
                let inner = &text[start..end];
                let lead = inner.len() - inner.trim_start().len();
                let trimmed = inner.trim();
                let at = span.start() + start + lead;
                self.visit_interpolation(trimmed, at, ctx);
                i = j + 1;
                continue;
            }
            i += 1;
        }
    }

    fn visit_interpolation(&mut self, text: &str, at: usize, ctx: &Ctx) {
        let Ok(mut pairs) = OlangParser::parse(Rule::expr, text) else {
            return;
        };
        let Some(expr) = pairs.next() else {
            return;
        };
        if expr.as_span().end() != text.len() {
            return;
        }
        let mut sub = Analyzer {
            src: text,
            base: self.base + at,
            file: self.file,
            target: self.target,
            edits: Vec::new(),
            skipped: Vec::new(),
            shadowed: self.shadowed,
            scopes: if self.tracking {
                self.scopes.clone()
            } else {
                Scopes::default()
            },
            tracking: self.tracking,
            dry: self.dry,
            noted: Vec::new(),
        };
        sub.visit(expr, &ctx.inner());
        self.edits.extend(sub.edits);
        self.skipped.extend(sub.skipped);
    }

    fn visit_binary(&mut self, p: Pair<'a, Rule>, ctx: &Ctx) {
        let kids: Vec<Pair<'a, Rule>> = p.clone().into_inner().collect();
        if kids.len() == 1 {
            let only = kids.into_iter().next().expect("one");
            self.visit(only, ctx);
            return;
        }
        // A lambda right after `|>` is a stage the parser cuts at its
        // first `|>`: this flat list is not the tree, so nothing that
        // depends on the tree's shape is made here.
        let staged = kids
            .windows(2)
            .any(|w| w[0].as_rule() == Rule::pipe_op && bare_lambda(&w[1]));
        let Some(tree) = build_tree(kids) else {
            return;
        };
        let allow = !ctx.stage && !staged;
        self.visit_node(&tree, ctx, allow, true);
    }

    fn visit_node(&mut self, n: &Node<'a>, ctx: &Ctx, allow: bool, tail: bool) {
        match n {
            Node::Leaf(p) => self.visit(p.clone(), ctx),
            Node::Bin { op, left, right } => {
                if allow {
                    self.bin_patterns(n);
                } else {
                    let was = self.dry;
                    self.dry = true;
                    self.bin_patterns(n);
                    self.dry = was;
                }
                let level = precedence(op.as_rule(), op.as_str());
                let tight = level >= 4;
                let lctx = Ctx {
                    tight,
                    nonempty: ctx.nonempty.clone(),
                    ..Ctx::default()
                };
                let mut rctx = Ctx {
                    tight,
                    nonempty: ctx.nonempty.clone(),
                    stage: ctx.stage && tail,
                    else_if: false,
                };
                if op.as_rule() == Rule::pipe_op && right.leaf().is_some_and(bare_lambda) {
                    rctx.stage = true;
                }
                if allow
                    && self.target == Target::FirstLast
                    && (op.as_str() == "&&" || op.as_str() == "||")
                {
                    let mut found = Vec::new();
                    facts(left, op.as_str() == "&&", &mut found);
                    for c in found {
                        if !node_rebinds(right, &c[0]) {
                            rctx.nonempty.push(c);
                        }
                    }
                }
                self.visit_node(left, &lctx, allow, false);
                self.visit_node(right, &rctx, allow, tail);
            }
        }
    }

    /// `guards` on `&&` and `||`.
    fn bin_patterns(&mut self, n: &Node<'a>) {
        if self.target != Target::Guards {
            return;
        }
        let Some((left, op, right)) = n.bin() else {
            return;
        };
        let and = match op {
            "&&" => true,
            "||" => false,
            _ => return,
        };
        // The trailing guards of the left side: `p != ()` under `&&`,
        // `p == ()` under `||`.
        let mut guards: Vec<(Vec<String>, usize)> = Vec::new();
        let mut l = left;
        loop {
            if let Some(p) = unit_test(l, !and) {
                guards.push((p, l.start()));
                break;
            }
            match l.bin() {
                Some((ll, o, lr)) if o == op => match unit_test(lr, !and) {
                    Some(p) => {
                        guards.push((p, lr.start()));
                        l = ll;
                    }
                    None => break,
                },
                _ => break,
            }
        }
        if guards.is_empty() {
            return;
        }
        // The right side: a test `()` answers as the guard does.
        //
        // Under `&&` (the guard's false is the answer when p is ()):
        //   `c != ()` — R1 makes c () when p is (), so `c != ()` is false;
        //   `c == v`, v a literal that is not () — `() == v` is false (R5).
        // Under `||` (the guard's true is the answer when p is ()):
        //   `c == ()` — true; `c != v`, v a non-() literal — true.
        // When p is not (), the guard is true (false) and `&&` (`||`)
        // answers its right side, which is the rewrite. Reading c has no
        // effect (R6), so evaluating it when p is () changes nothing,
        // and a c that raises raises in both (p not () reaches it too).
        let (c, cmp_ok, why) = classify_right(right, and);
        let Some(c) = c else {
            return;
        };
        // only guards of this read's path come off; the run stops at the
        // first that is not one
        let mut take = 0;
        for (p, _) in &guards {
            if is_prefix(p, &c) {
                take += 1;
            } else {
                break;
            }
        }
        if take == 0 {
            return;
        }
        if !cmp_ok {
            if let Some(why) = why {
                self.skip(n.start(), why);
            }
            return;
        }
        let from = guards[take - 1].1;
        let to = right.start();
        if self.has_comment(from, to) {
            self.skip(from, SKIP_COMMENT);
            return;
        }
        self.edit(
            from,
            to,
            Vec::new(),
            if and {
                "x != () && x.f == v"
            } else {
                "x == () || x.f != v"
            },
        );
    }

    fn visit_if(&mut self, p: Pair<'a, Rule>, ctx: &Ctx) {
        let kids: Vec<Pair<'a, Rule>> = p.clone().into_inner().collect();
        // if_kw, cond, then, [else_kw, else]
        let cond = kids.get(1).cloned();
        let then = kids.get(2).cloned();
        let els = kids.get(4).cloned();
        let (Some(cond), Some(then)) = (cond, then) else {
            for k in kids {
                self.visit(k, &ctx.inner());
            }
            return;
        };
        let allow = !ctx.stage;
        if allow {
            match self.target {
                Target::Guards => self.if_guard(&p, &cond, &then, els.as_ref(), ctx),
                Target::Defaults => self.if_default(&p, &cond, &then, els.as_ref(), ctx),
                Target::FirstLast => self.if_first_last(&p, &cond, &then, els.as_ref(), ctx),
                Target::Units => {}
            }
        } else {
            let was = self.dry;
            self.dry = true;
            match self.target {
                Target::Guards => self.if_guard(&p, &cond, &then, els.as_ref(), ctx),
                Target::Defaults => self.if_default(&p, &cond, &then, els.as_ref(), ctx),
                Target::FirstLast => self.if_first_last(&p, &cond, &then, els.as_ref(), ctx),
                Target::Units => {}
            }
            self.dry = was;
        }
        // the branches, with what the condition proves
        let mut then_facts = Vec::new();
        let mut else_facts = Vec::new();
        if self.target == Target::FirstLast
            && let Some(t) = tree_of(cond.clone())
        {
            facts(&t, true, &mut then_facts);
            facts(&t, false, &mut else_facts);
        }
        self.visit(cond.clone(), &ctx.inner());
        let then_tail = els.is_none();
        let mut tc = ctx.inner();
        tc.stage = ctx.stage && then_tail;
        if !(ctx.stage && then_tail) {
            for c in then_facts {
                if !rebinds(&then, &c[0]) {
                    tc.nonempty.push(c);
                }
            }
        }
        self.visit(then, &tc);
        if let Some(e) = els {
            let mut ec = ctx.inner();
            ec.stage = ctx.stage;
            ec.else_if = e.as_rule() == Rule::if_expr;
            if !ctx.stage {
                for c in else_facts {
                    if !rebinds(&e, &c[0]) {
                        ec.nonempty.push(c);
                    }
                }
            }
            self.visit(e, &ec);
        }
    }

    /// `if x == () => () else => x.f` → `x.f`, and its `||`/`&&` forms.
    ///
    /// (A) `if p1 == () || … || pn == () => () else => c`, each p a
    /// prefix of the chain c: when some p is (), the `if` answers ();
    /// so does c, by R1 (a field of () is ()). When none is, it answers
    /// c. Reading c has no effect (R6); a c that raises (a field of a
    /// number) raises in both, since no p is () on that path.
    /// (B) `if p1 != () && … => c else => ()` (or with no else) is (A)
    /// turned round.
    fn if_guard(
        &mut self,
        p: &Pair<'a, Rule>,
        cond: &Pair<'a, Rule>,
        then: &Pair<'a, Rule>,
        els: Option<&Pair<'a, Rule>>,
        ctx: &Ctx,
    ) {
        let Some(t) = tree_of(cond.clone()) else {
            return;
        };
        let (guards, read) = if is_unit(then) && els.is_some() {
            match all_of(&t, "||", &|n| unit_test(n, true)) {
                Some(g) => (g, els.cloned().expect("else")),
                None => return,
            }
        } else if els.is_none_or(is_unit) {
            match all_of(&t, "&&", &|n| unit_test(n, false)) {
                Some(g) => (g, then.clone()),
                None => return,
            }
        } else {
            return;
        };
        if read.as_rule() != Rule::expr {
            return;
        }
        let Some(c) = chain(&read) else {
            if let Some(root) = calls_on_path(&read)
                && guards.iter().any(|g| g[0] == root)
            {
                self.skip(p.as_span().start(), SKIP_CALL_ON_PATH);
            }
            return;
        };
        if !guards.iter().all(|g| is_prefix(g, &c)) {
            return;
        }
        let (ps, pe) = self.span(p);
        let (rs, re) = self.span(&strip(read.clone()));
        if self.has_comment(ps, rs) || self.has_comment(re, pe) {
            self.skip(ps, SKIP_COMMENT);
            return;
        }
        let piece = self.src_piece(rs, re);
        self.edit_if(ctx, ps, pe, vec![piece], "if x == () => () else => x.f");
    }

    /// `if x == () => d else => x` → `x ?? d`, and `if x != () => x else
    /// => d`.
    ///
    /// When x is (), the `if` answers d, evaluating it then and only
    /// then; `x ?? d` does the same (R2). When x is not (), both answer
    /// x. x is a name or a field chain, so reading it twice or once is
    /// the same (R6).
    fn if_default(
        &mut self,
        p: &Pair<'a, Rule>,
        cond: &Pair<'a, Rule>,
        then: &Pair<'a, Rule>,
        els: Option<&Pair<'a, Rule>>,
        ctx: &Ctx,
    ) {
        let Some(els) = els else {
            return;
        };
        let Some(t) = tree_of(cond.clone()) else {
            return;
        };
        let (x, d, kept) = if let Some(x) = unit_test(&t, true) {
            (x, then, els)
        } else if let Some(x) = unit_test(&t, false) {
            (x, els, then)
        } else {
            return;
        };
        if kept.as_rule() != Rule::expr || chain(kept).as_ref() != Some(&x) {
            return;
        }
        let at = p.as_span().start();
        if d.as_rule() == Rule::block {
            self.skip(at, SKIP_DEFAULT_BLOCK);
            return;
        }
        if d.as_rule() == Rule::if_expr {
            // `else if`: the default is the rest of the chain
        } else if d.as_rule() != Rule::expr
            || d.clone().into_inner().next().map(|k| k.as_rule()) != Some(Rule::binary_expr)
        {
            self.skip(at, SKIP_DEFAULT_JUMP);
            return;
        }
        if is_unit(d) {
            self.skip(at, SKIP_DEFAULT_UNIT);
            return;
        }
        let (ps, pe) = self.span(p);
        let (ds, de) = self.span(d);
        let (ks, ke) = self.span(&strip(kept.clone()));
        // dropped: everything but x (kept) and d
        let (a, b) = if ds < ks {
            ((ds, de), (ks, ke))
        } else {
            ((ks, ke), (ds, de))
        };
        if self.has_comment(ps, a.0) || self.has_comment(a.1, b.0) || self.has_comment(b.1, pe) {
            self.skip(at, SKIP_COMMENT);
            return;
        }
        let parens = d.as_rule() == Rule::if_expr || crate::tools::fix::default_needs_parens(d);
        let mut pieces = Vec::new();
        if ctx.tight {
            pieces.push(Piece::Text("(".into()));
        }
        pieces.push(self.src_piece(ks, ke));
        pieces.push(Piece::Text(" ?? ".into()));
        if parens {
            pieces.push(Piece::Text("(".into()));
        }
        pieces.push(self.src_piece(ds, de));
        if parens {
            pieces.push(Piece::Text(")".into()));
        }
        if ctx.tight {
            pieces.push(Piece::Text(")".into()));
        }
        self.edit_if(ctx, ps, pe, pieces, "if x == () => d else => x");
    }

    /// `if len(xs) == 0 => () else => xs[len(xs) - 1]` → `last(xs)`, and
    /// the `first` and `> 0` forms.
    ///
    /// When xs is empty the `if` answers (), as `last(xs)` does (R3);
    /// when it is not, `xs[len(xs) - 1]` is its last element, as
    /// `last(xs)` is — for a list, a tuple or a string, which is what
    /// this needs proven of xs (`len` and `[i]` also take Bytes, which
    /// `last` refuses). A value `len` refuses raises at the test in the
    /// original and in `last` in the rewrite.
    fn if_first_last(
        &mut self,
        p: &Pair<'a, Rule>,
        cond: &Pair<'a, Rule>,
        then: &Pair<'a, Rule>,
        els: Option<&Pair<'a, Rule>>,
        ctx: &Ctx,
    ) {
        let Some(t) = tree_of(cond.clone()) else {
            return;
        };
        let (c, read) = if let Some(c) = empty_test(&t) {
            match els {
                Some(e) if is_unit(then) => (c, e.clone()),
                _ => return,
            }
        } else if let Some(c) = nonempty_test(&t) {
            if !els.is_none_or(is_unit) {
                return;
            }
            (c, then.clone())
        } else {
            return;
        };
        if read.as_rule() != Rule::expr {
            return;
        }
        let Some((which, chain_span)) = end_read(&read, &c) else {
            return;
        };
        let at = p.as_span().start();
        if !self.may_call_first_last(at, &c) {
            let read_at = self.abs(strip(read.clone()).as_span().start());
            self.noted.push(read_at);
            return;
        }
        let (ps, pe) = self.span(p);
        let (rs, re) = self.span(&strip(read.clone()));
        if self.has_comment(ps, rs) || self.has_comment(re, pe) {
            self.skip(at, SKIP_COMMENT);
            return;
        }
        let pieces = vec![
            Piece::Text(format!("{which}(")),
            self.src_piece(chain_span.0, chain_span.1),
            Piece::Text(")".into()),
        ];
        self.edit_if(
            ctx,
            ps,
            pe,
            pieces,
            if which == "last" {
                "if len(xs) == 0 => () else => xs[len(xs) - 1]"
            } else {
                "if len(xs) > 0 => xs[0] else => ()"
            },
        );
    }

    /// The checks every first/last rewrite shares; a skip is counted.
    fn may_call_first_last(&mut self, at: usize, c: &[String]) -> bool {
        if self.shadowed {
            self.skip(at, SKIP_SHADOW_FL);
            return false;
        }
        if c.len() != 1 || !self.scopes.known(&c[0]) {
            self.skip(at, SKIP_NOT_SEQUENCE);
            return false;
        }
        true
    }

    fn visit_call(&mut self, p: Pair<'a, Rule>, ctx: &Ctx) {
        let kids: Vec<Pair<'a, Rule>> = p.clone().into_inner().collect();
        match self.target {
            Target::FirstLast => self.call_first_last(&p, &kids, ctx),
            Target::Units => self.call_units(&p, &kids),
            _ => {}
        }
        let single = kids.len() == 1;
        for k in kids {
            let c = if single {
                ctx.clone()
            } else {
                let mut c = ctx.inner();
                c.tight = true;
                c
            };
            self.visit(k, &c);
        }
    }

    /// A guarded `xs[len(xs) - 1]` → `last(xs)`, and `xs[0]` →
    /// `first(xs)`: where xs is proven non-empty (the branch of a length
    /// test, the right of `len(xs) > 0 &&`), the element is the one
    /// `last`/`first` answers (R3), for a list, tuple or string.
    fn call_first_last(&mut self, p: &Pair<'a, Rule>, kids: &[Pair<'a, Rule>], ctx: &Ctx) {
        // primary, then the chain's fields, then the index
        let head = strip(kids[0].clone());
        if head.as_rule() != Rule::identifier {
            return;
        }
        let mut c = vec![head.as_str().to_string()];
        let mut i = 1;
        while i < kids.len() && kids[i].as_rule() == Rule::member_name {
            c.push(kids[i].as_str().to_string());
            i += 1;
        }
        let Some(index) = kids.get(i) else {
            return;
        };
        if index.as_rule() != Rule::expr {
            return;
        }
        let which = if int_literal(index) == Some(0) {
            "first"
        } else if is_last_index(index, &c) {
            "last"
        } else {
            return;
        };
        let start = p.as_span().start();
        let chain_end = kids[i - 1].as_span().end();
        let Some(close) = self.src[index.as_span().end()..].find(']') else {
            return;
        };
        let end = index.as_span().end() + close + 1;
        if !self.src[index.as_span().end()..end - 1].trim().is_empty() {
            return;
        }
        let guarded = ctx.nonempty.contains(&c);
        if !guarded {
            if which == "last" {
                self.skip(start, SKIP_UNGUARDED);
            }
            return;
        }
        if self.noted.contains(&self.abs(start)) {
            return;
        }
        if !self.may_call_first_last(start, &c) {
            return;
        }
        if self.has_comment(chain_end, end) {
            self.skip(start, SKIP_COMMENT);
            return;
        }
        let pieces = vec![
            Piece::Text(format!("{which}(")),
            self.src_piece(start, chain_end),
            Piece::Text(")".into()),
        ];
        self.edit(
            start,
            end,
            pieces,
            if which == "last" {
                "guarded xs[len(xs) - 1]"
            } else {
                "guarded xs[0]"
            },
        );
    }

    /// `to_int(math.floor(e))` → `math.floor(e)`: floor, ceil, round and
    /// trunc answer an Int or raise (R4), and `to_int` of an Int is that
    /// Int, so the outer call changes nothing; a raise is the same raise,
    /// since `math.…` raises before `to_int` is reached.
    fn call_units(&mut self, p: &Pair<'a, Rule>, kids: &[Pair<'a, Rule>]) {
        if kids.len() < 2 || kids[1].as_rule() != Rule::function_call {
            return;
        }
        let head = strip(kids[0].clone());
        if head.as_rule() != Rule::identifier || head.as_str() != "to_int" {
            return;
        }
        let Some(args) = call_args(&kids[1]) else {
            return;
        };
        let [arg] = args.as_slice() else {
            return;
        };
        let inner = strip(arg.clone());
        if inner.as_rule() != Rule::call_expr {
            return;
        }
        let ik: Vec<_> = inner.clone().into_inner().collect();
        let is_math = ik.len() == 3
            && strip(ik[0].clone()).as_str() == "math"
            && strip(ik[0].clone()).as_rule() == Rule::identifier
            && ik[1].as_rule() == Rule::member_name
            && matches!(ik[1].as_str(), "floor" | "ceil" | "round" | "trunc")
            && ik[2].as_rule() == Rule::function_call;
        if !is_math {
            return;
        }
        let start = p.as_span().start();
        if call_args(&ik[2]).is_none_or(|a| a.len() != 1) {
            self.skip(start, SKIP_ROUND_ARGS);
            return;
        }
        if self.shadowed {
            self.skip(start, SKIP_SHADOW_UNITS);
            return;
        }
        let call_end = kids[1].as_span().end();
        let (is, ie) = self.span(&inner);
        if self.has_comment(start, is) || self.has_comment(ie, call_end) {
            self.skip(start, SKIP_COMMENT);
            return;
        }
        let piece = self.src_piece(is, ie);
        self.edit(start, call_end, vec![piece], "to_int(math.floor(e))");
    }
}

/// `xs[len(xs) - 1]` or `xs[0]` reading the chain c: which, and the
/// chain's span.
fn end_read(read: &Pair<'_, Rule>, c: &[String]) -> Option<(&'static str, (usize, usize))> {
    let s = strip(read.clone());
    if s.as_rule() != Rule::call_expr {
        return None;
    }
    let kids: Vec<_> = s.into_inner().collect();
    let n = kids.len();
    if n < 2 || kids[n - 1].as_rule() != Rule::expr {
        return None;
    }
    let head = strip(kids[0].clone());
    if head.as_rule() != Rule::identifier {
        return None;
    }
    let mut names = vec![head.as_str().to_string()];
    for k in &kids[1..n - 1] {
        if k.as_rule() != Rule::member_name {
            return None;
        }
        names.push(k.as_str().to_string());
    }
    if names != c {
        return None;
    }
    let index = &kids[n - 1];
    let which = if int_literal(index) == Some(0) {
        "first"
    } else if is_last_index(index, c) {
        "last"
    } else {
        return None;
    };
    Some((
        which,
        (kids[0].as_span().start(), kids[n - 2].as_span().end()),
    ))
}

/// `len(c) - 1`.
fn is_last_index(index: &Pair<'_, Rule>, c: &[String]) -> bool {
    let Some(t) = tree_of(index.clone()) else {
        return false;
    };
    let Some((l, op, r)) = t.bin() else {
        return false;
    };
    op == "-" && node_int(r) == Some(1) && len_of(l).as_deref() == Some(c)
}

/// A lambda right after `|>` whose body the parser cuts: a bare lambda
/// whose body (as the grammar reads it) has a `|>` in it.
fn bare_lambda(p: &Pair<'_, Rule>) -> bool {
    p.as_rule() == Rule::unary_expr && {
        let s = strip(p.clone());
        s.as_rule() == Rule::lambda
            && s.into_inner()
                .find(|k| k.as_rule() == Rule::expr)
                .is_some_and(|body| body.as_str().contains("|>"))
    }
}

fn node_rebinds(n: &Node<'_>, name: &str) -> bool {
    match n {
        Node::Leaf(p) => rebinds(p, name),
        Node::Bin { left, right, .. } => node_rebinds(left, name) || node_rebinds(right, name),
    }
}

/// The right side of a guarded `&&` (`and`) or `||`: the chain it reads,
/// whether it answers as the guard does when the chain is (), and if not,
/// why.
fn classify_right(
    right: &Node<'_>,
    and: bool,
) -> (Option<Vec<String>>, bool, Option<&'static str>) {
    if let Some(c) = node_chain(right) {
        return (Some(c), false, Some(SKIP_BOOL_READ));
    }
    let Some((l, op, r)) = right.bin() else {
        return (None, false, None);
    };
    let (c, other) = match (node_chain(l), node_chain(r)) {
        (Some(c), _) => (c, r),
        (None, Some(c)) => (c, l),
        _ => return (None, false, None),
    };
    // `c != ()` under && and `c == ()` under ||
    let unit_op = if and { "!=" } else { "==" };
    // `c == v` under && and `c != v` under ||
    let lit_op = if and { "==" } else { "!=" };
    if node_is_unit(other) {
        return (
            Some(c),
            op == unit_op,
            (op != unit_op).then_some(SKIP_OTHER_CMP),
        );
    }
    if op == lit_op {
        if node_literal(other) {
            return (Some(c), true, None);
        }
        return (Some(c), false, Some(SKIP_MAYBE_UNIT));
    }
    if matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=") {
        return (Some(c), false, Some(SKIP_OTHER_CMP));
    }
    (None, false, None)
}

// ── the safety gate: each rewritten form read back as its original ────

fn normalized(program: &crate::ast::Program, target: Target) -> serde_json::Value {
    let mut v = serde_json::to_value(program).unwrap_or_default();
    normalize(&mut v, target);
    v
}

fn normalize(v: &mut serde_json::Value, target: Target) {
    use serde_json::Value as J;
    match v {
        J::Object(o) => {
            for x in o.values_mut() {
                normalize(x, target);
            }
        }
        J::Array(a) => {
            for x in a.iter_mut() {
                normalize(x, target);
            }
        }
        _ => {}
    }
    let J::Object(o) = v else { return };
    o.remove("line");
    o.remove("column");
    o.remove("span");
    o.remove("name_span");
    if o.len() != 1 {
        return;
    }
    if let Some(stmt) = o.get("Located").and_then(|l| l.get("stmt")).cloned() {
        *v = stmt;
        return;
    }
    // Rewrite to a fixed point at this node: one rule's answer may be
    // another's input.
    for _ in 0..8 {
        let next = match target {
            Target::Guards => j_guards(v),
            Target::Defaults => j_defaults(v),
            Target::FirstLast => j_first_last(v),
            Target::Units => j_units(v),
        };
        match next {
            Some(n) => *v = n,
            None => break,
        }
    }
}

mod j {
    use serde_json::{Value as J, json};

    pub fn tag<'v>(v: &'v J, name: &str) -> Option<&'v J> {
        v.as_object().filter(|o| o.len() == 1)?.get(name)
    }

    pub fn is_unit(v: &J) -> bool {
        tag(v, "Tuple")
            .and_then(|t| t.as_array())
            .is_some_and(|a| a.is_empty())
    }

    /// A name and its fields, root first.
    pub fn chain(v: &J) -> Option<Vec<String>> {
        if let Some(n) = tag(v, "Identifier").and_then(|n| n.as_str()) {
            return Some(vec![n.to_string()]);
        }
        let f = tag(v, "FieldAccess")?;
        let mut c = chain(f.get("object")?)?;
        c.push(f.get("field")?.as_str()?.to_string());
        Some(c)
    }

    pub fn binop(v: &J) -> Option<(&J, &str, &J)> {
        let b = tag(v, "BinaryOp")?;
        Some((b.get("left")?, b.get("op")?.as_str()?, b.get("right")?))
    }

    pub fn unit_test(v: &J, eq: bool) -> Option<Vec<String>> {
        let (l, op, r) = binop(v)?;
        if op != if eq { "Equal" } else { "NotEqual" } {
            return None;
        }
        if is_unit(r) {
            chain(l)
        } else if is_unit(l) {
            chain(r)
        } else {
            None
        }
    }

    pub fn all_of(
        v: &J,
        op: &str,
        f: &dyn Fn(&J) -> Option<Vec<String>>,
    ) -> Option<Vec<Vec<String>>> {
        if let Some((l, o, r)) = binop(v)
            && o == op
        {
            let mut out = all_of(l, op, f)?;
            out.push(f(r)?);
            return Some(out);
        }
        Some(vec![f(v)?])
    }

    pub fn is_literal_not_unit(v: &J) -> bool {
        let Some(o) = v.as_object().filter(|o| o.len() == 1) else {
            return false;
        };
        let (k, inner) = o.iter().next().expect("one");
        match k.as_str() {
            "Integer" | "Float" | "String" | "Boolean" | "RawString" | "Char" => true,
            "UnaryOp" => {
                inner.get("op").and_then(|o| o.as_str()) == Some("Negate")
                    && inner
                        .get("operand")
                        .is_some_and(|x| tag(x, "Integer").is_some() || tag(x, "Float").is_some())
            }
            "Literal" => true,
            _ => false,
        }
    }

    pub fn call1(v: &J) -> Option<(&J, &J)> {
        let c = tag(v, "Call")?;
        let args = c.get("arguments")?.as_array()?;
        let [a] = args.as_slice() else {
            return None;
        };
        Some((c.get("callee")?, tag(a, "Positional")?))
    }

    pub fn ident(v: &J) -> Option<&str> {
        tag(v, "Identifier")?.as_str()
    }

    pub fn call_named(name: &str, arg: J) -> J {
        json!({ "Call": { "callee": { "Identifier": name }, "arguments": [ { "Positional": arg } ] } })
    }

    pub fn len_of(v: &J) -> Option<J> {
        let (callee, arg) = call1(v)?;
        (ident(callee)? == "len").then(|| arg.clone())
    }

    pub fn int(v: &J) -> Option<i64> {
        tag(v, "Integer")?.as_i64()
    }
}

fn j_guards(v: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(i) = j::tag(v, "If") {
        let cond = i.get("condition")?;
        let then = i.get("then_branch")?;
        let els = i.get("else_branch").filter(|e| !e.is_null());
        // (A)
        if j::is_unit(then)
            && let Some(e) = els
            && let Some(c) = j::chain(e)
            && let Some(g) = j::all_of(cond, "Or", &|n| j::unit_test(n, true))
            && g.iter().all(|p| is_prefix(p, &c))
        {
            return Some(e.clone());
        }
        // (B)
        if els.is_none_or(j::is_unit)
            && let Some(c) = j::chain(then)
            && let Some(g) = j::all_of(cond, "And", &|n| j::unit_test(n, false))
            && g.iter().all(|p| is_prefix(p, &c))
        {
            return Some(then.clone());
        }
        return None;
    }
    let (left, op, right) = j::binop(v)?;
    let and = match op {
        "And" => true,
        "Or" => false,
        _ => return None,
    };
    // the right side's chain, when it answers as the guard does
    let (rl, rop, rr) = j::binop(right)?;
    let (c, other) = match (j::chain(rl), j::chain(rr)) {
        (Some(c), _) => (c, rr),
        (None, Some(c)) => (c, rl),
        _ => return None,
    };
    let unit_op = if and { "NotEqual" } else { "Equal" };
    let lit_op = if and { "Equal" } else { "NotEqual" };
    let ok =
        (j::is_unit(other) && rop == unit_op) || (rop == lit_op && j::is_literal_not_unit(other));
    if !ok {
        return None;
    }
    // take the trailing guards off the left
    let guard = |n: &serde_json::Value| j::unit_test(n, !and).filter(|p| is_prefix(p, &c));
    if guard(left).is_some() {
        return Some(right.clone());
    }
    let mut l = left;
    let mut took = false;
    while let Some((ll, o, lr)) = j::binop(l) {
        if o != op || guard(lr).is_none() {
            break;
        }
        took = true;
        l = ll;
        if guard(l).is_some() {
            return Some(right.clone());
        }
    }
    if !took {
        return None;
    }
    Some(serde_json::json!({ "BinaryOp": { "left": l.clone(), "op": op, "right": right.clone() } }))
}

fn j_defaults(v: &serde_json::Value) -> Option<serde_json::Value> {
    let i = j::tag(v, "If")?;
    let cond = i.get("condition")?;
    let then = i.get("then_branch")?;
    let els = i.get("else_branch").filter(|e| !e.is_null())?;
    let (x, d) = if let Some(x) = j::unit_test(cond, true) {
        (x, then)
    } else {
        let x = j::unit_test(cond, false)?;
        (x, els)
    };
    let kept = if std::ptr::eq(d, then) { els } else { then };
    if j::chain(kept).as_ref() != Some(&x) || j::is_unit(d) {
        return None;
    }
    Some(
        serde_json::json!({ "BinaryOp": { "left": kept.clone(), "op": "Coalesce", "right": d.clone() } }),
    )
}

fn j_first_last(v: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(ix) = j::tag(v, "Index") {
        let object = ix.get("object")?;
        let index = ix.get("index")?;
        j::chain(object)?;
        if j::int(index) == Some(0) {
            return Some(j::call_named("first", object.clone()));
        }
        let (l, op, r) = j::binop(index)?;
        if op == "Subtract" && j::int(r) == Some(1) && j::len_of(l).as_ref() == Some(object) {
            return Some(j::call_named("last", object.clone()));
        }
        return None;
    }
    let i = j::tag(v, "If")?;
    let cond = i.get("condition")?;
    let then = i.get("then_branch")?;
    let els = i.get("else_branch").filter(|e| !e.is_null());
    let (l, op, r) = j::binop(cond)?;
    let (c, k, op) = match (j::len_of(l), j::len_of(r)) {
        (Some(c), _) => (c, j::int(r)?, op.to_string()),
        (None, Some(c)) => (
            c,
            j::int(l)?,
            match op {
                "LessThan" => "GreaterThan",
                "GreaterThan" => "LessThan",
                "LessThanEqual" => "GreaterThanEqual",
                "GreaterThanEqual" => "LessThanEqual",
                o => o,
            }
            .to_string(),
        ),
        _ => return None,
    };
    let empty = matches!(
        (op.as_str(), k),
        ("Equal", 0) | ("LessThan", 1) | ("LessThanEqual", 0)
    );
    let nonempty = matches!(
        (op.as_str(), k),
        ("GreaterThan", 0) | ("NotEqual", 0) | ("GreaterThanEqual", 1)
    );
    let read = if empty && j::is_unit(then) {
        els?
    } else if nonempty && els.is_none_or(j::is_unit) {
        then
    } else {
        return None;
    };
    let (callee, arg) = j::call1(read)?;
    let name = j::ident(callee)?;
    ((name == "first" || name == "last") && *arg == c).then(|| read.clone())
}

fn j_units(v: &serde_json::Value) -> Option<serde_json::Value> {
    let (callee, arg) = j::call1(v)?;
    if j::ident(callee)? != "to_int" {
        return None;
    }
    let (inner_callee, _) = j::call1(arg)?;
    let f = j::tag(inner_callee, "FieldAccess")?;
    let object = f.get("object")?;
    let field = f.get("field")?.as_str()?;
    (j::ident(object)? == "math" && matches!(field, "floor" | "ceil" | "round" | "trunc"))
        .then(|| arg.clone())
}

// ── the command ───────────────────────────────────────────────────────

/// `olang fix <target>`: every `.ol` file under `paths`, fixed in place —
/// or, with `check`, reported with a non-zero exit when something would
/// change, or with `dry_run`, reported (each skip too) with nothing
/// written.
pub fn run(target: Target, paths: &[PathBuf], check: bool, dry_run: bool, skip: &[PathBuf]) -> i32 {
    let mut kinds: Vec<(&'static str, usize)> = Vec::new();
    let mut reasons: Vec<(&'static str, usize)> = Vec::new();
    let mut files_changed = 0;
    let mut failed = 0;
    let skip: Vec<PathBuf> = skip.iter().filter_map(|p| p.canonicalize().ok()).collect();
    let write = !check && !dry_run;
    for path in paths {
        for file in super::discover_ol_files(path) {
            if file
                .canonicalize()
                .is_ok_and(|c| skip.iter().any(|s| c.starts_with(s)))
            {
                println!("  skipped {}", file.display());
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&file) else {
                continue;
            };
            match fix(&src, target) {
                Ok((fixed, report)) => {
                    if dry_run {
                        for (line, why) in &report.skipped {
                            println!("  left {}:{}: {}", display(&file), line, why);
                        }
                    }
                    if fixed != src {
                        files_changed += 1;
                        println!(
                            "  {} {} ({})",
                            if write { "fixed" } else { "would fix" },
                            display(&file),
                            report.count()
                        );
                        if write && std::fs::write(&file, &fixed).is_err() {
                            eprintln!("  could not write {}", file.display());
                            failed += 1;
                        }
                    }
                    for (_, k) in &report.rewritten {
                        bump(&mut kinds, k);
                    }
                    for (_, why) in &report.skipped {
                        bump(&mut reasons, why);
                    }
                }
                Err(why) => {
                    failed += 1;
                    println!("  left {} — {}", file.display(), why);
                }
            }
        }
    }
    let total: usize = kinds.iter().map(|(_, n)| n).sum();
    println!(
        "olang fix {}: {} rewritten in {} file{}{}{}",
        target.name(),
        total,
        files_changed,
        if files_changed == 1 { "" } else { "s" },
        if kinds.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                kinds
                    .iter()
                    .map(|(k, n)| format!("{n} `{k}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        if reasons.is_empty() {
            String::new()
        } else {
            format!(
                "; left: {}",
                reasons
                    .iter()
                    .map(|(w, n)| format!("{n} ({w})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    if failed > 0 || (check && files_changed > 0) {
        1
    } else {
        0
    }
}

fn bump(v: &mut Vec<(&'static str, usize)>, k: &'static str) {
    match v.iter_mut().find(|(w, _)| *w == k) {
        Some((_, n)) => *n += 1,
        None => v.push((k, 1)),
    }
}

fn display(p: &Path) -> String {
    p.display().to_string()
}
