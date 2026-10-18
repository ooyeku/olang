//! A notebook's outputs written out (docs/notebooks.md, "Export" and
//! "Pinned output"): the page `olang notebook export --html` writes, one
//! output's HTML (what a published app swaps in as a control moves), and
//! the text a pinned output keeps in the file.
//!
//! Everything here reads **an outputs document**: the notebook's code
//! cells' replies as the kernel gave them (`olang repl --serve`'s `eval`
//! replies: a value's kind, a table's columns and first rows, a chart's
//! SVG and description, printed lines, an error), in the shape olang
//! Studio keeps a run in:
//!
//! ```json
//! {"v": 1, "cells": [{"name": "load", "line": 29, "reply": {…}, "page": [{"i": 0, "c": ["…"]}]}],
//!  "interp": [{"expr": "total", "reply": {…}}], "params": [{"name": "region", "value": "\"east\""}],
//!  "text": "…the notebook's text, when it is not the file's…"}
//! ```
//!
//! A cell is found by its first code line (`line`), else its `name`, else
//! its place among the cells with neither. `olang notebook export --json`
//! writes the document from a run; Studio writes it from what it drew.

use serde_json::{Value as J, json};
use std::fmt::Write as _;
use std::path::Path;

/// What a page is for: a file to read (controls drawn at their last
/// value, read only) or a published app's page (controls live).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Export,
    App,
}

/// How many of a table's rows a page or a pin shows.
pub const TABLE_ROWS: usize = 20;

// ── reading the document ────────────────────────────────────────────

/// The document's entry for each of `nb`'s code cells, in order (None
/// where it has none: a cell never run, a `no-run` cell).
pub fn cell_entries<'a>(nb: &crate::notebook::Notebook, doc: &'a J) -> Vec<Option<&'a J>> {
    let entries: Vec<&J> = doc
        .get("cells")
        .and_then(|c| c.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let mut used = vec![false; entries.len()];
    let mut out: Vec<Option<usize>> = vec![None; nb.cells.len()];
    // by line
    for (ci, c) in nb.cells.iter().enumerate() {
        let first = c.fence_line + 1;
        if let Some(k) = (0..entries.len()).find(|&k| {
            !used[k] && entries[k].get("line").and_then(|l| l.as_u64()) == Some(first as u64)
        }) {
            used[k] = true;
            out[ci] = Some(k);
        }
    }
    // by name
    for (ci, c) in nb.cells.iter().enumerate() {
        if out[ci].is_some() {
            continue;
        }
        if let Some(name) = &c.options.name
            && let Some(k) = (0..entries.len()).find(|&k| {
                !used[k] && entries[k].get("name").and_then(|n| n.as_str()) == Some(name.as_str())
            })
        {
            used[k] = true;
            out[ci] = Some(k);
        }
    }
    // the rest in order: an unnamed cell takes the next entry with no name
    // and no line
    let mut free = (0..entries.len()).filter(|&k| {
        !used[k]
            && entries[k].get("line").is_none_or(|l| l.is_null())
            && entries[k].get("name").is_none_or(|n| n.is_null())
    });
    for (ci, c) in nb.cells.iter().enumerate() {
        if out[ci].is_none() && c.options.name.is_none() && !c.no_run {
            out[ci] = free.next();
        }
    }
    out.into_iter().map(|k| k.map(|k| entries[k])).collect()
}

/// The `${expr}` values the document holds: expression → its reply.
fn interp_of(doc: &J) -> Vec<(String, J)> {
    match doc.get("interp") {
        Some(J::Array(a)) => a
            .iter()
            .filter_map(|e| {
                Some((
                    e.get("expr")?.as_str()?.to_string(),
                    e.get("reply")?.clone(),
                ))
            })
            .collect(),
        Some(J::Object(o)) => o.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => Vec::new(),
    }
}

// ── values in words ─────────────────────────────────────────────────

/// A float's text as a person reads it, as Studio draws one: six
/// significant digits, no trailing zeros but one after the point
/// (`0.290016`, `1204.5`, `12.0`); a very large or small one as it is.
pub fn human_float(s: &str) -> String {
    let Ok(x) = s.trim().parse::<f64>() else {
        return s.to_string();
    };
    let a = x.abs();
    if !x.is_finite() || a == 0.0 || !(1e-4..1e15).contains(&a) {
        return s.to_string();
    }
    let digits = (5 - a.log10().floor() as i32).clamp(0, 12) as usize;
    let t = format!("{:.*}", digits, x);
    if !t.contains('.') {
        return t;
    }
    let u = t.trim_end_matches('0');
    if u.ends_with('.') {
        format!("{}0", u)
    } else {
        u.to_string()
    }
}

/// A value's text as an output shows it in a line: a string bare, a float
/// as a person reads it, anything else as the kernel said it.
pub fn value_text(v: &J) -> String {
    let s = v.get("s").and_then(|s| s.as_str()).unwrap_or("");
    match v.get("k").and_then(|k| k.as_str()) {
        Some("float") => human_float(s),
        Some("unit") => String::new(),
        _ => s.to_string(),
    }
}

/// A value's source text without its quotes when it is a plain string
/// (`"east"` → `east`), as a run's label says it.
pub fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        serde_json::from_str::<String>(t).unwrap_or_else(|_| t[1..t.len() - 1].to_string())
    } else {
        t.to_string()
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

fn num_type(t: &str) -> bool {
    matches!(t, "Int" | "Float" | "int" | "float" | "i64" | "f64")
}

/// A table read: its columns, their types, its rows (a null `None`), its size.
type TableOf = (Vec<String>, Vec<String>, Vec<Vec<Option<String>>>, u64);

/// A table's columns, types and rows: a Frame's, a Series', a list of
/// records'. `page` (Studio's first page) when given, else the value's own
/// first rows.
fn table_of(v: &J, page: Option<&J>) -> Option<TableOf> {
    let table = v.get("table")?;
    let cols: Vec<String> = table
        .get("cols")?
        .as_array()?
        .iter()
        .map(|c| c.as_str().unwrap_or("").to_string())
        .collect();
    let types: Vec<String> = table
        .get("types")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .map(|t| t.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_else(|| vec![String::new(); cols.len()]);
    let cell = |c: &J| -> Option<String> {
        match c {
            J::Null => None,
            J::String(s) if s == "null" => None,
            J::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        }
    };
    let rows_of = |src: &J| -> Vec<Vec<Option<String>>> {
        src.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.get("c").and_then(|c| c.as_array()))
                    .map(|c| c.iter().map(cell).collect())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut rows = match page {
        Some(p) if p.is_array() => rows_of(p),
        _ => Vec::new(),
    };
    if rows.is_empty() {
        if let Some(r) = v.get("rows") {
            rows = rows_of(r);
        } else if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
            // a list of records: each item's entries by column
            rows = items
                .iter()
                .map(|it| {
                    cols.iter()
                        .map(|c| {
                            it.get("entries")
                                .and_then(|e| e.as_array())
                                .and_then(|es| {
                                    es.iter().find(|e| {
                                        e.get("key").and_then(|k| k.as_str()) == Some(c.as_str())
                                    })
                                })
                                .and_then(|e| e.get("v"))
                                .map(value_text)
                        })
                        .collect()
                })
                .collect();
        }
    }
    let n = v
        .get("n")
        .and_then(|n| n.as_u64())
        .unwrap_or(rows.len() as u64);
    rows.truncate(TABLE_ROWS);
    Some((cols, types, rows, n))
}

fn count_text(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn size_words(n: u64, cols: usize) -> String {
    format!(
        "{} row{} × {} column{}",
        count_text(n),
        if n == 1 { "" } else { "s" },
        cols,
        if cols == 1 { "" } else { "s" }
    )
}

// ── a pinned output ─────────────────────────────────────────────────

/// What a pinned cell's output block holds, from its reply: `(kind,
/// body)` — `table` (its columns aligned: names, types, a rule, its first
/// rows, its size), `chart` (the chart's SVG), `text` (a value's words,
/// what it printed above them, an error's message). None for a cell whose
/// output is nothing (its definitions), where no block is kept.
pub fn pin_of(reply: &J, page: Option<&J>) -> Option<(String, String)> {
    if reply.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        let m = reply
            .pointer("/error/message")
            .and_then(|m| m.as_str())
            .unwrap_or("an error");
        return Some(("text".into(), format!("error: {}", m)));
    }
    let v = reply.get("value").cloned().unwrap_or(J::Null);
    let printed = reply
        .get("out")
        .and_then(|o| o.as_str())
        .unwrap_or("")
        .trim_end()
        .to_string();
    if let Some(ctl) = reply.get("control")
        && ctl.get("kind").and_then(|k| k.as_str()) != Some("brush")
    {
        let name = ctl.get("name").and_then(|n| n.as_str()).unwrap_or("value");
        let val = ctl.get("value").map(value_text).unwrap_or_default();
        return Some(("text".into(), format!("{} = {}", name, val)));
    }
    if let Some(svg) = v.pointer("/svg/text").and_then(|t| t.as_str()) {
        return Some(("chart".into(), svg.to_string()));
    }
    if let Some((cols, types, rows, n)) = table_of(&v, page) {
        return Some(("table".into(), table_text(&cols, &types, &rows, n)));
    }
    let words = value_text(&v);
    let body = match (printed.is_empty(), words.is_empty()) {
        (true, true) => return None,
        (false, true) => printed,
        (true, false) => words,
        (false, false) => format!("{}\n\n{}", printed, words),
    };
    Some(("text".into(), body))
}

/// A table as aligned text: names, types, a rule of `─` under each
/// column, the rows, then its size. Every column starts where its rule
/// does, so a reader splits the rows by the rule.
pub fn table_text(
    cols: &[String],
    types: &[String],
    rows: &[Vec<Option<String>>],
    n: u64,
) -> String {
    let width = |s: &str| s.chars().count();
    let text = |c: &Option<String>| {
        c.clone()
            .unwrap_or_else(|| "null".to_string())
            .replace('\n', " ")
    };
    let mut w: Vec<usize> = cols.iter().map(|c| width(c).max(1)).collect();
    for (j, t) in types.iter().enumerate() {
        if j < w.len() {
            w[j] = w[j].max(width(t));
        }
    }
    for r in rows {
        for (j, c) in r.iter().enumerate() {
            if j < w.len() {
                w[j] = w[j].max(width(&text(c)).min(40));
            }
        }
    }
    let line = |cells: Vec<String>| -> String {
        let mut s = String::new();
        for (j, c) in cells.iter().enumerate() {
            let c: String = c.chars().take(40).collect();
            if j + 1 == cells.len() {
                s.push_str(&c);
            } else {
                s.push_str(&c);
                s.push_str(&" ".repeat(w[j] - width(&c) + 2));
            }
        }
        s.trim_end().to_string()
    };
    let mut out = vec![
        line(cols.to_vec()),
        line(
            (0..cols.len())
                .map(|j| types.get(j).cloned().unwrap_or_default())
                .collect(),
        ),
        line(w.iter().map(|&k| "─".repeat(k)).collect()),
    ];
    for r in rows {
        out.push(line(
            (0..cols.len())
                .map(|j| r.get(j).map(text).unwrap_or_default())
                .collect(),
        ));
    }
    let shown = rows.len() as u64;
    out.push(if shown < n {
        format!("{} · the first {} shown", size_words(n, cols.len()), shown)
    } else {
        size_words(n, cols.len())
    });
    out.join("\n")
}

// ── one output as HTML ──────────────────────────────────────────────

/// One cell's output as HTML: what it printed, then its value by kind (a
/// table, a chart in both themes, a picture, a control, a number, words,
/// a record's entries), or its error. `as_markdown`: a String drawn as
/// markdown. Empty for a cell whose output is nothing.
pub fn output_html(
    reply: &J,
    page: Option<&J>,
    mode: Mode,
    cell: &str,
    as_markdown: bool,
) -> String {
    let mut out = String::new();
    let printed = reply
        .get("out")
        .and_then(|o| o.as_str())
        .unwrap_or("")
        .trim_end();
    if !printed.is_empty() {
        let lines: Vec<&str> = printed.split('\n').collect();
        let _ = write!(
            out,
            "<div class=\"printed\"><div class=\"label\">Printed</div><pre>{}</pre></div>",
            esc(&lines.join("\n"))
        );
    }
    if reply.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        let e = reply.get("error").cloned().unwrap_or(J::Null);
        let msg = e
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("an error");
        let _ = write!(
            out,
            "<div class=\"error\"><div class=\"msg\">{}</div>",
            esc(msg)
        );
        if let Some(frames) = e.get("stack").and_then(|s| s.as_array())
            && !frames.is_empty()
        {
            out.push_str("<ol class=\"frames\">");
            for f in frames.iter().take(8) {
                let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let line = f
                    .get("line")
                    .and_then(|l| l.as_u64())
                    .map(|l| format!(" · line {}", l))
                    .unwrap_or_default();
                let _ = write!(out, "<li><code>{}</code>{}</li>", esc(name), line);
            }
            out.push_str("</ol>");
        }
        out.push_str("</div>");
        return out;
    }
    if let Some(ctl) = reply.get("control") {
        out.push_str(&control_html(ctl, mode, cell, reply.get("value")));
        return out;
    }
    let v = reply.get("value").cloned().unwrap_or(J::Null);
    out.push_str(&value_html(&v, page, as_markdown));
    out
}

