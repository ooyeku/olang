//! What completes inside data: a Frame's columns where a column is named
//! (`sales["`, `ods.select(sales, ["`, `ods.group_by(sales, "`, `… |>
//! ods.sort_by("`, an aggregation's column), a map's keys (`m["`,
//! `map_get(m, "`, `m.`). Read from the text before the caret, so it
//! answers while the line does not parse.
//!
//! The receiver is an expression's text (`sales`, `sales |>
//! ods.group_by("region", [["revenue", "sum", "amount"]])`); its fields
//! are read by following the expression's verbs from the fields of the
//! name it starts from, which the caller resolves: the language server
//! from the literals the program builds (`ods.frame([["a", …]])`,
//! `ods.read_csv("a,b\n…")`, `#{ "k": … }`), a notebook's kernel from the
//! values it holds.

/// A field: its name and its type's name (`Int`, `String`, …; empty when
/// it is not known).
pub type Field = (String, String);

/// What a receiver is, as far as its fields go.
#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    /// `frame`, `table` (a list of maps), `map` or `record`.
    pub kind: &'static str,
    pub fields: Vec<Field>,
}

/// What may complete at the caret.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Want {
    /// A Frame's column (an `ods` verb's argument).
    Columns,
    /// A map's key or a record's field (after `.`, `map_get(m, "`).
    Keys,
    /// Either (`x["`).
    Any,
}

/// A place in data the caret is at.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    /// The text of the expression whose fields complete.
    pub receiver: String,
    pub want: Want,
    /// What is typed of the field so far.
    pub typed: String,
    /// Inside a string (`"re`), rather than after a `.`.
    pub quoted: bool,
}

impl Fields {
    /// Whether these fields answer `want`.
    pub fn answers(&self, want: Want) -> bool {
        match want {
            Want::Any => true,
            Want::Columns => self.kind == "frame",
            Want::Keys => self.kind == "map" || self.kind == "record",
        }
    }
}

// ── tokens ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    /// A string the text ends inside: what it holds so far.
    Unclosed(String),
    Num(String),
    /// `(`, `[`, `{`, or `#` for `#{`.
    Open(char),
    Close(char),
    Comma,
    Colon,
    Pipe,
    Dot,
    Assign,
    Arrow,
    Newline,
    Other,
}

#[derive(Clone, Debug)]
struct T {
    tok: Tok,
    start: usize,
    end: usize,
}

fn tokenize(text: &str) -> Vec<T> {
    let b: Vec<(usize, char)> = text.char_indices().collect();
    let at = |i: usize| b.get(i).map(|x| x.1);
    let pos = |i: usize| b.get(i).map(|x| x.0).unwrap_or(text.len());
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i].1;
        let start = b[i].0;
        let push = |out: &mut Vec<T>, tok: Tok, end_i: usize| {
            out.push(T {
                tok,
                start,
                end: pos(end_i),
            })
        };
        if c == '\n' {
            push(&mut out, Tok::Newline, i + 1);
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == Some('/') {
            while i < b.len() && b[i].1 != '\n' {
                i += 1;
            }
        } else if c == '"' {
            let mut s = String::new();
            let mut j = i + 1;
            let mut closed = false;
            while j < b.len() {
                let d = b[j].1;
                if d == '\\' {
                    match at(j + 1) {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some(e) => s.push(e),
                        None => {}
                    }
                    j += 2;
                } else if d == '"' {
                    closed = true;
                    j += 1;
                    break;
                } else {
                    s.push(d);
                    j += 1;
                }
            }
            let tok = if closed {
                Tok::Str(s)
            } else {
                Tok::Unclosed(s)
            };
            push(&mut out, tok, j.min(b.len()));
            i = j;
        } else if c == '`' {
            let mut j = i + 1;
            while j < b.len() && b[j].1 != '`' {
                j += 1;
            }
            push(&mut out, Tok::Other, (j + 1).min(b.len()));
            i = j + 1;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < b.len() && (b[j].1.is_alphanumeric() || b[j].1 == '_') {
                j += 1;
            }
            let word: String = b[i..j].iter().map(|x| x.1).collect();
            push(&mut out, Tok::Ident(word), j);
            i = j;
        } else if c.is_ascii_digit() {
            let mut j = i;
            while j < b.len()
                && (b[j].1.is_ascii_alphanumeric()
                    || b[j].1 == '_'
                    || (b[j].1 == '.' && at(j + 1).is_some_and(|d| d.is_ascii_digit())))
            {
                j += 1;
            }
            let word: String = b[i..j].iter().map(|x| x.1).collect();
            push(&mut out, Tok::Num(word), j);
            i = j;
        } else {
            let two: String = [Some(c), at(i + 1)].iter().flatten().collect();
            let (tok, n) = match two.as_str() {
                "|>" => (Tok::Pipe, 2),
                "=>" => (Tok::Arrow, 2),
                "==" | "!=" | "<=" | ">=" | "&&" | "||" | ".." | "??" => (Tok::Other, 2),
                "#{" => (Tok::Open('#'), 2),
                _ => (
                    match c {
                        '(' | '[' | '{' => Tok::Open(c),
                        ')' | ']' | '}' => Tok::Close(c),
                        ',' => Tok::Comma,
                        ':' => Tok::Colon,
                        '.' => Tok::Dot,
                        '=' => Tok::Assign,
                        _ => Tok::Other,
                    },
                    1,
                ),
            };
            push(&mut out, tok, i + n);
            i += n;
        }
    }
    out
}

