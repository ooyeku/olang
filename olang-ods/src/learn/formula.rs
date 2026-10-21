//! Model formulas: `"price ~ sqft + C(hood) + C(kind):C(hour) + I(sqft^2)"`.
//!
//! The Wilkinson–Rogers notation R and patsy read, parsed natively:
//!
//! | Form | Meaning |
//! |---|---|
//! | `a + b` | both terms |
//! | `a:b` | their interaction (the product of their columns) |
//! | `a * b` | `a + b + a:b` |
//! | `a / b` | `a + a:b` (b nested in a); `(a + b) / c` is `a + b + a:b:c` |
//! | `(a + b + c)^2` | every interaction of up to two of them (`**` too) |
//! | `(a + b):c` | `a:c + b:c` (the operators distribute) |
//! | `- a` | leave a term out; `- 1` (or `+ 0`) drops the intercept |
//! | `C(x)` | `x` as categorical, whatever its type (an Int hour) |
//! | `I(expr)` | arithmetic (`+ - * / ^`, numbers, columns, functions) as one numeric column |
//! | `log(x)`, `sqrt(x)`, … | a math function of an expression, a numeric column |
//! | `Q("a name")` | a column whose name is not an identifier |
//!
//! A String, Category or Bool-free text column named bare is categorical
//! too (`hood` and `C(hood)` encode alike). The response may be a column
//! or an expression (`log(price) ~ …`).
//!
//! **Coding** follows patsy's rule, so a formula gives a full-rank design
//! with the columns patsy would span: each term adds only what the terms
//! before it (lower order first) do not already span. A categorical
//! factor is coded by treatment contrasts (its first level the baseline)
//! where its margin is already in the model, and in full where it is not:
//! `C(day):C(hour)` alone, with an intercept, gives `days × hours − 1`
//! columns, and `C(day) * C(hour)` gives the main effects and
//! `(days − 1)(hours − 1)` interaction columns.
//!
//! Arithmetic compiles to the expression engine (`fuse::Expr`), so
//! `I(sqft^2)` is one fused pass, and a prediction recomputes it on the
//! new rows. Errors point into the formula.

use super::data::{Result, bad, is_valid, numeric};
use crate::fuse::{self, BinOp, Math, Op};
use crate::{Frame, Series};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------
// Errors that point into the formula
// ---------------------------------------------------------------------

fn at(src: &str, (start, end): (usize, usize), msg: impl Into<String>) -> crate::OdsError {
    let width = src[start.min(src.len())..end.min(src.len())].chars().count().max(1);
    let pad = src[..start.min(src.len())].chars().count();
    bad(format!(
        "learn: in the formula, {}:\n    {}\n    {}{}",
        msg.into(),
        src,
        " ".repeat(pad),
        "^".repeat(width)
    ))
}

// ---------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    Sym(&'static str),
    End,
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    span: (usize, usize),
}

fn lex(src: &str) -> Result<Vec<Token>> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Ident(src[start..i].to_string()),
                span: (start, i),
            });
        } else if c.is_ascii_digit() || (c == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit()) {
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                i += 1;
                if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                    i += 1;
                }
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let v: f64 = src[start..i]
                .parse()
                .map_err(|_| at(src, (start, i), "this is not a number"))?;
            out.push(Token {
                tok: Tok::Num(v),
                span: (start, i),
            });
        } else if c == b'"' || c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != c {
                i += 1;
            }
            if i >= b.len() {
                return Err(at(src, (start, b.len()), "this quote is not closed"));
            }
            out.push(Token {
                tok: Tok::Str(src[start + 1..i].to_string()),
                span: (start, i + 1),
            });
            i += 1;
        } else {
            let two = if i + 1 < b.len() { &src[i..i + 2] } else { "" };
            let sym: &'static str = match (c, two) {
                (_, "**") => "**",
                (b'~', _) => "~",
                (b'+', _) => "+",
                (b'-', _) => "-",
                (b'*', _) => "*",
                (b'/', _) => "/",
                (b':', _) => ":",
                (b'^', _) => "^",
                (b'(', _) => "(",
                (b')', _) => ")",
                (b',', _) => ",",
                _ => {
                    let ch = src[i..].chars().next().unwrap_or(' ');
                    return Err(at(
                        src,
                        (i, i + ch.len_utf8()),
                        format!("'{}' is not part of a formula", ch),
                    ));
                }
            };
            i += sym.len();
            out.push(Token {
                tok: Tok::Sym(sym),
                span: (start, i),
            });
        }
    }
    out.push(Token {
        tok: Tok::End,
        span: (b.len(), b.len()),
    });
    Ok(out)
}

// ---------------------------------------------------------------------
// Arithmetic (inside I() and function calls)
// ---------------------------------------------------------------------

/// Numeric arithmetic over columns: what `I(…)` and `log(…)` hold.
#[derive(Clone, Debug)]
pub enum Arith {
    Col(String, (usize, usize)),
    Num(f64),
    Neg(Box<Arith>),
    Bin(BinOp, Box<Arith>, Box<Arith>),
    Call(Math, Box<Arith>),
}

impl Arith {
    fn columns(&self, out: &mut Vec<String>) {
        match self {
            Arith::Col(c, _) => {
                if !out.contains(c) {
                    out.push(c.clone())
                }
            }
            Arith::Num(_) => {}
            Arith::Neg(a) | Arith::Call(_, a) => a.columns(out),
            Arith::Bin(_, a, b) => {
                a.columns(out);
                b.columns(out);
            }
        }
    }