/// A value as HTML, by its kind.
fn value_html(v: &J, page: Option<&J>, as_markdown: bool) -> String {
    let k = v.get("k").and_then(|k| k.as_str()).unwrap_or("");
    if let Some(svg) = v.pointer("/svg/text").and_then(|t| t.as_str()) {
        return chart_html(svg);
    }
    if let Some(png) = v.get("png").and_then(|p| p.as_str()) {
        let fmt = v
            .pointer("/img/format")
            .and_then(|f| f.as_str())
            .unwrap_or("png");
        return format!(
            "<figure class=\"picture\"><img alt=\"A picture\" src=\"data:image/{};base64,{}\"></figure>",
            esc(fmt),
            png
        );
    }
    if let Some(path) = v.get("img").and_then(|p| p.as_str())
        && let Ok(bytes) = std::fs::read(path)
    {
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_lowercase();
        let mime = match ext.as_str() {
            "svg" => "svg+xml",
            "jpg" | "jpeg" => "jpeg",
            "gif" => "gif",
            "webp" => "webp",
            _ => "png",
        };
        return format!(
            "<figure class=\"picture\"><img alt=\"A picture\" src=\"data:image/{};base64,{}\"></figure>",
            mime,
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
        );
    }
    if let Some((cols, types, rows, n)) = table_of(v, page) {
        return table_html(&cols, &types, &rows, n, v.get("stats"));
    }
    match k {
        "unit" | "decl" | "" => String::new(),
        "int" | "float" | "bool" => format!("<div class=\"number\">{}</div>", esc(&value_text(v))),
        "str" => {
            let s = v.get("s").and_then(|s| s.as_str()).unwrap_or("");
            if as_markdown {
                format!("<div class=\"prose md\">{}</div>", markdown_html(s))
            } else if s.contains('\n') || s.chars().count() > 120 {
                format!("<pre class=\"text\">{}</pre>", esc(s))
            } else {
                format!("<div class=\"words\">{}</div>", esc(s))
            }
        }
        "map" | "record" => {
            let entries = v
                .get("entries")
                .and_then(|e| e.as_array())
                .cloned()
                .unwrap_or_default();
            if entries.is_empty() {
                return format!("<div class=\"words mono\">{}</div>", esc(&value_text(v)));
            }
            let mut o = String::from("<table class=\"entries\"><tbody>");
            for e in &entries {
                let key = e.get("key").and_then(|k| k.as_str()).unwrap_or("");
                let val = e.get("v").cloned().unwrap_or(J::Null);
                let t = val.get("t").and_then(|t| t.as_str()).unwrap_or("");
                let _ = write!(
                    o,
                    "<tr><th>{}</th><td class=\"mono\">{}</td><td class=\"type\">{}</td></tr>",
                    esc(key),
                    esc(&short(&value_text(&val), 160)),
                    esc(t)
                );
            }
            let n = v
                .get("n")
                .and_then(|n| n.as_u64())
                .unwrap_or(entries.len() as u64);
            o.push_str("</tbody></table>");
            if n > entries.len() as u64 {
                let _ = write!(
                    o,
                    "<div class=\"foot\">{} entries · the first {} shown</div>",
                    count_text(n),
                    entries.len()
                );
            }
            o
        }
        _ => format!(
            "<div class=\"words mono\">{}</div>",
            esc(&short(&value_text(v), 2000))
        ),
    }
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

/// A table: each column's name over its type, numbers at the right in
/// the mono face, a null quiet; its size under it.
fn table_html(
    cols: &[String],
    types: &[String],
    rows: &[Vec<Option<String>>],
    n: u64,
    stats: Option<&J>,
) -> String {
    let mut o = String::from("<div class=\"table-wrap\"><table class=\"grid\"><thead><tr>");
    for (j, c) in cols.iter().enumerate() {
        let t = types.get(j).map(String::as_str).unwrap_or("");
        let _ = write!(
            o,
            "<th class=\"{}\"><span class=\"name\">{}</span><span class=\"type\">{}</span></th>",
            if num_type(t) { "num" } else { "" },
            esc(c),
            esc(t)
        );
    }
    o.push_str("</tr></thead><tbody>");
    for r in rows {
        o.push_str("<tr>");
        for j in 0..cols.len() {
            let t = types.get(j).map(String::as_str).unwrap_or("");
            match r.get(j).cloned().flatten() {
                Some(c) => {
                    let shown = if t == "Float" { human_float(&c) } else { c };
                    let _ = write!(
                        o,
                        "<td class=\"{}\">{}</td>",
                        if num_type(t) { "num" } else { "" },
                        esc(&shown)
                    );
                }
                None => o.push_str("<td class=\"null\">null</td>"),
            }
        }
        o.push_str("</tr>");
    }
    o.push_str("</tbody></table></div>");
    let shown = rows.len() as u64;
    let mut foot = size_words(n, cols.len());
    if shown < n {
        let _ = write!(foot, " · the first {} shown", shown);
    }
    if let Some(st) = stats {
        let f = |k: &str| {
            st.get(k)
                .and_then(|x| x.as_f64())
                .map(|x| human_float(&x.to_string()))
        };
        if let (Some(mean), Some(lo), Some(hi)) = (f("mean"), f("min"), f("max")) {
            let _ = write!(foot, " · mean {} · {} to {}", mean, lo, hi);
        }
    }
    let _ = write!(o, "<div class=\"foot\">{}</div>", esc(&foot));
    o
}

/// A chart as its SVG, drawn in both themes (the page shows the one its
/// reader's appearance asks for); its description rides inside.
fn chart_html(svg: &str) -> String {
    use olang_ods::plot::{Theme, retheme};
    let light = retheme(svg, Theme::Light);
    let dark = retheme(svg, Theme::Dark);
    if light == dark {
        return format!("<figure class=\"chart\">{}</figure>", svg);
    }
    format!(
        "<figure class=\"chart\"><div class=\"light-only\">{}</div><div class=\"dark-only\">{}</div></figure>",
        light, dark
    )
}

/// A control: its name, then the control at its value — read only on an
/// exported page, live on an app's (each change posts the cell and its
/// value).
fn control_html(ctl: &J, mode: Mode, cell: &str, value: Option<&J>) -> String {
    let kind = ctl.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    let name = ctl.get("name").and_then(|n| n.as_str()).unwrap_or("value");
    let val = ctl
        .get("value")
        .or(value)
        .map(value_text)
        .unwrap_or_default();
    let live = mode == Mode::App && ctl.get("name").and_then(|n| n.as_str()).is_some();
    let dis = if live { "" } else { " disabled" };
    let data = if live {
        format!(" data-cell=\"{}\" data-name=\"{}\"", esc(cell), esc(name))
    } else {
        String::new()
    };
    let num = |k: &str| {
        ctl.get(k).and_then(|x| {
            x.get("s")
                .and_then(|s| s.as_str())
                .map(str::to_string)
                .or_else(|| x.as_f64().map(|f| f.to_string()))
        })
    };
    let mut o = format!(
        "<div class=\"control\"><label class=\"name\" for=\"ctl-{}\">{}</label>",
        esc(cell),
        esc(name)
    );
    match kind {
        "slider" => {
            let lo = num("min").unwrap_or_else(|| "0".into());
            let hi = num("max").unwrap_or_else(|| "1".into());
            let step = num("step").unwrap_or_else(|| "any".into());
            // how far along its track the knob stands (the track filled to it)
            let f = |t: &str| t.parse::<f64>().ok();
            let pct = match (f(&lo), f(&hi), f(&val)) {
                (Some(a), Some(b), Some(v)) if b > a => {
                    ((v - a) / (b - a) * 100.0).clamp(0.0, 100.0)
                }
                _ => 0.0,
            };
            let _ = write!(
                o,
                "<span class=\"bound\">{}</span><input id=\"ctl-{}\" type=\"range\" style=\"--p:{:.1}%\" min=\"{}\" max=\"{}\" step=\"{}\" value=\"{}\"{}{}><span class=\"bound\">{}</span><output class=\"value\">{}</output>",
                esc(&human_float(&lo)),
                esc(cell),
                pct,
                esc(&lo),
                esc(&hi),
                esc(&step),
                esc(&val),
                data,
                dis,
                esc(&human_float(&hi)),
                esc(&val)
            );
        }
        "select" => {
            let _ = write!(o, "<select id=\"ctl-{}\"{}{}>", esc(cell), data, dis);
            for opt in ctl
                .get("options")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default()
            {
                let t = value_text(&opt);
                // the option as olang source: what a published app `set`s
                let raw = opt.get("s").and_then(|s| s.as_str()).unwrap_or("");
                let src = if opt.get("k").and_then(|k| k.as_str()) == Some("str") {
                    serde_json::to_string(raw).unwrap_or_default()
                } else {
                    raw.to_string()
                };
                let _ = write!(
                    o,
                    "<option value=\"{}\"{}>{}</option>",
                    esc(&src),
                    if t == val { " selected" } else { "" },
                    esc(&t)
                );
            }
            o.push_str("</select>");
        }
        "toggle" => {
            let _ = write!(
                o,
                "<input id=\"ctl-{}\" type=\"checkbox\" role=\"switch\"{}{}{}>",
                esc(cell),
                if val == "true" { " checked" } else { "" },
                data,
                dis
            );
        }
        "date" => {
            let _ = write!(
                o,
                "<input id=\"ctl-{}\" type=\"date\" value=\"{}\"{}{}>",
                esc(cell),
                esc(&val),
                data,
                dis
            );
        }
        "brush" => {
            // the chart under a layer the pointer drags a range on (the
            // page's script): its range said as data, drawn as it loads
            let range = brush_range(&val)
                .map(|(a, b)| format!(" data-x0=\"{}\" data-x1=\"{}\"", a, b))
                .unwrap_or_default();
            let _ = write!(
                o,
                "<div class=\"brush\" role=\"group\" aria-label=\"{}: drag across the chart to choose a range\"{}{}>",
                esc(name),
                data,
                range
            );
            if let Some(svg) = ctl.pointer("/svg/text").and_then(|t| t.as_str()) {
                o.push_str(&chart_html(svg));
            }
            o.push_str("</div>");
            let _ = write!(
                o,
                "<output class=\"value\">{}</output>",
                esc(if val.is_empty() || val == "()" {
                    "no range brushed"
                } else {
                    &val
                })
            );
        }
        _ => {
            let _ = write!(
                o,
                "<input id=\"ctl-{}\" type=\"text\" value=\"{}\"{}{}>",
                esc(cell),
                esc(&val),
                data,
                dis
            );
        }
    }
    o.push_str("</div>");
    o
}

/// A brush's range from its value's words (`[x0, x1]`), or `None`.
fn brush_range(val: &str) -> Option<(f64, f64)> {
    let inner = val.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut parts = inner.split(',').map(|p| p.trim().parse::<f64>());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Ok(a)), Some(Ok(b)), None) if a.is_finite() && b.is_finite() => Some((a, b)),
        _ => None,
    }
}