// ── the place ──────────────────────────────────────────────────────────

/// Whether the text before the caret ends inside a string.
pub fn in_string(prefix: &str) -> bool {
    matches!(
        tokenize(prefix).last(),
        Some(T {
            tok: Tok::Unclosed(_),
            ..
        })
    )
}

/// What the text before the caret (`prefix`) is completing in data, if
/// anything: inside a string where a column or a key is named, or after
/// `name.`.
pub fn context(prefix: &str) -> Option<Context> {
    let toks = tokenize(prefix);
    let last = toks.len().checked_sub(1)?;
    match &toks[last].tok {
        Tok::Unclosed(typed) => quoted_context(prefix, &toks, last, typed.clone()),
        Tok::Dot => dot_context(prefix, &toks, last, String::new()),
        Tok::Ident(w)
            if last > 0
                && toks[last - 1].tok == Tok::Dot
                && toks[last - 1].end == toks[last].start =>
        {
            dot_context(prefix, &toks, last - 1, w.clone())
        }
        _ => None,
    }
}

fn dot_context(prefix: &str, toks: &[T], dot: usize, typed: String) -> Option<Context> {
    let j = dot.checked_sub(1)?;
    let glued = matches!(
        toks[j].tok,
        Tok::Ident(_) | Tok::Close(')') | Tok::Close(']')
    ) && toks[j].end == toks[dot].start;
    let j = if glued { j } else { chain_above(toks, dot)? };
    let s = postfix_start(toks, j)?;
    Some(Context {
        receiver: prefix[toks[s].start..toks[j].end].to_string(),
        want: Want::Keys,
        typed,
        quoted: false,
    })
}