    fn compile(&self, frame: &Frame, src: &str) -> Result<Arc<fuse::Expr>> {
        Ok(match self {
            Arith::Col(name, span) => {
                let s = frame.column(name).map_err(|_| {
                    at(
                        src,
                        *span,
                        format!("no column '{}' (the Frame has {})", name, frame.names().join(", ")),
                    )
                })?;
                if !matches!(s, Series::F64 { .. } | Series::I64 { .. } | Series::Bool { .. }) {
                    return Err(at(
                        src,
                        *span,
                        format!("'{}' is {}, and arithmetic needs numbers", name, s.dtype()),
                    ));
                }
                let s = match s {
                    Series::Bool { .. } => {
                        let (v, ok) = numeric(s, name)?;
                        Series::from_f64_options(v.into_iter().zip(ok).map(|(x, o)| o.then_some(x)).collect())
                    }
                    _ => s.clone(),
                };
                fuse::Expr::col(s)
            }
            Arith::Num(v) => fuse::Expr::lit(crate::Scalar::F64(*v)),
            Arith::Neg(a) => fuse::Expr::op(Op::Neg, vec![a.compile(frame, src)?]),
            Arith::Bin(op, a, b) => fuse::Expr::bin(*op, a.compile(frame, src)?, b.compile(frame, src)?),
            Arith::Call(m, a) => fuse::Expr::op(Op::Math(*m), vec![a.compile(frame, src)?]),
        })
    }

    /// The values over `frame`'s rows (one fused pass), as Floats with
    /// their validity.
    fn eval(&self, frame: &Frame, src: &str, par: bool) -> Result<(Vec<f64>, Vec<bool>)> {
        let e = self.compile(frame, src)?;
        let s = e.eval(frame.n_rows(), par)?;
        numeric(&s, "formula")
    }
}

/// `log` is the natural logarithm (NumPy's and R's), beside the engine's
/// own names.
fn math_named(name: &str) -> Option<Math> {
    match name {
        "log" | "np.log" => Some(Math::Ln),
        "np.exp" => Some(Math::Exp),
        "np.sqrt" => Some(Math::Sqrt),
        "np.log2" => Some(Math::Log2),
        "np.log10" => Some(Math::Log10),
        "np.log1p" | "log1p" => Some(Math::Ln1p),
        "np.abs" => Some(Math::Abs),
        _ => Math::parse(name),
    }
}

// ---------------------------------------------------------------------
// Factors and terms
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum FactorKind {
    /// A bare column: numeric, or categorical when it holds text.
    Column(String),
    /// `C(col)`: categorical whatever its type.
    Categorical(String),
    /// `I(…)` or a function: a computed numeric column.
    Arith(Arith),
}

#[derive(Clone, Debug)]
pub struct FactorSpec {
    /// As written, without spaces: `C(hood)`, `I(sqft^2)`, `sqft`.
    pub label: String,
    pub kind: FactorKind,
    span: (usize, usize),
}

/// A parsed formula, not yet tied to a Frame.
#[derive(Clone, Debug)]
pub struct Formula {
    pub text: String,
    pub response: FactorSpec,
    pub factors: Vec<FactorSpec>,
    /// Each a set of factor indices, in the order written.
    pub terms: Vec<Vec<usize>>,
    pub intercept: bool,
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    factors: Vec<FactorSpec>,
}

/// A right-hand side under construction: terms, and what it says about
/// the intercept (`Some(true)` for `1`, `Some(false)` for `0`).
#[derive(Clone, Debug, Default)]
struct Terms {
    terms: Vec<Vec<usize>>,
    intercept: Option<bool>,
}

fn same_term(a: &[usize], b: &[usize]) -> bool {
    a.len() == b.len() && a.iter().all(|x| b.contains(x))
}

fn push_term(list: &mut Vec<Vec<usize>>, t: Vec<usize>) {
    if !list.iter().any(|u| same_term(u, &t)) {
        list.push(t);
    }
}

impl Terms {
    fn plus(mut self, other: Terms) -> Terms {
        for t in other.terms {
            push_term(&mut self.terms, t);
        }
        if other.intercept.is_some() {
            self.intercept = other.intercept;
        }
        self
    }

    fn minus(mut self, other: Terms) -> Terms {
        self.terms.retain(|t| !other.terms.iter().any(|u| same_term(t, u)));
        match other.intercept {
            Some(true) => self.intercept = Some(false),
            Some(false) => self.intercept = Some(true),
            None => {}
        }
        self
    }

    /// `self / other`: self's terms, then the interaction of all of
    /// self's factors with each of other's terms (patsy's nesting).
    fn nest(self, other: &Terms) -> Terms {
        let mut all: Vec<usize> = Vec::new();
        for t in &self.terms {
            for f in t {
                if !all.contains(f) {
                    all.push(*f);
                }
            }
        }
        let whole = Terms {
            terms: if all.is_empty() { Vec::new() } else { vec![all] },
            intercept: None,
        };
        let inner = whole.colon(other);
        self.plus(inner)
    }