// ── prose ───────────────────────────────────────────────────────────

/// Markdown as HTML (CommonMark with tables and strikethrough); raw HTML
/// in it is escaped, never passed through.
pub fn markdown_html(text: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, html};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let events = Parser::new_ext(text, opts).map(|e| match e {
        Event::Html(h) | Event::InlineHtml(h) => Event::Text(h),
        e => e,
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

/// Prose with each `${expr}` made a placeholder (a private-use character
/// and its index), the expressions in order.
fn take_exprs(text: &str) -> (String, Vec<String>) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut exprs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && chars.get(i + 1) == Some(&'{') {
            let mut depth = 0;
            let mut j = i + 1;
            let mut end = None;
            while j < chars.len() {
                match chars[j] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(j);
                            break;
                        }
                    }
                    '\n' => break,
                    _ => {}
                }
                j += 1;
            }
            if let Some(end) = end {
                let e: String = chars[i + 2..end].iter().collect();
                let _ = write!(out, "\u{E000}{}\u{E001}", exprs.len());
                exprs.push(e.trim().to_string());
                i = end + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, exprs)
}

/// Prose as HTML, each `${expr}` its value (on a quiet well), or the
/// expression itself where no value is known.
fn prose_html(text: &str, values: &[(String, J)], mode: Mode, base: usize) -> String {
    let (md, exprs) = take_exprs(text);
    let mut html = markdown_html(&md);
    for (k, e) in exprs.iter().enumerate() {
        let token = format!("\u{E000}{}\u{E001}", k);
        let found = values
            .iter()
            .find(|(x, _)| x.trim() == e.as_str())
            .map(|(_, r)| r);
        let shown = match found {
            Some(r) if r.get("ok").and_then(|o| o.as_bool()) == Some(true) => {
                let v = r.get("value").cloned().unwrap_or(J::Null);
                format!(
                    "<span class=\"val\" data-expr=\"{}\">{}</span>",
                    base + k,
                    esc(&value_text(&v))
                )
            }
            _ => format!(
                "<code class=\"expr\" data-expr=\"{}\">{}</code>",
                base + k,
                esc(&format!("${{{}}}", e))
            ),
        };
        let _ = mode;
        html = html.replace(&token, &shown);
    }
    html
}