fn quoted_context(prefix: &str, toks: &[T], i: usize, typed: String) -> Option<Context> {
    // the brackets open where the string starts
    let mut stack: Vec<usize> = Vec::new();
    for (k, t) in toks[..i].iter().enumerate() {
        match t.tok {
            Tok::Open(_) => stack.push(k),
            Tok::Close(_) => {
                stack.pop();
            }
            _ => {}
        }
    }
    let prev = (0..i).rev().find(|&k| toks[k].tok != Tok::Newline)?;
    let top = *stack.last()?;
    let ctx = |receiver: String, want: Want| {
        Some(Context {
            receiver,
            want,
            typed: typed.clone(),
            quoted: true,
        })
    };
    // `x["`: a subscript
    if prev == top
        && toks[top].tok == Tok::Open('[')
        && top > 0
        && matches!(
            toks[top - 1].tok,
            Tok::Ident(_) | Tok::Close(')') | Tok::Close(']')
        )
        && toks[top - 1].end == toks[top].start
    {
        let s = postfix_start(toks, top - 1)?;
        return ctx(
            prefix[toks[s].start..toks[top - 1].end].to_string(),
            Want::Any,
        );
    }
    // otherwise the string must start an element: after `(`, `[` or `,`
    if !matches!(toks[prev].tok, Tok::Open('(') | Tok::Open('[') | Tok::Comma) {
        return None;
    }
    let elem = commas(toks, top, i);
    match toks[top].tok {
        Tok::Open('(') => {
            let call = call_at(prefix, toks, top)?;
            let want = string_arg(&call.callee, call.arg(elem))?;
            ctx(call.receiver(prefix, toks, top, i)?, want)
        }
        Tok::Open('[') => {
            let parent = *stack.get(stack.len().checked_sub(2)?)?;
            match toks[parent].tok {
                Tok::Open('(') => {
                    let call = call_at(prefix, toks, parent)?;
                    let want = list_arg(&call.callee, call.arg(commas(toks, parent, top)))?;
                    ctx(call.receiver(prefix, toks, parent, top)?, want)
                }
                // an aggregation's column: `[["out", "op", "`
                Tok::Open('[') if elem == 2 => {
                    let grand = *stack.get(stack.len().checked_sub(3)?)?;
                    if toks[grand].tok != Tok::Open('(') {
                        return None;
                    }
                    let call = call_at(prefix, toks, grand)?;
                    if call.callee != "ods.group_by" || call.arg(commas(toks, grand, parent)) != 2 {
                        return None;
                    }
                    ctx(call.receiver(prefix, toks, grand, parent)?, Want::Columns)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// How many commas at the bracket `open`'s own depth stand between it and
/// `upto`.
fn commas(toks: &[T], open: usize, upto: usize) -> usize {
    let mut depth = 0i32;
    let mut n = 0;
    for t in &toks[open + 1..upto] {
        match t.tok {
            Tok::Open(_) => depth += 1,
            Tok::Close(_) => depth -= 1,
            Tok::Comma if depth == 0 => n += 1,
            _ => {}
        }
    }
    n
}

/// A call whose `(` is at a token: its callee (`ods.select`) and whether
/// its frame comes from a pipe.
struct Call {
    callee: String,
    /// The pipe's token, when the call's first argument is piped in.
    pipe: Option<usize>,
}

impl Call {
    /// An argument's place counting the piped frame as the first.
    fn arg(&self, written: usize) -> usize {
        if self.pipe.is_some() {
            written + 1
        } else {
            written
        }
    }

    /// The text of the call's frame: the pipe's left side, or its first
    /// argument (`None` while the caret is in that argument).
    fn receiver(&self, prefix: &str, toks: &[T], open: usize, upto: usize) -> Option<String> {
        if let Some(p) = self.pipe {
            let end = (0..p).rev().find(|&k| toks[k].tok != Tok::Newline)?;
            let s = pipe_left_start(toks, p);
            return (s <= end).then(|| prefix[toks[s].start..toks[end].end].trim().to_string());
        }
        // the first argument: to the first comma at the call's depth
        let mut depth = 0i32;
        for k in open + 1..upto {
            match toks[k].tok {
                Tok::Open(_) => depth += 1,
                Tok::Close(_) => depth -= 1,
                Tok::Comma if depth == 0 => {
                    if k == open + 1 {
                        return None;
                    }
                    return Some(
                        prefix[toks[open + 1].start..toks[k - 1].end]
                            .trim()
                            .to_string(),
                    );
                }
                _ => {}
            }
        }
        None
    }
}

fn call_at(_prefix: &str, toks: &[T], open: usize) -> Option<Call> {
    let mut k = open.checked_sub(1)?;
    if toks[k].end != toks[open].start {
        return None;
    }
    let mut parts = Vec::new();
    loop {
        let Tok::Ident(w) = &toks[k].tok else {
            return None;
        };
        parts.push(w.clone());
        if k >= 2 && toks[k - 1].tok == Tok::Dot {
            k -= 2;
        } else {
            break;
        }
    }
    parts.reverse();
    let before = (0..k).rev().find(|&j| toks[j].tok != Tok::Newline);
    let pipe = before.filter(|&j| toks[j].tok == Tok::Pipe);
    Some(Call {
        callee: parts.join("."),
        pipe,
    })
}

/// What a string argument of `callee` at `arg` (the frame first) names.
fn string_arg(callee: &str, arg: usize) -> Option<Want> {
    match (callee, arg) {
        ("ods.group_by" | "ods.sort_by" | "ods.with_column", 1) => Some(Want::Columns),
        ("ods.pivot", 1..=3) => Some(Want::Columns),
        ("ods.join", 2) => Some(Want::Columns),
        ("map_get" | "map_has_key" | "map_set" | "map_remove", 1) => Some(Want::Keys),
        _ => None,
    }
}

/// What a list argument's strings of `callee` at `arg` name.
fn list_arg(callee: &str, arg: usize) -> Option<Want> {
    match (callee, arg) {
        ("ods.select" | "ods.drop" | "ods.distinct" | "ods.drop_null" | "ods.group_by", 1) => {
            Some(Want::Columns)
        }
        ("ods.unpivot", 1 | 2) => Some(Want::Columns),
        _ => None,
    }
}

/// Where the postfix expression ending at token `j` starts: a name, a
/// dotted path, calls and subscripts (`ods.head(sales, 8)`, `t["a"]`).
fn postfix_start(toks: &[T], j: usize) -> Option<usize> {
    let mut k = j;
    loop {
        match &toks[k].tok {
            Tok::Close(_) => {
                let open = matching_open(toks, k)?;
                if open > 0
                    && matches!(toks[open - 1].tok, Tok::Ident(_) | Tok::Close(_))
                    && toks[open - 1].end == toks[open].start
                {
                    k = open - 1;
                } else {
                    return Some(open);
                }
            }
            Tok::Ident(_) => {
                if k >= 2
                    && toks[k - 1].tok == Tok::Dot
                    && matches!(toks[k - 2].tok, Tok::Ident(_) | Tok::Close(_))
                {
                    k -= 2;
                } else if let Some(above) = (k >= 1 && toks[k - 1].tok == Tok::Dot)
                    .then(|| chain_above(toks, k - 1))
                    .flatten()
                {
                    k = above;
                } else {
                    return Some(k);
                }
            }
            _ => return None,
        }
    }
}

/// A chain's step on a line of its own (`    .filter(…)`): the dot first
/// on its line continues the value the line above ends with, which is
/// the receiver. The index of that value's last token.
fn chain_above(toks: &[T], dot: usize) -> Option<usize> {
    let mut k = dot.checked_sub(1)?;
    if toks[k].tok != Tok::Newline {
        return None;
    }
    while toks[k].tok == Tok::Newline {
        k = k.checked_sub(1)?;
    }
    matches!(toks[k].tok, Tok::Ident(_) | Tok::Close(')') | Tok::Close(']')).then_some(k)
}

fn matching_open(toks: &[T], close: usize) -> Option<usize> {
    let mut depth = 0i32;
    for k in (0..=close).rev() {
        match toks[k].tok {
            Tok::Close(_) => depth += 1,
            Tok::Open(_) => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// Where the left side of the pipe at token `p` starts: back to what
/// opens it (a bracket, a comma, `=`, `=>`, a keyword, or a line that
/// is no continuation of a chain).
fn pipe_left_start(toks: &[T], p: usize) -> usize {
    let mut depth = 0i32;
    let mut k = p;
    while k > 0 {
        k -= 1;
        match &toks[k].tok {
            Tok::Close(_) => depth += 1,
            Tok::Open(_) => {
                if depth == 0 {
                    return k + 1;
                }
                depth -= 1;
            }
            Tok::Comma | Tok::Assign | Tok::Arrow if depth == 0 => return k + 1,
            Tok::Ident(w)
                if depth == 0
                    && matches!(
                        w.as_str(),
                        "let" | "mut" | "return" | "if" | "else" | "for" | "in" | "while"
                    ) =>
            {
                return k + 1;
            }
            Tok::Newline if depth == 0 => {
                // the line after it goes on with `|>` (or `.`): one chain
                let next = (k + 1..toks.len()).find(|&j| toks[j].tok != Tok::Newline);
                if !next.is_some_and(|j| matches!(toks[j].tok, Tok::Pipe | Tok::Dot)) {
                    return k + 1;
                }
            }
            _ => {}
        }
    }
    0
}

// ── an expression's fields ─────────────────────────────────────────────

/// The fields of the expression `expr`, a name's own resolved by
/// `resolve`.
pub fn fields_of(expr: &str, resolve: &mut dyn FnMut(&str) -> Option<Fields>) -> Option<Fields> {
    let toks: Vec<T> = tokenize(expr)
        .into_iter()
        .filter(|t| t.tok != Tok::Newline)
        .collect();
    eval(&toks, resolve)
}

fn eval(ts: &[T], resolve: &mut dyn FnMut(&str) -> Option<Fields>) -> Option<Fields> {
    let parts = split_at(ts, |t| t.tok == Tok::Pipe);
    let mut acc = term(parts.first()?, resolve);
    for step in &parts[1..] {
        let (callee, args) = as_call(step)?;
        acc = apply(&callee, acc, &args, resolve);
    }
    acc
}

/// Tokens split where `cut` holds at depth 0.
fn split_at(ts: &[T], cut: impl Fn(&T) -> bool) -> Vec<&[T]> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut from = 0;
    for (k, t) in ts.iter().enumerate() {
        match t.tok {
            Tok::Open(_) => depth += 1,
            Tok::Close(_) => depth -= 1,
            _ if depth == 0 && cut(t) => {
                out.push(&ts[from..k]);
                from = k + 1;
            }
            _ => {}
        }
    }
    out.push(&ts[from..]);
    out
}

/// `a.b(args…)` as its callee and its arguments, when it is the whole of
/// `ts`.
fn as_call(ts: &[T]) -> Option<(String, Vec<&[T]>)> {
    let mut k = 0;
    let mut parts = Vec::new();
    loop {
        let Tok::Ident(w) = &ts.get(k)?.tok else {
            return None;
        };
        parts.push(w.clone());
        if ts.get(k + 1).is_some_and(|t| t.tok == Tok::Dot) {
            k += 2;
        } else {
            k += 1;
            break;
        }
    }
    if ts.get(k)?.tok != Tok::Open('(') || ts.last()?.tok != Tok::Close(')') {
        return None;
    }
    let inner = &ts[k + 1..ts.len() - 1];
    // the parentheses close the call, not an earlier group
    if matching_open(ts, ts.len() - 1)? != k {
        return None;
    }
    let args = if inner.is_empty() {
        Vec::new()
    } else {
        split_at(inner, |t| t.tok == Tok::Comma)
    };
    Some((parts.join("."), args))
}

fn term(ts: &[T], resolve: &mut dyn FnMut(&str) -> Option<Fields>) -> Option<Fields> {
    let ts = match ts.last() {
        Some(T {
            tok: Tok::Other, ..
        }) => &ts[..ts.len() - 1],
        _ => ts,
    };
    match ts {
        [
            T {
                tok: Tok::Ident(n), ..
            },
        ] => resolve(n),
        [
            T {
                tok: Tok::Open('('),
                ..
            },
            ..,
            T {
                tok: Tok::Close(')'),
                ..
            },
        ] if matching_open(ts, ts.len() - 1) == Some(0) => eval(&ts[1..ts.len() - 1], resolve),
        [
            T {
                tok: Tok::Open('#'),
                ..
            },
            ..,
        ] => map_literal(ts).map(|fields| Fields {
            kind: "map",
            fields,
        }),
        [
            T {
                tok: Tok::Open('['),
                ..
            },
            ..,
        ] => {
            let items = list_items(ts)?;
            let first = items.first()?;
            map_literal(first).map(|fields| Fields {
                kind: "table",
                fields,
            })
        }
        _ => {
            let (callee, args) = as_call(ts)?;
            match callee.as_str() {
                "unwrap" | "ods.collect" => eval(args.first()?, resolve),
                "ods.frame" => {
                    let pairs = list_items(args.first()?)?;
                    let fields = pairs
                        .iter()
                        .filter_map(|p| {
                            let inner = list_items(p)?;
                            let Tok::Str(name) = &inner.first()?.first()?.tok else {
                                return None;
                            };
                            let ty = inner.get(1).map(|c| column_type(c)).unwrap_or_default();
                            Some((name.clone(), ty))
                        })
                        .collect::<Vec<_>>();
                    (!fields.is_empty()).then_some(Fields {
                        kind: "frame",
                        fields,
                    })
                }
                "ods.read_csv" => {
                    let [
                        T {
                            tok: Tok::Str(text),
                            ..
                        },
                    ] = args.first()?
                    else {
                        return None;
                    };
                    let mut lines = text.lines();
                    let head = lines.next()?;
                    let row: Vec<&str> = lines
                        .next()
                        .map(|l| l.split(',').collect())
                        .unwrap_or_default();
                    let fields = head
                        .split(',')
                        .enumerate()
                        .map(|(j, n)| {
                            (
                                n.trim().to_string(),
                                row.get(j).map(|c| csv_type(c)).unwrap_or_default(),
                            )
                        })
                        .filter(|(n, _)| !n.is_empty())
                        .collect();
                    Some(Fields {
                        kind: "frame",
                        fields,
                    })
                }
                "ods.frame_from_records" | "ods.from_records" => {
                    let items = list_items(args.first()?)?;
                    map_literal(items.first()?).map(|fields| Fields {
                        kind: "frame",
                        fields,
                    })
                }
                _ => {
                    let frame = args.first().and_then(|a| eval(a, resolve));
                    apply(&callee, frame, args.get(1..).unwrap_or(&[]), resolve)
                }
            }
        }
    }
}

/// A verb applied to a frame's fields (`None`: not known), its other
/// arguments `args`.
fn apply(
    callee: &str,
    frame: Option<Fields>,
    args: &[&[T]],
    resolve: &mut dyn FnMut(&str) -> Option<Fields>,
) -> Option<Fields> {
    let type_of = |frame: &Option<Fields>, n: &str| {
        frame
            .as_ref()
            .and_then(|f| {
                f.fields
                    .iter()
                    .find(|(m, _)| m == n)
                    .map(|(_, t)| t.clone())
            })
            .unwrap_or_default()
    };
    let frame_of = |fields: Vec<Field>| {
        Some(Fields {
            kind: "frame",
            fields,
        })
    };
    match callee {
        "ods.filter" | "ods.sort_by" | "ods.head" | "ods.tail" | "ods.take" | "ods.distinct"
        | "ods.drop_null" | "ods.sample" | "ods.slice" | "ods.shuffle" | "ods.fill_nulls" => frame,
        "ods.select" => {
            let names = strings(args.first()?)?;
            frame_of(
                names
                    .iter()
                    .map(|n| (n.clone(), type_of(&frame, n)))
                    .collect(),
            )
        }
        "ods.drop" => {
            let names = strings(args.first()?)?;
            frame_of(
                frame?
                    .fields
                    .into_iter()
                    .filter(|(n, _)| !names.contains(n))
                    .collect(),
            )
        }
        "ods.with_column" => {
            let [
                T {
                    tok: Tok::Str(name),
                    ..
                },
            ] = args.first()?
            else {
                return None;
            };
            let mut fields = frame?.fields;
            if !fields.iter().any(|(n, _)| n == name) {
                fields.push((name.clone(), String::new()));
            }
            frame_of(fields)
        }
        "ods.rename" => {
            let pairs = map_pairs(args.first()?)?;
            frame_of(
                frame?
                    .fields
                    .into_iter()
                    .map(|(n, t)| match pairs.iter().find(|(a, _)| *a == n) {
                        Some((_, b)) => (b.clone(), t),
                        None => (n, t),
                    })
                    .collect(),
            )
        }
        "ods.group_by" => {
            let keys = match args.first()? {
                [
                    T {
                        tok: Tok::Str(k), ..
                    },
                ] => vec![k.clone()],
                k => strings(k)?,
            };
            let mut fields: Vec<Field> = keys
                .iter()
                .map(|k| (k.clone(), type_of(&frame, k)))
                .collect();
            for agg in args.get(1).and_then(|a| list_items(a)).unwrap_or_default() {
                let parts = list_items(agg).unwrap_or_default();
                let text = |j: usize| match parts.get(j).and_then(|p| p.first()).map(|t| &t.tok) {
                    Some(Tok::Str(s)) => Some(s.clone()),
                    _ => None,
                };
                let (Some(out), Some(op)) = (text(0), text(1)) else {
                    continue;
                };
                let ty = match op.as_str() {
                    "count" | "n_unique" => "Int".to_string(),
                    "mean" | "median" | "std" | "var" => "Float".to_string(),
                    _ => text(2).map(|c| type_of(&frame, &c)).unwrap_or_default(),
                };
                fields.push((out, ty));
            }
            frame_of(fields)
        }
        "ods.join" => {
            let left = frame?;
            let right = eval(args.first()?, resolve)?;
            let on = match args.get(1) {
                Some(
                    [
                        T {
                            tok: Tok::Str(k), ..
                        },
                    ],
                ) => vec![k.clone()],
                Some(k) => strings(k).unwrap_or_default(),
                None => Vec::new(),
            };
            let mut fields = left.fields;
            for (n, t) in right.fields {
                if on.contains(&n) {
                    continue;
                }
                let name = if fields.iter().any(|(m, _)| *m == n) {
                    format!("{n}_right")
                } else {
                    n
                };
                fields.push((name, t));
            }
            frame_of(fields)
        }
        _ => None,
    }
}

/// A list literal's items (`[a, b]`), each its tokens.
fn list_items(ts: &[T]) -> Option<Vec<&[T]>> {
    if ts.first()?.tok != Tok::Open('[')
        || ts.last()?.tok != Tok::Close(']')
        || matching_open(ts, ts.len() - 1)? != 0
    {
        return None;
    }
    let inner = &ts[1..ts.len() - 1];
    if inner.is_empty() {
        return Some(Vec::new());
    }
    Some(
        split_at(inner, |t| t.tok == Tok::Comma)
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect(),
    )
}

/// A list literal of strings.
fn strings(ts: &[T]) -> Option<Vec<String>> {
    list_items(ts)?
        .into_iter()
        .map(|p| match p {
            [
                T {
                    tok: Tok::Str(s), ..
                },
            ] => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// A map literal's (`#{ "k": v, … }`) string keys and pairs.
fn map_entries(ts: &[T]) -> Option<Vec<(String, &[T])>> {
    if ts.first()?.tok != Tok::Open('#')
        || ts.last()?.tok != Tok::Close('}')
        || matching_open(ts, ts.len() - 1)? != 0
    {
        return None;
    }
    let inner = &ts[1..ts.len() - 1];
    if inner.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for e in split_at(inner, |t| t.tok == Tok::Comma) {
        if let [
            T {
                tok: Tok::Str(k), ..
            },
            T {
                tok: Tok::Colon, ..
            },
            v @ ..,
        ] = e
        {
            out.push((k.clone(), v));
        }
    }
    Some(out)
}

fn map_literal(ts: &[T]) -> Option<Vec<Field>> {
    Some(
        map_entries(ts)?
            .into_iter()
            .map(|(k, v)| (k, value_type(v)))
            .collect(),
    )
}

fn map_pairs(ts: &[T]) -> Option<Vec<(String, String)>> {
    Some(
        map_entries(ts)?
            .into_iter()
            .filter_map(|(k, v)| match v {
                [
                    T {
                        tok: Tok::Str(s), ..
                    },
                ] => Some((k, s.clone())),
                _ => None,
            })
            .collect(),
    )
}

/// A literal value's type, as far as its tokens say.
fn value_type(v: &[T]) -> String {
    match v {
        [
            T {
                tok: Tok::Str(_), ..
            },
        ] => "String".into(),
        [
            T {
                tok: Tok::Num(n), ..
            },
        ]
        | [
            T {
                tok: Tok::Other, ..
            },
            T {
                tok: Tok::Num(n), ..
            },
        ] => {
            if n.contains('.') || n.contains('e') {
                "Float".into()
            } else {
                "Int".into()
            }
        }
        [
            T {
                tok: Tok::Ident(b), ..
            },
        ] if b == "true" || b == "false" => "Bool".into(),
        [
            T {
                tok: Tok::Open('['),
                ..
            },
            ..,
        ] => "List".into(),
        [
            T {
                tok: Tok::Open('#'),
                ..
            },
            ..,
        ] => "Map".into(),
        _ => String::new(),
    }
}

/// A column's type from its values' literal (`[1, 2]`, `ods.series([…])`).
fn column_type(v: &[T]) -> String {
    let list = match as_call(v) {
        Some((c, args)) if c == "ods.series" => args.first().copied(),
        _ => Some(v),
    };
    list.and_then(list_items)
        .and_then(|items| items.first().map(|x| value_type(x)))
        .unwrap_or_default()
}

/// A CSV cell's type as `ods.read_csv` infers it.
fn csv_type(c: &str) -> String {
    let c = c.trim();
    if c.parse::<i64>().is_ok() {
        "Int".into()
    } else if c.parse::<f64>().is_ok() {
        "Float".into()
    } else if c == "true" || c == "false" {
        "Bool".into()
    } else {
        "String".into()
    }
}

/// The text of the expression a `let` binds `name` to, the nearest
/// before the end of `text` (`let name = …` to the statement's end).
pub fn binding_text(text: &str, name: &str) -> Option<String> {
    let toks = tokenize(text);
    let mut at = None;
    for k in 0..toks.len() {
        if toks[k].tok == Tok::Ident("let".into()) {
            let mut j = k + 1;
            if toks
                .get(j)
                .is_some_and(|t| t.tok == Tok::Ident("mut".into()))
            {
                j += 1;
            }
            if toks
                .get(j)
                .is_some_and(|t| t.tok == Tok::Ident(name.into()))
                && toks.get(j + 1).is_some_and(|t| t.tok == Tok::Assign)
            {
                at = Some(j + 2);
            }
        }
    }
    let from = at?;
    let mut depth = 0i32;
    let mut end = None;
    for k in from..toks.len() {
        match toks[k].tok {
            Tok::Open(_) => depth += 1,
            Tok::Close(_) => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            Tok::Newline if depth == 0 => {
                let next = (k + 1..toks.len()).find(|&j| toks[j].tok != Tok::Newline);
                if !next.is_some_and(|j| matches!(toks[j].tok, Tok::Pipe | Tok::Dot)) {
                    break;
                }
                continue;
            }
            Tok::Unclosed(_) => return None,
            _ => {}
        }
        end = Some(k);
    }
    let end = end?;
    Some(text[toks[from].start..toks[end].end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<Fields> {
        None
    }

    fn sales(n: &str) -> Option<Fields> {
        (n == "sales").then(|| Fields {
            kind: "frame",
            fields: vec![
                ("region".into(), "String".into()),
                ("amount".into(), "Float".into()),
                ("qty".into(), "Int".into()),
            ],
        })
    }

    fn names(f: Option<Fields>) -> Vec<String> {
        f.map(|f| f.fields.into_iter().map(|x| x.0).collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_places_a_column_is_named() {
        let at = |p: &str| context(p).map(|c| (c.receiver, c.want, c.typed));
        assert_eq!(at("sales[\""), Some(("sales".into(), Want::Any, "".into())));
        assert_eq!(
            at("plot.bar(by_region[\"re"),
            Some(("by_region".into(), Want::Any, "re".into()))
        );
        assert_eq!(
            at("ods.select(sales, [\"a\", \""),
            Some(("sales".into(), Want::Columns, "".into()))
        );
        assert_eq!(
            at("ods.group_by(sales, \"r"),
            Some(("sales".into(), Want::Columns, "r".into()))
        );
        assert_eq!(
            at("let x = sales |> ods.sort_by(\""),
            Some(("sales".into(), Want::Columns, "".into()))
        );
        let chain = "let by = sales\n    |> ods.group_by(\"region\", [[\"revenue\", \"sum\", \"amount\"]])\n    |> ods.sort_by(\"";
        assert_eq!(
            at(chain).map(|c| c.0),
            Some(
                "sales\n    |> ods.group_by(\"region\", [[\"revenue\", \"sum\", \"amount\"]])"
                    .into()
            )
        );
        assert_eq!(
            at("sales |> ods.group_by(\"region\", [[\"t\", \"sum\", \"").map(|c| c.0),
            Some("sales".into())
        );
        assert_eq!(
            at("map_get(m, \"k").map(|c| (c.0, c.1)),
            Some(("m".into(), Want::Keys))
        );
        assert_eq!(at("cfg.na"), Some(("cfg".into(), Want::Keys, "na".into())));
        assert_eq!(
            at("ods.head(sales, 8)[\""),
            Some(("ods.head(sales, 8)".into(), Want::Any, "".into()))
        );
        // a chain's step on a line of its own: the receiver is the line above
        assert_eq!(
            at("let x = cfg\n    .na"),
            Some(("cfg".into(), Want::Keys, "na".into()))
        );
        assert_eq!(
            at("let x = f\n    .with(a)\n    .").map(|c| c.0),
            Some("f\n    .with(a)".into())
        );
        assert_eq!(at("let r = 0\n..").map(|c| c.0), None);
        // not a column's place
        assert_eq!(at("println(\"hello"), None);
        assert_eq!(at("ods.group_by(\""), None);
        assert_eq!(at("let s = \"a"), None);
        assert_eq!(at("x = sales[\"a\"] + \"b"), None);
    }

    #[test]
    fn an_expressions_columns_follow_its_verbs() {
        let f = |e: &str| names(fields_of(e, &mut sales));
        assert_eq!(f("sales"), ["region", "amount", "qty"]);
        assert_eq!(
            f(
                "sales |> ods.group_by(\"region\", [[\"revenue\", \"sum\", \"amount\"], [\"n\", \"count\"]]) |> ods.sort_by(\"revenue\", true)"
            ),
            ["region", "revenue", "n"]
        );
        assert_eq!(
            f("ods.filter(sales, sales[\"amount\"] > 2.0)"),
            ["region", "amount", "qty"]
        );
        assert_eq!(f("ods.select(sales, [\"qty\"])"), ["qty"]);
        assert_eq!(
            f("sales |> ods.with_column(\"month\", m)"),
            ["region", "amount", "qty", "month"]
        );
        assert_eq!(
            f("ods.rename(sales, #{ \"qty\": \"units\" })"),
            ["region", "amount", "units"]
        );
        assert_eq!(
            f("unwrap(ods.read_csv_file(\"x.csv\"))"),
            Vec::<String>::new()
        );
        let g = fields_of("sales |> ods.group_by(\"region\", [[\"total\", \"sum\", \"amount\"], [\"n\", \"count\"]])", &mut sales).unwrap();
        assert_eq!(g.fields[1], ("total".into(), "Float".into()));
        assert_eq!(g.fields[2], ("n".into(), "Int".into()));
    }

    #[test]
    fn literals_say_their_fields_and_types() {
        let lit = |e: &str| fields_of(e, &mut none).unwrap();
        let fr = lit("ods.frame([[\"arm\", ods.series([\"A\", \"B\"])], [\"mean\", [1.5, 2.0]]])");
        assert_eq!(fr.kind, "frame");
        assert_eq!(
            fr.fields,
            vec![
                ("arm".into(), "String".into()),
                ("mean".into(), "Float".into())
            ]
        );
        let csv = lit("ods.read_csv(\"region,amount,qty\\neast,25.5,10\\n\")");
        assert_eq!(
            csv.fields,
            vec![
                ("region".into(), "String".into()),
                ("amount".into(), "Float".into()),
                ("qty".into(), "Int".into())
            ]
        );
        let m = lit("#{ \"title\": \"x\", \"n\": 3, \"tags\": [1] }");
        assert_eq!(m.kind, "map");
        assert_eq!(
            m.fields,
            vec![
                ("title".into(), "String".into()),
                ("n".into(), "Int".into()),
                ("tags".into(), "List".into())
            ]
        );
        let recs = lit("ods.frame_from_records([#{ \"a\": 1, \"b\": true }])");
        assert_eq!(
            recs.fields,
            vec![("a".into(), "Int".into()), ("b".into(), "Bool".into())]
        );
    }

    #[test]
    fn a_bindings_text_runs_to_its_statements_end() {
        let t = "let a = 1\nlet by = sales\n    |> ods.group_by(\"r\", [])\nlet c = by\n";
        assert_eq!(
            binding_text(t, "by").as_deref(),
            Some("sales\n    |> ods.group_by(\"r\", [])")
        );
        assert_eq!(binding_text(t, "a").as_deref(), Some("1"));
        assert_eq!(binding_text(t, "zz"), None);
        let m = "let m = #{\n    \"a\": 1,\n    \"b\": 2,\n}\nm";
        assert_eq!(
            binding_text(m, "m").as_deref(),
            Some("#{\n    \"a\": 1,\n    \"b\": 2,\n}")
        );
    }
}