    /// `self ^ n`: every interaction of up to n of self's terms.
    fn power(&self, n: usize) -> Terms {
        let mut acc = Terms {
            terms: self.terms.clone(),
            intercept: None,
        };
        let mut cur = Terms {
            terms: self.terms.clone(),
            intercept: None,
        };
        for _ in 1..n {
            cur = cur.colon(self);
            acc = acc.plus(Terms {
                terms: cur.terms.clone(),
                intercept: None,
            });
        }
        acc
    }

    fn colon(&self, other: &Terms) -> Terms {
        let mut out = Vec::new();
        for a in &self.terms {
            for b in &other.terms {
                let mut t = a.clone();
                for f in b {
                    if !t.contains(f) {
                        t.push(*f);
                    }
                }
                push_term(&mut out, t);
            }
        }
        Terms {
            terms: out,
            intercept: None,
        }
    }
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }

    fn next(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn is(&self, sym: &str) -> bool {
        matches!(&self.peek().tok, Tok::Sym(s) if *s == sym)
    }

    fn expect(&mut self, sym: &'static str, what: &str) -> Result<Token> {
        if self.is(sym) {
            return Ok(self.next());
        }
        let t = self.peek().clone();
        Err(at(self.src, t.span, format!("expected {} here", what)))
    }

    fn text(&self, span: (usize, usize)) -> String {
        self.src[span.0..span.1]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    fn add_factor(&mut self, f: FactorSpec) -> usize {
        if let Some(i) = self.factors.iter().position(|g| g.label == f.label) {
            return i;
        }
        self.factors.push(f);
        self.factors.len() - 1
    }

    // sum := ['-'] product (('+' | '-') product)*
    fn sum(&mut self) -> Result<Terms> {
        let mut acc = if self.is("-") {
            self.next();
            Terms {
                intercept: None,
                terms: Vec::new(),
            }
            .minus(self.product()?)
        } else {
            self.product()?
        };
        loop {
            if self.is("+") {
                self.next();
                acc = acc.plus(self.product()?);
            } else if self.is("-") {
                self.next();
                acc = acc.minus(self.product()?);
            } else {
                return Ok(acc);
            }
        }
    }

    // product := inter (('*' | '/') inter)*
    fn product(&mut self) -> Result<Terms> {
        let mut acc = self.inter()?;
        while self.is("*") || self.is("/") {
            let op = self.next();
            let rhs = self.inter()?;
            let star = op.tok == Tok::Sym("*");
            if acc.intercept.is_some() || rhs.intercept.is_some() {
                return Err(at(
                    self.src,
                    op.span,
                    if star { "`*` joins terms, not 0 or 1" } else { "`/` nests terms, not 0 or 1" },
                ));
            }
            if star {
                let both = acc.colon(&rhs);
                acc = acc.plus(rhs).plus(both);
            } else {
                acc = acc.nest(&rhs);
            }
        }
        Ok(acc)
    }

    // inter := power (':' power)*
    fn inter(&mut self) -> Result<Terms> {
        let mut acc = self.power()?;
        while self.is(":") {
            let colon = self.next();
            let rhs = self.power()?;
            if acc.intercept.is_some() || rhs.intercept.is_some() {
                return Err(at(self.src, colon.span, "`:` joins terms, not 0 or 1"));
            }
            acc = acc.colon(&rhs);
        }
        Ok(acc)
    }

    // power := atom (('^' | '**') Int)*
    fn power(&mut self) -> Result<Terms> {
        let mut acc = self.atom()?;
        while self.is("^") || self.is("**") {
            let op = self.next();
            if acc.intercept.is_some() {
                return Err(at(self.src, op.span, "a power raises terms, not 0 or 1"));
            }
            let n = self.next();
            match n.tok {
                Tok::Num(v) if v >= 1.0 && v.fract() == 0.0 => acc = acc.power(v as usize),
                _ => {
                    return Err(at(
                        self.src,
                        n.span,
                        "a power of terms is a whole number of at least 1 (`(a + b)^2`); arithmetic goes inside I(…)",
                    ));
                }
            }
        }
        Ok(acc)
    }

    fn atom(&mut self) -> Result<Terms> {
        let t = self.peek().clone();
        match &t.tok {
            Tok::Sym("(") => {
                self.next();
                let inner = self.sum()?;
                self.expect(")", "`)`")?;
                Ok(inner)
            }
            Tok::Num(v) if *v == 0.0 || *v == 1.0 => {
                self.next();
                Ok(Terms {
                    terms: Vec::new(),
                    intercept: Some(*v == 1.0),
                })
            }
            Tok::Num(_) => Err(at(
                self.src,
                t.span,
                "a number is a term only as 1 (the intercept) or 0 (none); arithmetic goes inside I(…)",
            )),
            _ => {
                let f = self.factor()?;
                let i = self.add_factor(f);
                Ok(Terms {
                    terms: vec![vec![i]],
                    intercept: None,
                })
            }
        }
    }

    /// A column, `C(col)`, `I(arith)`, `f(arith)` or `Q("name")`.
    fn factor(&mut self) -> Result<FactorSpec> {
        let t = self.next();
        let name = match &t.tok {
            Tok::Ident(n) => n.clone(),
            Tok::End => return Err(at(self.src, t.span, "a term is missing here")),
            _ => return Err(at(self.src, t.span, "expected a column or a term here")),
        };
        if !self.is("(") {
            return Ok(FactorSpec {
                label: name.clone(),
                kind: FactorKind::Column(name),
                span: t.span,
            });
        }
        self.next();
        let kind = match name.as_str() {
            "C" => {
                let col = self.column_name()?;
                if self.is(",") {
                    let c = self.peek().clone();
                    return Err(at(
                        self.src,
                        c.span,
                        "C() takes one column; its levels come from the data (or the Category's own order)",
                    ));
                }
                FactorKind::Categorical(col)
            }
            "Q" => {
                let col = self.column_name()?;
                self.expect(")", "`)`")?;
                let end = self.toks[self.pos - 1].span.1;
                return Ok(FactorSpec {
                    label: col.clone(),
                    kind: FactorKind::Column(col),
                    span: (t.span.0, end),
                });
            }
            "I" => FactorKind::Arith(self.arith()?),
            other => match math_named(other) {
                Some(m) => FactorKind::Arith(Arith::Call(m, Box::new(self.arith()?))),
                None => {
                    return Err(at(
                        self.src,
                        t.span,
                        format!(
                            "'{}' is not a formula function (C, I, Q, or a math function: log, exp, sqrt, abs, …)",
                            other
                        ),
                    ));
                }
            },
        };
        let close = self.expect(")", "`)`")?;
        let span = (t.span.0, close.span.1);
        Ok(FactorSpec {
            label: self.text(span),
            kind,
            span,
        })
    }

    fn column_name(&mut self) -> Result<String> {
        let t = self.next();
        match &t.tok {
            Tok::Ident(n) => Ok(n.clone()),
            Tok::Str(s) => Ok(s.clone()),
            Tok::Sym("(") if false => unreachable!(),
            _ => Err(at(self.src, t.span, "expected a column name here")),
        }
    }

    // arith := term (('+' | '-') term)*
    fn arith(&mut self) -> Result<Arith> {
        let mut acc = self.arith_term()?;
        loop {
            let op = if self.is("+") {
                BinOp::Add
            } else if self.is("-") {
                BinOp::Sub
            } else {
                return Ok(acc);
            };
            self.next();
            acc = Arith::Bin(op, Box::new(acc), Box::new(self.arith_term()?));
        }
    }

    fn arith_term(&mut self) -> Result<Arith> {
        let mut acc = self.arith_unary()?;
        loop {
            let op = if self.is("*") {
                BinOp::Mul
            } else if self.is("/") {
                BinOp::Div
            } else {
                return Ok(acc);
            };
            self.next();
            acc = Arith::Bin(op, Box::new(acc), Box::new(self.arith_unary()?));
        }
    }

    fn arith_unary(&mut self) -> Result<Arith> {
        if self.is("-") {
            self.next();
            return Ok(Arith::Neg(Box::new(self.arith_unary()?)));
        }
        if self.is("+") {
            self.next();
            return self.arith_unary();
        }
        self.arith_power()
    }

    // power binds tighter than unary minus on its right: -x^2 = -(x^2)
    fn arith_power(&mut self) -> Result<Arith> {
        let base = self.arith_atom()?;
        if self.is("^") || self.is("**") {
            self.next();
            let exp = self.arith_unary()?;
            return Ok(Arith::Bin(BinOp::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn arith_atom(&mut self) -> Result<Arith> {
        let t = self.next();
        match &t.tok {
            Tok::Num(v) => Ok(Arith::Num(*v)),
            Tok::Sym("(") => {
                let inner = self.arith()?;
                self.expect(")", "`)`")?;
                Ok(inner)
            }
            Tok::Ident(n) if self.is("(") => {
                if n == "Q" {
                    self.next();
                    let span = self.peek().span;
                    let col = self.column_name()?;
                    self.expect(")", "`)`")?;
                    return Ok(Arith::Col(col, span));
                }
                let Some(m) = math_named(n) else {
                    return Err(at(
                        self.src,
                        t.span,
                        format!("'{}' is not a math function (log, exp, sqrt, abs, sin, …)", n),
                    ));
                };
                self.next();
                let inner = self.arith()?;
                self.expect(")", "`)`")?;
                Ok(Arith::Call(m, Box::new(inner)))
            }
            Tok::Ident(n) => Ok(Arith::Col(n.clone(), t.span)),
            Tok::End => Err(at(self.src, t.span, "the expression stops short here")),
            _ => Err(at(self.src, t.span, "expected a number, a column or `(` here")),
        }
    }
}

/// Whether a `y` argument is a formula rather than a column name.
pub fn is_formula(y: &str) -> bool {
    y.contains('~')
}

impl Formula {
    pub fn parse(src: &str) -> Result<Formula> {
        let toks = lex(src)?;
        let mut p = Parser {
            src,
            toks,
            pos: 0,
            factors: Vec::new(),
        };
        if p.is("~") {
            let t = p.peek().clone();
            return Err(at(src, t.span, "the response (the column to predict) goes before `~`"));
        }
        let response = p.factor()?;
        if let FactorKind::Categorical(_) = response.kind {
            return Err(at(src, response.span, "the response is a column or an expression, not C(…)"));
        }
        p.expect("~", "`~` after the response")?;
        if matches!(p.peek().tok, Tok::End) {
            let t = p.peek().clone();
            return Err(at(src, t.span, "the right side of `~` is empty"));
        }
        let rhs = p.sum()?;
        if !matches!(p.peek().tok, Tok::End) {
            let t = p.peek().clone();
            return Err(at(src, t.span, "expected `+`, `-`, `*` or `:` here"));
        }
        let mut terms = rhs.terms;
        // Lower order first (stable): what patsy codes against.
        terms.sort_by_key(|t| t.len());
        Ok(Formula {
            text: src.to_string(),
            response,
            factors: p.factors,
            terms,
            intercept: rhs.intercept.unwrap_or(true),
        })
    }

    /// Every column the formula reads, the response first.
    pub fn columns(&self) -> Vec<String> {
        let mut out = Vec::new();
        for f in std::iter::once(&self.response).chain(&self.factors) {
            match &f.kind {
                FactorKind::Column(c) | FactorKind::Categorical(c) => {
                    if !out.contains(c) {
                        out.push(c.clone())
                    }
                }
                FactorKind::Arith(a) => a.columns(&mut out),
            }
        }
        out
    }

    /// The response's name: the column, or the expression as written.
    pub fn response_name(&self) -> String {
        self.response.label.clone()
    }

    /// The response's values over `frame`: a column as it is, an
    /// expression computed.
    pub fn response_series(&self, frame: &Frame, par: bool) -> Result<Series> {
        response_series(&self.response, &self.text, frame, par)
    }
}

fn response_series(r: &FactorSpec, src: &str, frame: &Frame, par: bool) -> Result<Series> {
    match &r.kind {
        FactorKind::Column(c) => frame.column(c).cloned().map_err(|_| {
            at(
                src,
                r.span,
                format!("no column '{}' (the Frame has {})", c, frame.names().join(", ")),
            )
        }),
        FactorKind::Arith(a) => {
            let e = a.compile(frame, src)?;
            Ok(e.eval(frame.n_rows(), par)?)
        }
        FactorKind::Categorical(_) => Err(at(src, r.span, "the response cannot be C(…)")),
    }
}

/// The response of a fitted formula, kept with the model so scoring a
/// new Frame computes it the same way.
#[derive(Clone, Debug)]
pub struct Response {
    spec: FactorSpec,
    text: Arc<str>,
}

impl Response {
    pub fn series(&self, frame: &Frame, par: bool) -> Result<Series> {
        response_series(&self.spec, &self.text, frame, par)
    }

    /// The column it is, when it is one.
    pub fn column(&self) -> Option<&str> {
        match &self.spec.kind {
            FactorKind::Column(c) => Some(c),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------
// Coding against a Frame
// ---------------------------------------------------------------------

/// A categorical factor's levels, as learned from the training rows.
#[derive(Clone, Debug)]
pub enum Levels {
    Text(Vec<String>),
    Int(Vec<i64>),
    Bool(Vec<bool>),
}

impl Levels {
    fn len(&self) -> usize {
        match self {
            Levels::Text(v) => v.len(),
            Levels::Int(v) => v.len(),
            Levels::Bool(v) => v.len(),
        }
    }

    fn name(&self, i: usize) -> String {
        match self {
            Levels::Text(v) => v[i].clone(),
            Levels::Int(v) => v[i].to_string(),
            Levels::Bool(v) => v[i].to_string(),
        }
    }

    fn learn(s: &Series, col: &str, src: &str, span: (usize, usize)) -> Result<Levels> {
        Ok(match s {
            Series::Cat { levels, .. } => Levels::Text(levels.iter().map(str::to_string).collect()),
            Series::Str { .. } => Levels::Text(super::data::levels_of(s)),
            Series::I64 { values, .. } => {
                let set: std::collections::BTreeSet<i64> =
                    (0..s.len()).filter(|&i| is_valid(s, i)).map(|i| values[i]).collect();
                Levels::Int(set.into_iter().collect())
            }
            Series::Bool { values, .. } => {
                let mut seen = [false; 2];
                for i in 0..s.len() {
                    if is_valid(s, i) {
                        seen[values[i] as usize] = true;
                    }
                }
                Levels::Bool([false, true].into_iter().filter(|b| seen[*b as usize]).collect())
            }
            other => {
                return Err(at(
                    src,
                    span,
                    format!(
                        "C({}) needs text, a Category, Int or Bool; '{}' is {}",
                        col,
                        col,
                        other.dtype()
                    ),
                ));
            }
        })
    }

    /// Each row's level (`usize::MAX`: a level training never saw, which
    /// codes as the baseline) and validity.
    fn codes(&self, s: &Series, col: &str) -> Result<(Vec<usize>, Vec<bool>)> {
        let n = s.len();
        let valid: Vec<bool> = (0..n).map(|i| is_valid(s, i)).collect();
        let mut out = vec![usize::MAX; n];
        match self {
            Levels::Text(levels) => {
                let text = s.key_view();
                let Series::Str { col: sc, .. } = text.as_ref() else {
                    return Err(bad(format!(
                        "learn: column '{}' was text when the model was fit, now {}",
                        col,
                        s.dtype()
                    )));
                };
                let index: HashMap<&str, usize> =
                    levels.iter().enumerate().map(|(i, l)| (l.as_str(), i)).collect();
                for i in 0..n {
                    if valid[i]
                        && let Some(&p) = index.get(sc.get(i))
                    {
                        out[i] = p;
                    }
                }
            }
            Levels::Int(levels) => {
                let Series::I64 { values, .. } = s else {
                    return Err(bad(format!(
                        "learn: column '{}' was Int when the model was fit, now {}",
                        col,
                        s.dtype()
                    )));
                };
                for i in 0..n {
                    if valid[i]
                        && let Ok(p) = levels.binary_search(&values[i])
                    {
                        out[i] = p;
                    }
                }
            }
            Levels::Bool(levels) => {
                let Series::Bool { values, .. } = s else {
                    return Err(bad(format!(
                        "learn: column '{}' was Bool when the model was fit, now {}",
                        col,
                        s.dtype()
                    )));
                };
                for i in 0..n {
                    if valid[i]
                        && let Some(p) = levels.iter().position(|b| *b == values[i])
                    {
                        out[i] = p;
                    }
                }
            }
        }
        Ok((out, valid))
    }
}

/// A numeric factor: a column, or arithmetic over columns.
#[derive(Clone, Debug)]
pub enum NumSource {
    Column(String),
    Arith(Arith),
}

#[derive(Clone, Debug)]
pub struct NumFactor {
    pub label: String,
    pub source: NumSource,
}

#[derive(Clone, Debug)]
pub struct CatFactor {
    /// The column.
    pub column: String,
    pub levels: Levels,
}

/// One block of a term's columns: a categorical subset, each factor
/// coded by contrasts (its first level dropped) or in full.
#[derive(Clone, Debug)]
pub struct Block {
    /// (index into the term's categorical factors, coded in full)
    pub factors: Vec<(usize, bool)>,
}

/// A term of a fitted formula and how it is coded.
#[derive(Clone, Debug)]
pub struct TermCode {
    /// As the formula writes it: `C(kind):C(hour)`.
    pub label: String,
    pub nums: Vec<NumFactor>,
    pub cats: Vec<CatFactor>,
    pub blocks: Vec<Block>,
    /// The formula's text, for errors.
    pub text: Arc<str>,
}

impl TermCode {
    /// The feature names, block by block.
    pub fn features(&self) -> Vec<String> {
        let num: Vec<&str> = self.nums.iter().map(|n| n.label.as_str()).collect();
        let mut out = Vec::new();
        for b in &self.blocks {
            let mut names: Vec<String> = vec![num.join(":")];
            for &(ci, full) in &b.factors {
                let c = &self.cats[ci];
                let from = usize::from(!full);
                let mut next = Vec::new();
                for prefix in &names {
                    for l in from..c.levels.len() {
                        let piece = format!("{}_{}", c.column, c.levels.name(l));
                        next.push(if prefix.is_empty() {
                            piece
                        } else {
                            format!("{}:{}", prefix, piece)
                        });
                    }
                }
                names = next;
            }
            out.extend(names);
        }
        out
    }

    /// The columns the term reads.
    pub fn sources(&self) -> Vec<String> {
        let mut out = Vec::new();
        for n in &self.nums {
            match &n.source {
                NumSource::Column(c) => {
                    if !out.contains(c) {
                        out.push(c.clone())
                    }
                }
                NumSource::Arith(a) => a.columns(&mut out),
            }
        }
        for c in &self.cats {
            if !out.contains(&c.column) {
                out.push(c.column.clone());
            }
        }
        out
    }

    /// The term's columns over `frame`, appended to `cols`; rows a value
    /// is missing from are marked invalid.
    pub fn encode(&self, frame: &Frame, cols: &mut Vec<Vec<f64>>, valid: &mut [bool], par: bool) -> Result<()> {
        let n = frame.n_rows();
        // The numeric part: the product of the numeric factors.
        let mut prod: Option<Vec<f64>> = None;
        for f in &self.nums {
            let (vals, ok) = match &f.source {
                NumSource::Column(c) => {
                    let s = frame.column(c)?;
                    if matches!(s, Series::Str { .. } | Series::Cat { .. }) {
                        return Err(bad(format!(
                            "learn: column '{}' was numeric when the model was fit, now {}",
                            c,
                            s.dtype()
                        )));
                    }
                    numeric(s, c)?
                }
                NumSource::Arith(a) => a.eval(frame, &self.text, par)?,
            };
            for (v, o) in valid.iter_mut().zip(&ok) {
                *v &= *o;
            }
            prod = Some(match prod {
                None => vals,
                Some(mut p) => {
                    for (x, v) in p.iter_mut().zip(&vals) {
                        *x *= v;
                    }
                    p
                }
            });
        }
        let codes: Vec<Vec<usize>> = self
            .cats
            .iter()
            .map(|c| {
                let s = frame.column(&c.column)?;
                let (codes, ok) = c.levels.codes(s, &c.column)?;
                for (v, o) in valid.iter_mut().zip(&ok) {
                    *v &= *o;
                }
                Ok(codes)
            })
            .collect::<Result<_>>()?;
        for b in &self.blocks {
            // The block's columns in mixed radix, the first factor most
            // significant (the order `features` names them).
            let widths: Vec<usize> = b
                .factors
                .iter()
                .map(|&(ci, full)| self.cats[ci].levels.len() - usize::from(!full))
                .collect();
            let total: usize = widths.iter().product();
            let base = cols.len();
            cols.extend((0..total).map(|_| vec![0.0f64; n]));
            for i in 0..n {
                let mut idx = 0usize;
                let mut hit = true;
                for (k, &(ci, full)) in b.factors.iter().enumerate() {
                    let code = codes[ci][i];
                    let pos = if code == usize::MAX {
                        None
                    } else if full {
                        Some(code)
                    } else if code == 0 {
                        None
                    } else {
                        Some(code - 1)
                    };
                    match pos {
                        Some(p) => idx = idx * widths[k] + p,
                        None => {
                            hit = false;
                            break;
                        }
                    }
                }
                if hit {
                    cols[base + idx][i] = prod.as_ref().map(|p| p[i]).unwrap_or(1.0);
                }
            }
        }
        Ok(())
    }
}

/// Tie a formula to a training Frame: resolve each factor's kind, learn
/// the categorical levels, and code each term against the ones before it.
pub fn code(f: &Formula, frame: &Frame) -> Result<(Vec<TermCode>, Response)> {
    let text: Arc<str> = Arc::from(f.text.as_str());
    // A factor's kind, from its spelling and the column's type.
    enum Resolved {
        Num(NumFactor),
        Cat(CatFactor),
    }
    let mut resolved = Vec::with_capacity(f.factors.len());
    for spec in &f.factors {
        let col_of = |c: &str| {
            frame.column(c).map_err(|_| {
                at(
                    &f.text,
                    spec.span,
                    format!("no column '{}' (the Frame has {})", c, frame.names().join(", ")),
                )
            })
        };
        resolved.push(match &spec.kind {
            FactorKind::Column(c) => {
                let s = col_of(c)?;
                match s {
                    Series::Str { .. } | Series::Cat { .. } => Resolved::Cat(CatFactor {
                        column: c.clone(),
                        levels: Levels::learn(s, c, &f.text, spec.span)?,
                    }),
                    Series::Date { .. } | Series::Datetime { .. } => {
                        return Err(at(
                            &f.text,
                            spec.span,
                            format!(
                                "'{}' is a {}; give the model its parts (a column of year(), month(), …) or C() of one",
                                c,
                                s.dtype()
                            ),
                        ));
                    }
                    _ => Resolved::Num(NumFactor {
                        label: c.clone(),
                        source: NumSource::Column(c.clone()),
                    }),
                }
            }
            FactorKind::Categorical(c) => {
                let s = col_of(c)?;
                let levels = Levels::learn(s, c, &f.text, spec.span)?;
                if levels.len() == 0 {
                    return Err(at(&f.text, spec.span, format!("'{}' has no values", c)));
                }
                Resolved::Cat(CatFactor {
                    column: c.clone(),
                    levels,
                })
            }
            FactorKind::Arith(a) => {
                // Checked now: a misspelt column fails at the fit.
                a.compile(frame, &f.text)?;
                Resolved::Num(NumFactor {
                    label: spec.label.clone(),
                    source: NumSource::Arith(a.clone()),
                })
            }
        });
    }
    let mut inputs = Vec::new();
    for g in &f.factors {
        match &g.kind {
            FactorKind::Column(c) | FactorKind::Categorical(c) => inputs.push(c.clone()),
            FactorKind::Arith(a) => a.columns(&mut inputs),
        }
    }
    if inputs.contains(&f.response.label) || f.factors.iter().any(|g| g.label == f.response.label) {
        return Err(at(
            &f.text,
            f.response.span,
            format!("the response '{}' is also on the right of `~`", f.response.label),
        ));
    }
    // What the model spans so far: (numeric factors, categorical factors)
    // of each term coded, the intercept first.
    let mut covered: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
    if f.intercept {
        covered.push((Vec::new(), Vec::new()));
    }
    let mut out = Vec::with_capacity(f.terms.len());
    for term in &f.terms {
        let mut nums_ix: Vec<usize> = term
            .iter()
            .copied()
            .filter(|&i| matches!(resolved[i], Resolved::Num(_)))
            .collect();
        let cats_ix: Vec<usize> = term
            .iter()
            .copied()
            .filter(|&i| matches!(resolved[i], Resolved::Cat(_)))
            .collect();
        nums_ix.sort_unstable();
        let k = cats_ix.len();
        // Every subset of the categorical factors (as a bit mask over
        // `cats_ix`), smallest first: which does the model not span yet?
        let mut masks: Vec<u32> = (0..(1u32 << k)).collect();
        masks.sort_by_key(|m| (m.count_ones(), *m));
        let spanned = |mask: u32| {
            covered.iter().any(|(n, c)| {
                *n == nums_ix
                    && (0..k).all(|b| mask & (1 << b) == 0 || c.contains(&cats_ix[b]))
            })
        };
        let needed: Vec<u32> = masks.iter().copied().filter(|&m| !spanned(m)).collect();
        // Larger subsets first: a factor coded in full absorbs the subset
        // without it.
        let mut absorbed: Vec<u32> = Vec::new();
        let mut blocks: Vec<(u32, Vec<(usize, bool)>)> = Vec::new();
        for &m in needed.iter().rev() {
            if absorbed.contains(&m) {
                continue;
            }
            let mut full = None;
            for b in 0..k {
                if m & (1 << b) != 0 {
                    let without = m & !(1 << b);
                    if needed.contains(&without) && !absorbed.contains(&without) {
                        full = Some(b);
                        absorbed.push(without);
                        break;
                    }
                }
            }
            let factors = (0..k)
                .filter(|b| m & (1 << b) != 0)
                .map(|b| (b, full == Some(b)))
                .collect();
            blocks.push((m, factors));
        }
        // Columns in the order patsy lists them: smaller subsets first.
        blocks.sort_by_key(|(m, _)| (m.count_ones(), *m));
        let nums = nums_ix
            .iter()
            .map(|&i| match &resolved[i] {
                Resolved::Num(n) => n.clone(),
                _ => unreachable!(),
            })
            .collect();
        let cats = cats_ix
            .iter()
            .map(|&i| match &resolved[i] {
                Resolved::Cat(c) => c.clone(),
                _ => unreachable!(),
            })
            .collect();
        let label = term
            .iter()
            .map(|&i| f.factors[i].label.as_str())
            .collect::<Vec<_>>()
            .join(":");
        let code = TermCode {
            label,
            nums,
            cats,
            blocks: blocks
                .into_iter()
                .filter(|(_, fs)| !fs.is_empty() || !nums_ix.is_empty())
                .map(|(_, factors)| Block { factors })
                .collect(),
            text: text.clone(),
        };
        if code.blocks.is_empty() {
            return Err(at(
                &f.text,
                f.factors[term[0]].span,
                format!("the term {} adds nothing the terms before it do not", code.label),
            ));
        }
        covered.push((nums_ix, cats_ix));
        out.push(code);
    }
    if out.is_empty() && !f.intercept {
        return Err(bad("learn: the formula has no terms"));
    }
    let response = Response {
        spec: f.response.clone(),
        text,
    };
    Ok((out, response))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame::new(vec![
            ("y".into(), Series::from_f64(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])),
            ("x".into(), Series::from_f64(vec![1.0, 2.0, 3.0, 4.0, 5.0, 7.0])),
            (
                "g".into(),
                Series::from_str_values(["a", "b", "c", "a", "b", "c"].map(String::from).to_vec()),
            ),
            ("h".into(), Series::from_i64(vec![0, 1, 0, 1, 0, 1])),
        ])
        .unwrap()
    }

    fn features(src: &str) -> Vec<String> {
        let f = Formula::parse(src).unwrap();
        let (terms, _) = code(&f, &frame()).unwrap();
        terms.iter().flat_map(|t| t.features()).collect()
    }

    #[test]
    fn terms_expand_like_patsy() {
        assert_eq!(features("y ~ x + g"), vec!["x", "g_b", "g_c"]);
        assert_eq!(features("y ~ x * C(h)"), vec!["x", "h_1", "x:h_1"]);
        // an interaction alone spans the cells, less the intercept
        assert_eq!(features("y ~ C(g):C(h)").len(), 3 * 2 - 1);
        assert_eq!(features("y ~ g * C(h)").len(), 2 + 1 + 2);
        // without an intercept the first factor is coded in full
        assert_eq!(features("y ~ g - 1"), vec!["g_a", "g_b", "g_c"]);
        assert_eq!(features("y ~ 0 + x + g").len(), 4);
        // a numeric factor in an interaction without its main effect
        assert_eq!(features("y ~ x:g"), vec!["x:g_a", "x:g_b", "x:g_c"]);
        assert_eq!(features("y ~ (x + I(x^2)):C(h)").len(), 4);
        assert_eq!(features("y ~ log(x) + I(x * 2 - 1)"), vec!["log(x)", "I(x*2-1)"]);
        // nesting: g / x is g + g:x (x's slope within each g)
        assert_eq!(features("y ~ g / x"), features("y ~ g + g:x"));
        assert_eq!(features("y ~ (g + C(h)) / x"), features("y ~ g + C(h) + g:C(h):x"));
        // powers: every interaction up to the order
        assert_eq!(features("y ~ (x + g + C(h))^2"), features("y ~ x + g + C(h) + x:g + x:C(h) + g:C(h)"));
        assert_eq!(features("y ~ (x + g)**2"), features("y ~ x * g"));
        assert_eq!(features("y ~ (x + g + C(h))^3"), features("y ~ x * g * C(h)"));
    }

    #[test]
    fn errors_point_into_the_formula() {
        let e = Formula::parse("y ~ x + ").unwrap_err().to_string();
        assert!(e.contains("missing"), "{e}");
        let f = Formula::parse("y ~ x + sqf").unwrap();
        let e = code(&f, &frame()).unwrap_err().to_string();
        assert!(e.contains("no column 'sqf'") && e.contains("            ^^^"), "{e}");
        let e = Formula::parse("y ~ (x + g)^1.5").unwrap_err().to_string();
        assert!(e.contains("whole number"), "{e}");
        let e = Formula::parse("y ~ foo(x)").unwrap_err().to_string();
        assert!(e.contains("'foo' is not a formula function"), "{e}");
        let f = Formula::parse("y ~ y + x").unwrap();
        assert!(code(&f, &frame()).unwrap_err().to_string().contains("also on the right"));
    }

    #[test]
    fn encodes_cells_and_unseen_levels() {
        let f = Formula::parse("y ~ C(g):C(h)").unwrap();
        let (terms, _) = code(&f, &frame()).unwrap();
        let mut cols = Vec::new();
        let mut valid = vec![true; 6];
        terms[0].encode(&frame(), &mut cols, &mut valid, false).unwrap();
        assert_eq!(cols.len(), 5);
        // with the intercept, the columns span the six cells: full rank
        let mut x = vec![vec![1.0; 6]];
        x.extend(cols);
        let k = x.len();
        let g: Vec<f64> = (0..k * k)
            .map(|p| x[p / k].iter().zip(&x[p % k]).map(|(a, b)| a * b).sum())
            .collect();
        assert!(crate::learn::linalg::cholesky(&g, k).is_some());
    }
}