/// Every `${expr}` of the notebook's prose, in order (an app evaluates
/// them again after each change).
pub fn prose_exprs(text: &str) -> Vec<String> {
    let nb = crate::notebook::parse(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    for (from, to) in prose_runs(&nb, lines.len()) {
        let chunk = lines[from..to].join("\n");
        out.extend(take_exprs(&chunk).1);
    }
    out
}

/// The 0-based line ranges of the prose between cells, outputs and the
/// front matter.
fn prose_runs(nb: &crate::notebook::Notebook, n: usize) -> Vec<(usize, usize)> {
    let mut taken: Vec<(usize, usize)> = Vec::new();
    if let Some((o, c)) = nb.front_matter {
        taken.push((o as usize - 1, c as usize));
    }
    for c in &nb.cells {
        taken.push((c.fence_line as usize - 1, (c.close_line as usize).min(n)));
    }
    for o in &nb.outputs {
        taken.push((o.fence_line as usize - 1, (o.close_line as usize).min(n)));
    }
    taken.sort();
    let mut out = Vec::new();
    let mut at = 0;
    for (a, b) in taken {
        if a > at {
            out.push((at, a));
        }
        at = at.max(b);
    }
    if at < n {
        out.push((at, n));
    }
    out
}

// ── code ────────────────────────────────────────────────────────────

/// A cell's code highlighted as the language server colours it.
fn code_html(code: &str) -> String {
    use crate::tools::semantic::*;
    let toks = crate::tools::lsp::highlight_tokens(code);
    let mut out = String::new();
    for (ln, line) in code.split('\n').enumerate() {
        if ln > 0 {
            out.push('\n');
        }
        let units: Vec<u16> = line.encode_utf16().collect();
        let mut at = 0usize;
        let mut mine: Vec<&Token> = toks.iter().filter(|t| t.line as usize == ln).collect();
        mine.sort_by_key(|t| t.start);
        for t in mine {
            let (a, b) = (t.start as usize, (t.start + t.len) as usize);
            if a < at || b > units.len() {
                continue;
            }
            out.push_str(&esc(&String::from_utf16_lossy(&units[at..a])));
            let class = match t.kind {
                T_KEYWORD => "k",
                T_STRING => "s",
                T_NUMBER => "n",
                T_COMMENT => "c",
                T_FUNCTION | T_METHOD => "f",
                T_TYPE | T_ENUM_MEMBER => "t",
                T_NAMESPACE => "m",
                T_PROPERTY => "p",
                T_OPERATOR => "o",
                _ => "",
            };
            let text = esc(&String::from_utf16_lossy(&units[a..b]));
            if class.is_empty() {
                out.push_str(&text);
            } else {
                let _ = write!(out, "<span class=\"{}\">{}</span>", class, text);
            }
            at = b;
        }
        out.push_str(&esc(&String::from_utf16_lossy(
            &units[at.min(units.len())..],
        )));
    }
    out
}

// ── the page ────────────────────────────────────────────────────────

/// The grant in words (`fs read · net off`).
fn grant_words(nb: &crate::notebook::Notebook) -> String {
    let Some(c) = &nb.capabilities else {
        return String::new();
    };
    let fs = match c.fs {
        crate::caps::FsCap::Read => "fs read",
        crate::caps::FsCap::None => "fs off",
        crate::caps::FsCap::Full => "fs",
    };
    let on = |b: bool, w: &str| {
        if b {
            w.to_string()
        } else {
            format!("{} off", w)
        }
    };
    [
        fs.to_string(),
        on(c.net, "net"),
        on(c.proc, "proc"),
        on(c.db, "db"),
        on(c.env, "env"),
    ]
    .join(" · ")
}

/// The page for `text` (the notebook at `path`) and its outputs `doc`:
/// self-contained HTML (its styles inline, its charts as SVG, its pictures
/// as data), light and dark by the reader's appearance, printable.
pub fn page_html(text: &str, path: &Path, doc: &J, mode: Mode) -> String {
    let nb = crate::notebook::parse(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let entries = cell_entries(&nb, doc);
    let values = interp_of(doc);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("notebook");
    let title = nb.title.clone().unwrap_or_else(|| stem.to_string());
    let mut body = String::new();
    // the head: the title, the params, the grant
    let _ = write!(body, "<header class=\"head\"><h1>{}</h1>", esc(&title));
    let params: Vec<(String, String)> = match doc.get("params").and_then(|p| p.as_array()) {
        Some(ps) => ps
            .iter()
            .filter_map(|p| {
                Some((
                    p.get("name")?.as_str()?.to_string(),
                    p.get("value")?.as_str()?.to_string(),
                ))
            })
            .collect(),
        None => nb
            .params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect(),
    };
    if !params.is_empty() || nb.capabilities.is_some() {
        body.push_str("<div class=\"meta\">");
        for (name, value) in &params {
            if mode == Mode::App {
                let _ = write!(
                    body,
                    "<label class=\"chip param\"><span class=\"k\">{}</span><input class=\"v\" data-param=\"{}\" value=\"{}\" size=\"{}\"></label>",
                    esc(name),
                    esc(name),
                    esc(value),
                    value.chars().count().clamp(3, 24)
                );
            } else {
                let _ = write!(
                    body,
                    "<span class=\"chip\"><span class=\"k\">{}</span> = <span class=\"v\">{}</span></span>",
                    esc(name),
                    esc(value)
                );
            }
        }
        let g = grant_words(&nb);
        if !g.is_empty() {
            let _ = write!(body, "<span class=\"grant\">{}</span>", esc(&g));
        }
        body.push_str("</div>");
    }
    body.push_str("</header>");
    // prose and cells in the file's order
    let runs = prose_runs(&nb, lines.len());
    let mut items: Vec<(usize, Option<usize>)> = runs.iter().map(|(a, _)| (*a, None)).collect();
    for (ci, c) in nb.cells.iter().enumerate() {
        items.push((c.fence_line as usize - 1, Some(ci)));
    }
    items.sort();
    let mut expr_base = 0usize;
    let mut first_heading_done = false;
    for (at, cell) in items {
        match cell {
            None => {
                let (a, b) = runs
                    .iter()
                    .find(|(x, _)| *x == at)
                    .copied()
                    .unwrap_or((at, at));
                let mut chunk = lines[a..b].join("\n");
                if chunk.trim().is_empty() {
                    continue;
                }
                // the title is not said twice: a first heading that says it
                if !first_heading_done {
                    first_heading_done = true;
                    let t = chunk.trim_start();
                    if let Some(rest) = t.strip_prefix("# ")
                        && rest.lines().next().map(str::trim) == Some(title.trim())
                    {
                        chunk = rest.split_once('\n').map(|x| x.1).unwrap_or("").to_string();
                    }
                }
                let n_exprs = take_exprs(&chunk).1.len();
                let _ = write!(
                    body,
                    "<section class=\"prose\">{}</section>",
                    prose_html(&chunk, &values, mode, expr_base)
                );
                expr_base += n_exprs;
            }
            Some(ci) => {
                let c = &nb.cells[ci];
                let hide = c.options.hide.as_deref().unwrap_or("");
                if hide == "all" {
                    continue;
                }
                let first = c.fence_line as usize;
                let last = (c.close_line as usize - 1).min(lines.len());
                let code = lines[first.min(last)..last].join("\n");
                let entry = entries[ci];
                let id = format!("c{}", ci + 1);
                let name = c.options.name.clone();
                let reply = entry.and_then(|e| e.get("reply")).filter(|r| !r.is_null());
                let page = entry.and_then(|e| e.get("page")).filter(|p| !p.is_null());
                let _ = write!(
                    body,
                    "<section class=\"cell{}\" id=\"{}\" data-cell=\"{}\">",
                    if c.no_run { " no-run" } else { "" },
                    esc(&name
                        .clone()
                        .map(|n| format!("cell-{}", n))
                        .unwrap_or_else(|| id.clone())),
                    esc(&id)
                );
                // its row: the name, what it defines, what it reached, its time
                body.push_str("<div class=\"row\">");
                if let Some(n) = &name {
                    let _ = write!(body, "<span class=\"cname\">{}</span>", esc(n));
                }
                if let Some(r) = reply {
                    if let Some(defs) = r.get("defines").and_then(|d| d.as_array())
                        && !defs.is_empty()
                        && matches!(
                            r.pointer("/value/k").and_then(|k| k.as_str()),
                            Some("unit") | Some("decl") | None
                        )
                        && r.get("control").is_none()
                    {
                        body.push_str("<span class=\"defs\">defines");
                        for d in defs.iter().take(6) {
                            let _ = write!(body, " <code>{}</code>", esc(d.as_str().unwrap_or("")));
                        }
                        body.push_str("</span>");
                    }
                    if let Some(caps) = r.get("caps").and_then(|c| c.as_array()) {
                        for cap in caps {
                            let _ = write!(
                                body,
                                "<span class=\"badge\">{}</span>",
                                esc(cap.as_str().unwrap_or(""))
                            );
                        }
                    }
                    if let Some(ms) = r.get("ms").and_then(|m| m.as_f64()) {
                        let _ = write!(body, "<span class=\"time\">{}</span>", esc(&ms_text(ms)));
                    }
                } else if c.no_run {
                    body.push_str("<span class=\"time\">not run</span>");
                }
                body.push_str("</div>");
                if hide != "code" {
                    let _ = write!(
                        body,
                        "<pre class=\"code\"><code>{}</code></pre>",
                        code_html(&code)
                    );
                }
                if hide != "output" {
                    let html = match reply {
                        Some(r) => output_html(
                            r,
                            page,
                            mode,
                            &id,
                            c.options.as_.as_deref() == Some("markdown"),
                        ),
                        // nothing ran: the pinned output, when the file keeps one
                        None => pinned_html(text, c),
                    };
                    if !html.is_empty() || mode == Mode::App {
                        let _ = write!(
                            body,
                            "<div class=\"output\" id=\"out-{}\">{}</div>",
                            esc(&id),
                            html
                        );
                    }
                }
                body.push_str("</section>");
            }
        }
    }
    let generated = format!(
        "olang {} · {}",
        crate::version::VERSION,
        if mode == Mode::App {
            "a notebook published as an app"
        } else {
            "an exported notebook"
        }
    );
    // a page runs nothing but its brushes (a range dragged on a chart,
    // highlighted on the page) and, an app's, its controls' posts
    let brushes = body.contains("<div class=\"brush\"");
    let script = match (mode, brushes) {
        (Mode::App, _) => format!("{}{}", BRUSH_SCRIPT, APP_SCRIPT),
        (_, true) => format!(
            "{}\n<script>olangBrush(document, null);</script>",
            BRUSH_SCRIPT
        ),
        _ => String::new(),
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<meta name=\"color-scheme\" content=\"light dark\">\n<meta name=\"generator\" content=\"{}\">\n<title>{}</title>\n<style>{}</style>\n</head>\n<body>\n<main class=\"nb\">\n{}\n<footer class=\"foot-note\">{} · {}</footer>\n</main>{}\n</body>\n</html>\n",
        esc(&generated),
        esc(&title),
        PAGE_CSS,
        body,
        esc(&format!("{}.olmd", stem)),
        esc(&generated),
        script
    )
}

/// A cell's pinned output, from the file (drawn where nothing ran).
fn pinned_html(text: &str, c: &crate::notebook::Cell) -> String {
    let Some(name) = &c.options.name else {
        return String::new();
    };
    let Some((o, body)) = crate::notebook::pinned_outputs(text)
        .into_iter()
        .find(|(o, _)| &o.cell == name)
    else {
        return String::new();
    };
    let inner = match o.kind.as_str() {
        "chart" => chart_html(&body),
        _ => format!("<pre class=\"text pinned\">{}</pre>", esc(&body)),
    };
    format!("<div class=\"pinned-note\">pinned</div>{}", inner)
}

fn ms_text(ms: f64) -> String {
    if ms < 1.0 {
        "< 1 ms".to_string()
    } else if ms < 1000.0 {
        format!("{} ms", ms.round() as i64)
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

/// The page's styles: Studio's notebook as a document — its tokens (Loom's
/// light and dark themes), the 680 reading measure, cells as quiet
/// surfaces with a hairline edge, outputs under them on the page.
const PAGE_CSS: &str = r#"
:root{--bg:#ffffff;--surface:#fafafa;--fg:#18181b;--muted:#71717a;--hairline:#e4e4e7;--border:#d4d4d8;--segment:#f1f1f3;--link:#2563eb;--accent:#2563eb;--danger:#b91c1c;--warn:#b45309;
--k:#7c3aed;--f:#1d4ed8;--t:#0f766e;--s:#15803d;--n:#c2410c;--c:#6a6a73;--m:#0e7490;--p:#3f3f46;--o:#52525b;color-scheme:light}
@media (prefers-color-scheme:dark){:root{--bg:#111113;--surface:#161618;--fg:#ededef;--muted:#a1a1aa;--hairline:#2a2a2e;--border:#3f3f46;--segment:#242428;--link:#60a5fa;--accent:#3b82f6;--danger:#f87171;--warn:#fbbf24;
--k:#c4a2fb;--f:#7cb4fa;--t:#5eead4;--s:#8fdfa5;--n:#fdba74;--c:#8b8b94;--m:#67d4e8;--p:#d4d4d8;--o:#a1a1aa;color-scheme:dark}}
*{box-sizing:border-box}
html{background:var(--bg)}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/24px -apple-system,BlinkMacSystemFont,"SF Pro Text","Segoe UI",system-ui,sans-serif;-webkit-font-smoothing:antialiased}
.nb{max-width:944px;margin:0 auto;padding:48px 32px 64px}
.head{margin:0 0 24px}
.head h1{font-size:28px;line-height:36px;font-weight:700;letter-spacing:-.01em;margin:0 0 12px}
.meta{display:flex;flex-wrap:wrap;gap:8px;align-items:center}
.chip{font:12px/22px "SF Mono",ui-monospace,Menlo,monospace;border:1px solid var(--hairline);border-radius:6px;padding:0 8px;color:var(--fg)}
.chip .k{color:var(--muted)}
.chip input{font:inherit;color:inherit;background:transparent;border:0;outline:0;padding:0 0 0 6px;width:auto}
.chip.param:focus-within{border-color:var(--accent)}
.grant{font-size:12px;color:var(--muted);margin-left:4px}
.prose{max-width:680px;margin:0 auto 16px;padding:0}
.prose h1{font-size:26px;line-height:32px;margin:32px 0 12px;font-weight:700}
.prose h2{font-size:20px;line-height:28px;margin:28px 0 10px;font-weight:650}
.prose h3{font-size:16px;line-height:24px;margin:24px 0 8px;font-weight:650}
.prose p{margin:0 0 12px}
.prose a{color:var(--link);text-decoration:none}
.prose a:hover{text-decoration:underline}
.prose blockquote{margin:0 0 12px;padding-left:16px;color:var(--muted)}
.prose ul,.prose ol{margin:0 0 12px;padding-left:24px}
.prose code,.prose .val,.prose .expr{font:13px/20px "SF Mono",ui-monospace,Menlo,monospace;background:var(--segment);border-radius:4px;padding:1px 5px}
.prose .val{font:inherit;font-weight:600;padding:1px 6px}
.prose .expr{color:var(--muted)}
.prose pre{font:13px/20px "SF Mono",ui-monospace,Menlo,monospace;background:var(--surface);border:1px solid var(--hairline);border-radius:8px;padding:12px 14px;overflow-x:auto}
.prose pre code{background:none;padding:0}
.prose table{border-collapse:collapse;margin:0 0 12px;font-size:14px}
.prose th,.prose td{border-bottom:1px solid var(--hairline);padding:4px 12px 4px 0;text-align:left}
.cell{background:var(--surface);border:1px solid var(--hairline);border-radius:8px;margin:16px 0;overflow:hidden}
.cell .row{display:flex;gap:8px;align-items:center;min-height:26px;padding:4px 14px 0;font-size:11px;color:var(--muted)}
.cell .cname{font:12px/18px "SF Mono",ui-monospace,Menlo,monospace;color:var(--muted)}
.cell .defs code{font:11px/16px "SF Mono",ui-monospace,Menlo,monospace;border:1px solid var(--hairline);border-radius:4px;padding:0 4px;color:var(--fg)}
.cell .badge{font:11px/16px "SF Mono",ui-monospace,Menlo,monospace;border:1px solid var(--hairline);border-radius:4px;padding:0 4px}
.cell .time{margin-left:auto;font:11px/16px "SF Mono",ui-monospace,Menlo,monospace}
.code{margin:0;padding:6px 14px 12px;font:13px/20px "SF Mono",ui-monospace,Menlo,monospace;overflow-x:auto;white-space:pre;color:var(--fg)}
.code .k{color:var(--k)}.code .f{color:var(--f)}.code .t{color:var(--t)}.code .s{color:var(--s)}.code .n{color:var(--n)}.code .c{color:var(--c);font-style:italic}.code .m{color:var(--m)}.code .p{color:var(--p)}.code .o{color:var(--o)}
.output{background:var(--bg);border-top:1px solid var(--hairline);padding:12px 14px;overflow-x:auto}
.output:empty{display:none}
.printed{margin:0 0 10px}
.printed .label,.pinned-note{font-size:11px;color:var(--muted);margin:0 0 2px}
.printed pre,.text{margin:0;font:12px/18px "SF Mono",ui-monospace,Menlo,monospace;white-space:pre-wrap;color:var(--fg)}
.printed pre{color:var(--muted)}
.words{font-size:13px;line-height:20px}
.mono,.number{font:13px/20px "SF Mono",ui-monospace,Menlo,monospace}
.number{font-size:15px}
.error .msg{color:var(--danger);font-size:13px;line-height:20px;white-space:pre-wrap}
.error .frames{margin:6px 0 0;padding-left:20px;font-size:12px;color:var(--muted)}
.table-wrap{overflow-x:auto;max-width:100%}
table.grid{border-collapse:collapse;font-size:13px;line-height:20px}
table.grid th{text-align:left;font-weight:600;padding:2px 16px 6px 0;border-bottom:1px solid var(--hairline);vertical-align:bottom;white-space:nowrap}
table.grid th .name{display:block;font-size:12px}
table.grid th .type{display:block;font:11px/16px "SF Mono",ui-monospace,Menlo,monospace;color:var(--muted);font-weight:400}
table.grid td{padding:3px 16px 3px 0;border-bottom:1px solid var(--hairline);white-space:nowrap;font:12px/20px "SF Mono",ui-monospace,Menlo,monospace}
table.grid .num{text-align:right}
table.grid td.null{color:var(--muted)}
.foot{font-size:11px;color:var(--muted);margin-top:6px}
table.entries{border-collapse:collapse;font-size:13px}
table.entries th{text-align:left;font-weight:600;padding:2px 16px 2px 0;font-size:12px;vertical-align:top}
table.entries td{padding:2px 16px 2px 0;vertical-align:top}
table.entries td.type{font:11px/20px "SF Mono",ui-monospace,Menlo,monospace;color:var(--muted)}
figure{margin:0}
.chart svg{display:block;max-width:100%;height:auto;border-radius:6px}
.picture img{display:block;max-width:100%;height:auto;border-radius:6px}
.dark-only{display:none}
@media (prefers-color-scheme:dark){.light-only{display:none}.dark-only{display:block}}
.control{display:flex;align-items:center;gap:10px;min-height:28px;font-size:13px}
.control .name{font:12px/20px "SF Mono",ui-monospace,Menlo,monospace;color:var(--fg);min-width:64px}
.control .bound{font:11px/16px "SF Mono",ui-monospace,Menlo,monospace;color:var(--muted)}
.control input[type=range]{-webkit-appearance:none;appearance:none;flex:1;max-width:360px;height:22px;margin:0;background:transparent}
.control input[type=range]::-webkit-slider-runnable-track{height:3px;border-radius:2px;background:linear-gradient(to right,var(--accent) var(--p,0%),var(--hairline) var(--p,0%))}
.control input[type=range]::-webkit-slider-thumb{-webkit-appearance:none;width:14px;height:14px;border-radius:50%;margin-top:-5.5px;background:var(--bg);border:1px solid var(--border);box-shadow:0 1px 2px rgba(0,0,0,.18)}
.control input[type=range]::-moz-range-track{height:3px;border-radius:2px;background:var(--hairline)}
.control input[type=range]::-moz-range-progress{height:3px;border-radius:2px;background:var(--accent)}
.control input[type=range]::-moz-range-thumb{width:14px;height:14px;border-radius:50%;background:var(--bg);border:1px solid var(--border)}
.control .value{font:12px/20px "SF Mono",ui-monospace,Menlo,monospace;border:1px solid var(--hairline);border-radius:6px;padding:0 8px;min-width:48px;text-align:right}
.control select,.control input[type=text],.control input[type=date]{font:13px/20px inherit;color:var(--fg);background:var(--bg);border:1px solid var(--border);border-radius:6px;padding:2px 8px;height:26px}
.control :disabled{opacity:1}
.control input[type=range]:disabled::-webkit-slider-thumb{box-shadow:none}
.brush{position:relative;flex:1;min-width:0;max-width:720px;cursor:crosshair;touch-action:none;-webkit-user-select:none;user-select:none}
.brush .sel{position:absolute;display:none;pointer-events:none;background:color-mix(in srgb,var(--accent) 14%,transparent);border-left:1px solid var(--accent);border-right:1px solid var(--accent)}
.control:has(.brush){align-items:flex-start;flex-wrap:wrap}
.no-run .code{opacity:.75}
.foot-note{margin:48px auto 0;max-width:680px;font-size:11px;color:var(--muted);border-top:1px solid var(--hairline);padding-top:12px}
.busy .output{opacity:.55;transition:opacity .12s}
@media print{:root{--bg:#fff;--surface:#fafafa;--fg:#18181b;--muted:#5f5f68;--hairline:#e4e4e7;--segment:#f1f1f3;color-scheme:light}
.light-only{display:block!important}.dark-only{display:none!important}.nb{padding:0;max-width:none}.cell{break-inside:avoid}}
@media (max-width:640px){.nb{padding:24px 16px 48px}}
"#;

/// A brush on a page: a range dragged across its chart (the pointer
/// mapped through the chart's description: its `x` extent over its
/// `plot`'s width), shaded, the points outside it dimmed, its words said
/// beside it; a press without a drag clears it. `olangBrush(root, send)`
/// binds every brush under `root` once; `send(el, literal)` hears a range
/// chosen (an app's: `[x0, x1]` or `()` as olang source), or nothing on
/// an exported page.
const BRUSH_SCRIPT: &str = r#"
<script>
window.olangBrush = (function () {
  function desc(svg) {
    var m = svg && svg.querySelector('metadata');
    try { return m ? JSON.parse(m.textContent) : null; } catch (e) { return null; }
  }
  function shown(el) {
    var all = el.querySelectorAll('svg');
    for (var i = 0; i < all.length; i++) { if (all[i].getBoundingClientRect().width > 0) return all[i]; }
    return all[0] || null;
  }
  function lit(v) {
    var t = String(Number(v.toPrecision(6)));
    return /[.e]/.test(t) ? t : t + '.0';
  }
  return function (root, send) {
    root.querySelectorAll('.brush').forEach(function (el) {
      if (el.dataset.brushed) return;
      var d = desc(shown(el));
      if (!d || !d.plot || !d.x || !(d.x.hi > d.x.lo)) return;
      el.dataset.brushed = '1';
      var sel = document.createElement('div');
      sel.className = 'sel';
      el.appendChild(sel);
      var out = el.parentNode.querySelector('output');
      var range = el.dataset.x0 !== undefined ? [parseFloat(el.dataset.x0), parseFloat(el.dataset.x1)] : null;
      var from = null;
      function geo() {
        var svg = shown(el), r = svg.getBoundingClientRect(), e = el.getBoundingClientRect();
        return { k: r.width / d.width, ox: r.left - e.left, oy: r.top - e.top, left: e.left };
      }
      function toX(clientX) {
        var g = geo(), f = ((clientX - g.left - g.ox) / g.k - d.plot.left) / d.plot.w;
        return d.x.lo + Math.max(0, Math.min(1, f)) * (d.x.hi - d.x.lo);
      }
      function toPx(x) {
        var g = geo();
        return g.ox + (d.plot.left + (x - d.x.lo) / (d.x.hi - d.x.lo) * d.plot.w) * g.k;
      }
      function draw() {
        var g = geo();
        if (range) {
          var a = toPx(range[0]), b = toPx(range[1]);
          sel.style.display = 'block';
          sel.style.left = a + 'px';
          sel.style.width = Math.max(1, b - a) + 'px';
          sel.style.top = (g.oy + d.plot.top * g.k) + 'px';
          sel.style.height = (d.plot.h * g.k) + 'px';
        } else {
          sel.style.display = 'none';
        }
        el.querySelectorAll('svg circle').forEach(function (c) {
          var cx = parseFloat(c.getAttribute('cx')), cy = parseFloat(c.getAttribute('cy'));
          if (!(cy >= d.plot.top - 1 && cy <= d.plot.top + d.plot.h + 1)) return;
          var v = d.x.lo + (cx - d.plot.left) / d.plot.w * (d.x.hi - d.x.lo);
          var inside = !range || (v >= range[0] - 1e-9 * Math.abs(d.x.hi - d.x.lo) && v <= range[1] + 1e-9 * Math.abs(d.x.hi - d.x.lo));
          c.style.opacity = inside ? '' : '0.22';
        });
        if (out) out.textContent = range ? '[' + lit(range[0]) + ', ' + lit(range[1]) + ']' : 'no range brushed';
      }
      el.addEventListener('pointerdown', function (e) {
        if (e.button !== 0) return;
        from = toX(e.clientX);
        range = [from, from];
        el.setPointerCapture(e.pointerId);
        draw();
        e.preventDefault();
      });
      el.addEventListener('pointermove', function (e) {
        if (from === null) return;
        var x = toX(e.clientX);
        range = [Math.min(from, x), Math.max(from, x)];
        draw();
      });
      function end() {
        if (from === null) return;
        from = null;
        if (range && toPx(range[1]) - toPx(range[0]) < 3) range = null;
        draw();
        if (send) send(el, range ? '[' + lit(range[0]) + ', ' + lit(range[1]) + ']' : '()');
      }
      el.addEventListener('pointerup', end);
      el.addEventListener('pointercancel', end);
      window.addEventListener('resize', draw);
      draw();
    });
  };
})();
</script>"#;

/// A published app's page: each control change posts its cell and value,
/// and the outputs the server ran again are swapped in.
const APP_SCRIPT: &str = r#"
<script>
(function () {
  var main = document.querySelector('.nb');
  function apply(r) {
    if (!r) return;
    var outs = r.outputs || {};
    Object.keys(outs).forEach(function (id) {
      var el = document.getElementById('out-' + id);
      if (el) el.innerHTML = outs[id];
    });
    var vals = r.interp || {};
    Object.keys(vals).forEach(function (i) {
      document.querySelectorAll('[data-expr="' + i + '"]').forEach(function (el) {
        el.className = 'val'; el.textContent = vals[i];
      });
    });
    bind();
  }
  var pending = null, busy = false;
  function post(path, body) {
    if (busy) { pending = [path, body]; return; }
    busy = true; main.classList.add('busy');
    fetch(path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })
      .then(function (r) { return r.json(); })
      .then(function (r) { apply(r.data || r); })
      .catch(function () {})
      .then(function () {
        busy = false; main.classList.remove('busy');
        if (pending) { var p = pending; pending = null; post(p[0], p[1]); }
      });
  }
  function literal(el) {
    if (el.type === 'checkbox') return el.checked ? 'true' : 'false';
    if (el.tagName === 'SELECT' || el.type === 'range') return el.value;
    return JSON.stringify(el.value);
  }
  function bind() {
    // a brush: a range dragged on its chart is set, as a slider's value is
    window.olangBrush(document, function (el, value) {
      if (el.dataset.cell) post('/api/rpc/nb.set', { cell: el.dataset.cell, value: value });
    });
    document.querySelectorAll('[data-cell][data-name]').forEach(function (el) {
      if (el.dataset.bound || el.classList.contains('brush')) return;
      el.dataset.bound = '1';
      var send = function () { post('/api/rpc/nb.set', { cell: el.dataset.cell, value: literal(el) }); };
      if (el.type === 'range') {
        var out = el.parentNode.querySelector('output');
        el.addEventListener('input', function () {
          if (out) out.textContent = el.value;
          var lo = parseFloat(el.min), hi = parseFloat(el.max), v = parseFloat(el.value);
          if (hi > lo) el.style.setProperty('--p', ((v - lo) / (hi - lo) * 100) + '%');
          send();
        });
      } else {
        el.addEventListener('change', send);
      }
    });
    document.querySelectorAll('[data-param]').forEach(function (el) {
      if (el.dataset.bound) return;
      el.dataset.bound = '1';
      el.addEventListener('change', function () { post('/api/rpc/nb.param', { name: el.dataset.param, value: el.value }); });
    });
  }
  bind();
})();
</script>"#;

/// The page printed to a PDF by WebKit (`osascript -l JavaScript` this
/// `page.html out.pdf`): loaded in a web view in a window of its own,
/// printed as Safari prints (the width fit, pages as they fall, half-inch
/// margins), the print's end waited for.
pub const PDF_SCRIPT: &str = r#"ObjC.import('Cocoa');
ObjC.import('WebKit');
let DONE = false;
ObjC.registerSubclass({ name: 'OlangPrintDone', methods: { 'printOperationDidRun:success:contextInfo:': { types: ['void', ['id', 'bool', 'void *']], implementation: function (op, ok, ctx) { DONE = true; } } } });
function run(argv) {
  $.NSApplication.sharedApplication;
  const frame = $.NSMakeRect(0, 0, 816, 1056);
  const win = $.NSWindow.alloc.initWithContentRectStyleMaskBackingDefer(frame, 0, $.NSBackingStoreBuffered, false);
  const view = $.WKWebView.alloc.initWithFrameConfiguration(frame, $.WKWebViewConfiguration.alloc.init);
  view.appearance = $.NSAppearance.appearanceNamed($.NSAppearanceNameAqua);
  win.contentView = view;
  const url = $.NSURL.fileURLWithPath(argv[0]);
  view.loadFileURLAllowingReadAccessToURL(url, url.URLByDeletingLastPathComponent);
  const spin = (until, secs) => { const end = Date.now() + secs * 1000; while (!until() && Date.now() < end) { $.NSRunLoop.currentRunLoop.runModeBeforeDate($.NSDefaultRunLoopMode, $.NSDate.dateWithTimeIntervalSinceNow(0.05)); } };
  spin(() => !view.isLoading, 30);
  spin(() => false, 0.3);
  const info = $.NSPrintInfo.alloc.initWithDictionary($.NSMutableDictionary.dictionary);
  info.dictionary.setObjectForKey($.NSPrintSaveJob, $.NSPrintJobDisposition);
  info.dictionary.setObjectForKey($.NSURL.fileURLWithPath(argv[1]), $.NSPrintJobSavingURL);
  info.horizontalPagination = 1;
  info.verticalPagination = 0;
  info.verticallyCentered = false;
  info.topMargin = 36; info.bottomMargin = 36; info.leftMargin = 36; info.rightMargin = 36;
  const op = view.printOperationWithPrintInfo(info);
  op.showsPrintPanel = false;
  op.showsProgressPanel = false;
  op.runOperationModalForWindowDelegateDidRunSelectorContextInfo(win, $.OlangPrintDone.alloc.init, 'printOperationDidRun:success:contextInfo:', null);
  spin(() => DONE, 60);
  return DONE ? 'ok' : 'timeout';
}
"#;

// ── a notebook as an app ────────────────────────────────────────────

/// Where web-sdk is (the framework a published app serves with): `given`,
/// else `WEB_SDK_DIR`, else the shelf's `web`, else beside this olang's
/// own checkout (`frameworks/web-sdk` above the binary).
pub fn web_sdk_dir(given: Option<&Path>) -> Option<std::path::PathBuf> {
    let ok = |p: &Path| p.join("olang.toml").is_file() && p.join("lib/server.ol").is_file();
    if let Some(g) = given {
        return ok(g).then(|| std::path::absolute(g).unwrap_or(g.to_path_buf()));
    }
    if let Ok(d) = std::env::var("WEB_SDK_DIR")
        && ok(Path::new(&d))
    {
        return Some(d.into());
    }
    let shelf = std::env::var("OLANG_SHELF")
        .map(std::path::PathBuf::from)
        .ok()
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| Path::new(&h).join(".olang/shelf"))
        });
    if let Some(s) = shelf
        && ok(&s.join("web"))
    {
        return Some(s.join("web"));
    }
    let exe = std::env::current_exe().ok()?;
    let mut at = exe.parent();
    while let Some(d) = at {
        let c = d.join("frameworks/web-sdk");
        if ok(&c) {
            return Some(c);
        }
        at = d.parent();
    }
    None
}

/// A published app for the notebook at `path`, written into `out`: its
/// `olang.toml` (web-sdk its one dependency), `main.ol` (a web-sdk server
/// that starts the notebook's kernel, runs every cell, serves the page and
/// runs again what a control or a param reaches) and `notebook.json` (its
/// cells, its prose's expressions, its params: what the server reads).
/// Answers the files written.
pub fn write_app(
    path: &Path,
    out: &Path,
    web: &Path,
    port: u16,
) -> Result<Vec<std::path::PathBuf>, String> {
    let abs = std::path::absolute(path).unwrap_or(path.to_path_buf());
    let text = std::fs::read_to_string(&abs).map_err(|e| format!("{}: {}", abs.display(), e))?;
    let nb = crate::notebook::parse(&text);
    if !nb.diagnostics.is_empty() {
        return Err(crate::notebook::report(&abs, &nb.diagnostics));
    }
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let cells: Vec<J> = nb
        .cells
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.no_run)
        .map(|(i, c)| {
            let from = c.fence_line as usize;
            let to = (c.close_line as usize - 1).min(lines.len());
            json!({ "id": format!("c{}", i + 1), "name": c.options.name, "line": c.fence_line + 1,
                "code": lines[from.min(to)..to].join("\n"), "markdown": c.options.as_.as_deref() == Some("markdown") })
        })
        .collect();
    let params: Vec<J> = nb
        .params
        .iter()
        .map(|p| json!({ "name": p.name, "value": p.value, "line": p.line }))
        .collect();
    let stem = abs
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("notebook")
        .to_string();
    let title = nb.title.clone().unwrap_or_else(|| stem.clone());
    let spec = json!({ "v": 1, "notebook": abs.to_string_lossy(), "title": title, "port": port,
        "cells": cells, "exprs": prose_exprs(&text), "params": params });
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {}", out.display(), e))?;
    let name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let manifest = format!(
        "# {title}: {stem}.olmd published as an app (olang notebook app). Local only:\n#   olang run main.ol [port]\n[package]\nname = \"{name}-app\"\nversion = \"0.1.0\"\n\n[dependencies]\nweb = {{ path = {web} }}\n",
        title = title.replace('\n', " "),
        stem = stem,
        name = name,
        web = serde_json::to_string(&web.to_string_lossy()).unwrap_or_default()
    );
    let files = [
        (out.join("olang.toml"), manifest),
        (
            out.join("main.ol"),
            APP_MAIN
                .replace("{TITLE}", &title.replace('\n', " "))
                .replace("{STEM}", &stem),
        ),
        (
            out.join("notebook.json"),
            serde_json::to_string_pretty(&spec).unwrap_or_default(),
        ),
    ];
    let mut written = Vec::new();
    for (p, t) in files {
        std::fs::write(&p, t).map_err(|e| format!("{}: {}", p.display(), e))?;
        written.push(p);
    }
    Ok(written)
}

/// A published app's server (olang, web-sdk): the notebook's kernel a
/// task owns (one request at a time), every cell run as it starts, the
/// page drawn by the kernel from the outputs it holds, and a control or a
/// param changed running again the cells that read it, near or far.
const APP_MAIN: &str = r#"//! {TITLE}: {STEM}.olmd published as an app (`olang notebook app`).
//! The notebook's kernel runs it; moving a control or changing a param
//! runs again only what reads it, and the page swaps those outputs in.
//! Local only: nothing here is deployed.
//!
//!     olang run main.ol [port]

use web { serve, route, rpc }

let HERE = match fs.real_path(os.args()[0]) { Ok(p) => fs.dirname(p), Err(e) => "." }
let SPEC = unwrap(json.parse(unwrap(fs.read_file(fs.join(HERE, "notebook.json")))))
let PORT = if len(os.args()) > 1 => unwrap(str.parse_int(os.args()[1])) else => SPEC.port
let OLANG = match os.exe_path() { Ok(p) => p, Err(e) => "olang" }

// One request to the kernel, its reply (what it says in between skipped).
fn ask(k, id, req) = {
    let _w = proc.write_line(k, unwrap(json.stringify(map_set(req, "id", id))))
    let mut reply = ()
    while reply == () {
        match proc.read_line(k) {
            Ok(line) => {
                let r = match json.parse(line) { Ok(v) => v, Err(e) => #{} }
                if r.id == id => { reply = r }
            },
            Err(e) => { reply = #{ "ok": false, "error": #{ "message": "the kernel ended" } } }
        }
    }
    reply
}

// What each cell defines and reads, from the kernel's `deps`.
fn names_of(xs) = map(xs ?? [], (x) => x.name)

// The cells to run again after `names` changed, in order: each that reads
// one of them, and what reads those (the cell `from` first, when given).
fn reached(cells, deps, names, from) = {
    let mut dirty = names
    let mut out = []
    for c in cells {
        let d = map_get(deps, c.id) ?? #{ "defines": [], "uses": [] }
        let reads = len(filter(d.uses, (u) => contains(dirty, u))) > 0
        if c.id == from || reads => {
            out = out + [c]
            dirty = dirty + d.defines
        }
    }
    out
}

// The state the kernel's task keeps: each cell's reply, each expression's
// text, the params and the notebook's text (its front matter edited).
fn run_cells(k, st, cells) = fold(cells, st, (s, c) => {
    let r = ask(k, s.n + 1, #{ "op": "eval", "cell": c.id, "code": c.code, "line": c.line, "html": #{ "markdown": c.markdown } })
    s |> map_set("n", s.n + 1) |> map_set("replies", map_set(s.replies, c.id, r)) |> map_set("fresh", s.fresh + [c.id])
})

fn run_exprs(k, st) = fold(range(0, len(SPEC.exprs)), st, (s, i) => {
    let r = ask(k, s.n + 1, #{ "op": "eval", "code": SPEC.exprs[i], "pure": true, "html": true })
    s |> map_set("n", s.n + 1) |> map_set("interp", map_set(s.interp, to_string(i), r))
})

// What a change answers: the outputs run again, as HTML, and every
// expression's text.
fn answer(st) = #{
    "outputs": fold(st.fresh, #{}, (acc, id) => map_set(acc, id, (map_get(st.replies, id) ?? #{}).html ?? "")),
    "interp": fold(map_keys(st.interp), #{}, (acc, i) => {
        let r = map_get(st.interp, i)
        if r.ok == true => map_set(acc, i, r.text ?? "") else => acc
    })
}

// The page: the kernel draws it from the outputs held.
fn page(k, st) = {
    let doc = #{
        "cells": map(SPEC.cells, (c) => #{ "line": c.line, "name": c.name, "reply": map_get(st.replies, c.id) }),
        "interp": map(range(0, len(SPEC.exprs)), (i) => #{ "expr": SPEC.exprs[i], "reply": map_get(st.interp, to_string(i)) }),
        "params": map(st.params, (p) => #{ "name": p.name, "value": p.value })
    }
    ask(k, st.n + 1, #{ "op": "page", "mode": "app", "doc": doc, "text": st.text })
}

// A param's front matter line with a new value (a control's spec keeps
// its options and bounds).
fn yaml_value(v) = {
    let t = str.trim(v)
    let plain = match str.parse_float(t) { Ok(x) => true, Err(e) => t == "true" || t == "false" || str.starts_with(t, "[") || str.starts_with(t, "\"") }
    if plain => t else => unwrap(json.stringify(t))
}

fn with_param(text_, p, v) = {
    let lines = str.split(text_, "\n")
    let i = p.line - 1
    let l = lines[i]
    let colon = str.index_of(l, ":")
    let head = str.substring(l, 0, colon + 1)
    let rest = str.trim(str.substring(l, colon + 1, str.length(l)))
    let at = str.index_of(rest, "value:")
    let new_rest = if str.starts_with(rest, "{") && at != () => {
        let after = str.substring(rest, at + 6, str.length(rest))
        let comma = str.index_of(after, ",")
        let close = str.index_of(after, "}")
        let end = if comma != () && (close == () || comma < close) => comma else => close ?? str.length(after)
        str.substring(rest, 0, at + 6) + " " + yaml_value(v) + str.substring(after, end, str.length(after))
    } else => yaml_value(v)
    join(take(lines, i) + [head + " " + new_rest] + skip(lines, i + 1), "\n")
}

fn handle(k, st, req) = {
    let st = map_set(st, "fresh", [])
    if req.op == "page" => (st, page(k, st))
    else if req.op == "set" => {
        let r = ask(k, st.n + 1, #{ "op": "set", "cell": req.cell, "value": req.value })
        let st = map_set(st, "n", st.n + 1)
        if r.ok != true => (st, #{ "error": (r.error ?? #{}).message ?? "refused" })
        else => {
            let c = map_get(st.deps, req.cell) ?? #{ "defines": [] }
            let st = run_exprs(k, run_cells(k, st, reached(SPEC.cells, st.deps, c.defines, req.cell)))
            (st, answer(st))
        }
    } else if req.op == "param" => {
        let p = first(filter(st.params, (x) => x.name == req.name))
        if p == () => (st, #{ "error": "no param " + req.name }) else => {
            let text_ = with_param(st.text, p, req.value)
            let r = ask(k, st.n + 1, #{ "op": "reread", "text": text_ })
            let st = map_set(st, "n", st.n + 1)
            if r.ok != true || len(r.problems ?? []) > 0 => (st, #{ "error": first(map(r.problems ?? [], (x) => x.message)) ?? "refused" })
            else => {
                let params = map(st.params, (x) => if x.name == req.name => map_set(x, "value", yaml_value(req.value)) else => x)
                let st = st |> map_set("text", text_) |> map_set("params", params)
                let st = run_exprs(k, run_cells(k, st, reached(SPEC.cells, st.deps, r.params ?? [], ())))
                (st, answer(st))
            }
        }
    } else => (st, #{ "error": "unknown request" })
}

// The kernel's task: started, every cell run, then each request in turn.
fn kernel(inbox) = {
    let k = unwrap(proc.spawn([OLANG, "repl", "--serve", "--notebook", SPEC.notebook]))
    let _hello = proc.read_line(k)
    let d = ask(k, 1, #{ "op": "deps", "cells": map(SPEC.cells, (c) => #{ "cell": c.id, "code": c.code, "line": c.line }) })
    let deps = fold(d.cells ?? [], #{}, (acc, c) => map_set(acc, c.cell, #{ "defines": names_of(c.defines), "uses": names_of(c.uses) }))
    let st0 = #{ "n": 1, "replies": #{}, "interp": #{}, "fresh": [], "deps": deps, "params": SPEC.params,
                 "text": unwrap(fs.read_file(SPEC.notebook)) }
    let mut st = run_exprs(k, run_cells(k, st0, SPEC.cells))
    println(SPEC.title + ": every cell ran")
    let mut going = true
    while going {
        match chan.recv(inbox) {
            Ok(m) => {
                let (st2, out) = handle(k, st, m.req)
                st = st2
                let _s = chan.send(m.reply, out)
            },
            Err(e) => { going = false }
        }
    }
}

let INBOX = chan.new()
let _kernel = spawn { kernel(INBOX) }

fn call(req) = match chan.ask(INBOX, req) { Ok(v) => v, Err(e) => #{ "error": show(e) } }

fn body(req) = match json.parse(req.body) { Ok(v) => v, Err(e) => #{} }

let routes = [
    route("GET", "/", (req, p) => {
        let r = call(#{ "op": "page" })
        if r.ok == true => http.response_with_headers(200, r.html, #{ "Content-Type": "text/html; charset=utf-8" })
        else => http.response(500, (r.error ?? #{}).message ?? "the page could not be drawn")
    }),
    rpc("nb.set", (req, p) => { let b = body(req); call(#{ "op": "set", "cell": b.cell, "value": b.value }) }),
    rpc("nb.param", (req, p) => { let b = body(req); call(#{ "op": "param", "name": b.name, "value": b.value }) })
]

serve(#{ "title": SPEC.title, "routes": routes, "client": "", "port": PORT })
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_reply() -> J {
        json!({ "ok": true, "out": "", "value": { "k": "frame", "t": "Frame", "n": 391, "s": "Frame[391 x 2]",
            "table": { "cols": ["region", "amount"], "types": ["String", "Float"] },
            "rows": [ { "i": 0, "c": ["east", "1204.5"] }, { "i": 1, "c": ["west", null] } ] } })
    }

    #[test]
    fn floats_read_as_a_person_reads_them() {
        assert_eq!(human_float("0.2900157511888042"), "0.290016");
        assert_eq!(human_float("1204.5"), "1204.5");
        assert_eq!(human_float("12.0"), "12.0");
        assert_eq!(human_float("123456.789"), "123457");
    }

    #[test]
    fn a_pinned_table_is_aligned_text() {
        let (kind, body) = pin_of(&frame_reply(), None).unwrap();
        assert_eq!(kind, "table");
        assert_eq!(
            body,
            "region  amount\nString  Float\n──────  ──────\neast    1204.5\nwest    null\n391 rows × 2 columns · the first 2 shown"
        );
        let num = json!({ "ok": true, "out": "3 rows\n", "value": { "k": "float", "t": "Float", "s": "0.2900157511888042" } });
        assert_eq!(
            pin_of(&num, None).unwrap(),
            ("text".into(), "3 rows\n\n0.290016".into())
        );
        let unit =
            json!({ "ok": true, "out": "", "value": { "k": "unit", "t": "Unit", "s": "()" } });
        assert_eq!(pin_of(&unit, None), None);
        let err = json!({ "ok": false, "error": { "message": "boom" } });
        assert_eq!(pin_of(&err, None).unwrap().1, "error: boom");
    }

    #[test]
    fn a_page_holds_prose_code_and_outputs() {
        let text = "---\ntitle: Sales\nparams:\n  region: \"east\"\n---\n\n# Sales\n\nRevenue in **${region}**: ${total}.\n\n```olang {name: load}\nlet total = 3\nlet t = 1\n```\n\n```olang {name: peek, hide: code}\nsales\n```\n\n```olang {name: gone, hide: all}\n1\n```\n";
        let doc = json!({ "cells": [
            { "name": "load", "reply": { "ok": true, "out": "", "defines": ["total", "t"], "caps": ["fs"], "ms": 3.2, "value": { "k": "unit", "s": "()" } } },
            { "name": "peek", "reply": frame_reply() } ],
            "interp": [ { "expr": "region", "reply": { "ok": true, "value": { "k": "str", "s": "east" } } } ] });
        let html = page_html(text, Path::new("/x/sales.olmd"), &doc, Mode::Export);
        assert!(html.contains("<h1>Sales</h1>"));
        assert_eq!(html.matches("<h1>").count(), 1, "the title is said once");
        assert!(
            html.contains("<strong><span class=\"val\" data-expr=\"0\">east</span></strong>"),
            "{}",
            html
        );
        assert!(html.contains("<code class=\"expr\" data-expr=\"1\">${total}</code>"));
        assert!(html.contains("defines <code>total</code> <code>t</code>"));
        assert!(html.contains("<span class=\"badge\">fs</span>"));
        assert!(
            html.contains("<span class=\"k\">let</span>"),
            "code is highlighted"
        );
        assert_eq!(
            html.matches("<pre class=\"code\">").count(),
            1,
            "peek's code is hidden"
        );
        assert!(
            html.contains("<td class=\"num\">1204.5</td>")
                && html.contains("<td class=\"null\">null</td>")
        );
        assert!(html.contains("391 rows × 2 columns · the first 2 shown"));
        assert!(
            !html.contains("cell-gone"),
            "a cell hidden all is not on the page"
        );
        assert!(
            !html.contains("<script"),
            "an exported page with no brush runs nothing"
        );
        assert!(html.contains("prefers-color-scheme:dark"));
    }

    #[test]
    fn a_control_is_its_last_value_or_live() {
        let reply = json!({ "ok": true, "out": "", "value": { "k": "float", "s": "0.12" },
            "control": { "kind": "slider", "name": "lift", "value": { "k": "float", "s": "0.12" },
                "min": { "k": "float", "s": "0.0" }, "max": { "k": "float", "s": "0.3" }, "step": { "k": "float", "s": "0.01" } } });
        let read_only = output_html(&reply, None, Mode::Export, "c3", false);
        assert!(
            read_only.contains("type=\"range\"")
                && read_only.contains("value=\"0.12\"")
                && read_only.contains(" disabled"),
            "{}",
            read_only
        );
        let live = output_html(&reply, None, Mode::App, "c3", false);
        assert!(
            live.contains("data-cell=\"c3\" data-name=\"lift\"") && !live.contains("disabled"),
            "{}",
            live
        );
    }

    #[test]
    fn a_brush_is_dragged_on_a_page_and_set_in_an_app() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 720 440\"><metadata id=\"olang-chart\">{\"v\":1,\"type\":\"xy\",\"width\":720,\"height\":440,\"x\":{\"lo\":0.0,\"hi\":10.0},\"plot\":{\"left\":56.0,\"top\":20.0,\"w\":644.0,\"h\":380.0}}</metadata></svg>";
        let reply = json!({ "ok": true, "out": "", "value": { "k": "list", "s": "[2.0, 4.5]" },
            "control": { "kind": "brush", "name": "window", "value": { "k": "list", "s": "[2.0, 4.5]" }, "svg": { "text": svg } } });
        // the chart under a layer a range is dragged on, its range said
        let page = output_html(&reply, None, Mode::Export, "c4", false);
        assert!(
            page.contains("<div class=\"brush\"")
                && page.contains("data-x0=\"2\" data-x1=\"4.5\"")
                && page.contains("olang-chart")
                && !page.contains("data-cell"),
            "{}",
            page
        );
        // an app's: the cell and name a drag sets
        let live = output_html(&reply, None, Mode::App, "c4", false);
        assert!(
            live.contains("data-cell=\"c4\" data-name=\"window\""),
            "{}",
            live
        );
        // no range: nothing drawn yet
        let none = json!({ "ok": true, "out": "", "value": { "k": "unit", "s": "()" },
            "control": { "kind": "brush", "name": "window", "value": { "k": "unit", "s": "()" }, "svg": { "text": svg } } });
        let empty = output_html(&none, None, Mode::Export, "c4", false);
        assert!(
            !empty.contains("data-x0") && empty.contains("no range brushed"),
            "{}",
            empty
        );
        // the page runs its brush script (only for a page with a brush),
        // an app's its posts too
        let text = "```olang\nlet window = ui.brush(chart)\n```\n";
        let doc = json!({ "cells": [{ "line": 2, "reply": reply }] });
        let exported = page_html(text, Path::new("/x/b.olmd"), &doc, Mode::Export);
        assert!(
            exported.contains("window.olangBrush")
                && exported.contains("olangBrush(document, null)")
                && !exported.contains("/api/rpc/nb.set"),
            "{}",
            exported
        );
        let app = page_html(text, Path::new("/x/b.olmd"), &doc, Mode::App);
        assert!(
            app.contains("window.olangBrush(document, function (el, value)"),
            "{}",
            app
        );
        let plain = page_html(
            "```olang\n1\n```\n",
            Path::new("/x/p.olmd"),
            &json!({}),
            Mode::Export,
        );
        assert!(!plain.contains("<script"));
        assert_eq!(brush_range("[1.5, -2e3]"), Some((1.5, -2000.0)));
        assert_eq!(brush_range("()"), None);
    }

    #[test]
    fn a_page_draws_a_pinned_output_where_nothing_ran() {
        let text = "```olang {name: total, pin: true}\n3\n```\n\n```output {cell: total, kind: text}\n3\n```\n";
        let html = page_html(text, Path::new("/x/t.olmd"), &json!({}), Mode::Export);
        assert!(
            html.contains("<pre class=\"text pinned\">3</pre>"),
            "{}",
            html
        );
    }
}
