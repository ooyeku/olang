//! `md` — Markdown to HTML: CommonMark with GitHub's extensions (tables,
//! task lists, strikethrough, footnotes, bare-URL autolinks), headings
//! with GitHub's anchors, the outline, front matter, and hooks a caller
//! draws code blocks, links and headings with.
//!
//! The parser is pulldown-cmark. This module walks its events once:
//! a heading's text becomes its slug, a fenced block goes to the `code`
//! hook, every link target (links, images, autolinks, and `href`/`src`
//! inside raw HTML) to the `link` hook, and the result is written by
//! pulldown-cmark's own HTML writer. The hooks are a trait here; the
//! interpreter implements it over olang functions (src/builtin.rs), so
//! `md.render` with hooks runs them on whichever tier called it.

use crate::ast::Value;
use pulldown_cmark::{CowStr, Event, LinkType, Options as CmOptions, Parser, Tag, TagEnd};
use std::sync::Arc;

// ── options and hooks ────────────────────────────────────────────────

/// What raw HTML in the source becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlMode {
    Keep,
    Escape,
    Drop,
}

/// `md.render`'s options, the hooks aside.
#[derive(Clone, Debug)]
pub struct Options {
    pub shift: u8,
    pub html: HtmlMode,
    pub safe: bool,
    pub highlight: bool,
    pub anchors: bool,
    pub inline: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            shift: 0,
            html: HtmlMode::Keep,
            safe: false,
            highlight: false,
            anchors: true,
            inline: false,
        }
    }
}

/// What a `link` hook answers for a target.
pub enum LinkAnswer {
    Keep,
    To(String),
    Drop,
}

/// The caller's drawing: each answers `None` (or `Keep`) to leave the
/// default in place.
pub trait Hooks {
    fn has_link(&self) -> bool {
        false
    }
    fn code(&mut self, _lang: &str, _info: &str, _text: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn link(&mut self, _url: &str) -> Result<LinkAnswer, String> {
        Ok(LinkAnswer::Keep)
    }
    fn heading(&mut self, _level: u8, _slug: &str, _html: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
}

/// No hooks: the defaults throughout.
pub struct NoHooks;
impl Hooks for NoHooks {}

/// One heading of the outline.
#[derive(Clone, Debug, PartialEq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub slug: String,
    pub html: String,
}

/// A rendered document.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    pub html: String,
    pub headings: Vec<Heading>,
    pub title: String,
}

fn cm_options() -> CmOptions {
    CmOptions::ENABLE_TABLES
        | CmOptions::ENABLE_FOOTNOTES
        | CmOptions::ENABLE_STRIKETHROUGH
        | CmOptions::ENABLE_TASKLISTS
        | CmOptions::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | CmOptions::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
}

// ── slugs ────────────────────────────────────────────────────────────

/// The anchor GitHub gives a heading whose text is `text`: lower-cased,
/// letters, digits, `-` and `_` kept, each space a `-`, everything else
/// dropped.
pub fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.trim().chars() {
        match c {
            ' ' => out.push('-'),
            '-' | '_' => out.push(c),
            c if c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => {}
        }
    }
    out
}

/// Slugs unique on a page, numbered as GitHub numbers a repeat.
#[derive(Default)]
struct Slugs {
    counts: std::collections::HashMap<String, usize>,
    used: std::collections::HashSet<String>,
}

impl Slugs {
    fn unique(&mut self, base: String) -> String {
        loop {
            let n = self.counts.entry(base.clone()).or_insert(0);
            let candidate = if *n == 0 { base.clone() } else { format!("{base}-{n}") };
            *n += 1;
            // a numbered slug may already be a heading's own ("a-1")
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
        }
    }
}

// ── escaping and URLs ────────────────────────────────────────────────

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// The scheme of a URL (`https`, `javascript`), lower-cased; `None` for
/// a relative target.
fn scheme(url: &str) -> Option<String> {
    let t = url.trim_start();
    let colon = t.find(':')?;
    let head = &t[..colon];
    if head.is_empty()
        || t[..colon].contains(['/', '?', '#'])
        || !head
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(head.to_ascii_lowercase())
}

/// Whether safe mode lets a target through.
fn safe_url(url: &str, image: bool) -> bool {
    match scheme(url).as_deref() {
        None | Some("http") | Some("https") | Some("mailto") => true,
        Some("data") if image => {
            let lower = url.trim_start().to_ascii_lowercase();
            ["data:image/png", "data:image/gif", "data:image/jpeg", "data:image/webp"]
                .iter()
                .any(|p| lower.starts_with(p))
        }
        _ => false,
    }
}

// ── the renderer ─────────────────────────────────────────────────────

struct Walk<'h, H: Hooks + ?Sized> {
    opts: Options,
    hooks: &'h mut H,
    slugs: Slugs,
    headings: Vec<Heading>,
    title: Option<String>,
    footnotes: Vec<Event<'static>>,
}

fn html_event(s: String) -> Event<'static> {
    Event::Html(CowStr::Boxed(s.into_boxed_str()))
}

fn text_event(s: String) -> Event<'static> {
    Event::Text(CowStr::Boxed(s.into_boxed_str()))
}

/// Adjacent text events merged, so an autolink split by the parser is
/// seen whole.
fn merge_text(events: Vec<Event<'_>>) -> Vec<Event<'static>> {
    let mut out: Vec<Event<'static>> = Vec::with_capacity(events.len());
    for e in events {
        if let Event::Text(t) = &e
            && let Some(Event::Text(prev)) = out.last_mut()
        {
            let mut s = prev.to_string();
            s.push_str(t);
            *prev = CowStr::Boxed(s.into_boxed_str());
            continue;
        }
        out.push(e.into_static());
    }
    out
}

/// The plain text of events: text and code, a break a space.
fn plain(events: &[Event<'_>]) -> String {
    let mut s = String::new();
    for e in events {
        match e {
            Event::Text(t) | Event::Code(t) => s.push_str(t),
            Event::SoftBreak | Event::HardBreak => s.push(' '),
            _ => {}
        }
    }
    s
}

fn push_html(events: Vec<Event<'static>>) -> String {
    let mut out = String::new();
    pulldown_cmark::html::push_html(&mut out, events.into_iter());
    out
}

impl<H: Hooks + ?Sized> Walk<'_, H> {
    /// A target through safe mode and the hook: `None` drops it.
    fn target(&mut self, url: &str, image: bool) -> Result<Option<String>, String> {
        if self.opts.safe && !safe_url(url, image) {
            return Ok(None);
        }
        Ok(match self.hooks.link(url)? {
            LinkAnswer::Keep => Some(url.to_string()),
            LinkAnswer::To(u) => Some(u),
            LinkAnswer::Drop => None,
        })
    }

    /// The events of a whole document (or a heading's inside), drawn.
    fn walk(&mut self, events: Vec<Event<'static>>) -> Result<Vec<Event<'static>>, String> {
        let mut out = Vec::with_capacity(events.len());
        let mut it = events.into_iter().peekable();
        // per open link: whether it was dropped
        let mut links: Vec<bool> = Vec::new();
        // inside a link (no autolinks there)
        let mut in_link = 0usize;
        while let Some(e) = it.next() {
            match e {
                Event::Start(Tag::MetadataBlock(_)) => {
                    for e in it.by_ref() {
                        if matches!(e, Event::End(TagEnd::MetadataBlock(_))) {
                            break;
                        }
                    }
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    let mut inner = Vec::new();
                    for e in it.by_ref() {
                        if matches!(e, Event::End(TagEnd::Heading(_))) {
                            break;
                        }
                        inner.push(e);
                    }
                    let inner = self.walk(inner)?;
                    let text = plain(&inner);
                    let level0 = level as u8;
                    if level0 == 1 && self.title.is_none() {
                        self.title = Some(text.clone());
                    }
                    let level = (level0 + self.opts.shift).min(6);
                    let inner_html = push_html(inner);
                    let slug = self.slugs.unique(slug(&text));
                    let drawn = match self.hooks.heading(level, &slug, &inner_html)? {
                        Some(h) => h,
                        None if self.opts.anchors => {
                            format!("<h{level} id=\"{}\">{inner_html}</h{level}>\n", escape_html(&slug))
                        }
                        None => format!("<h{level}>{inner_html}</h{level}>\n"),
                    };
                    self.headings.push(Heading {
                        level,
                        text,
                        slug,
                        html: inner_html,
                    });
                    out.push(html_event(drawn));
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    let mut text = String::new();
                    for e in it.by_ref() {
                        match e {
                            Event::End(TagEnd::CodeBlock) => break,
                            Event::Text(t) => text.push_str(&t),
                            _ => {}
                        }
                    }
                    let info = match &kind {
                        pulldown_cmark::CodeBlockKind::Fenced(i) => i.to_string(),
                        pulldown_cmark::CodeBlockKind::Indented => String::new(),
                    };
                    let lang = info
                        .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    // the block's text without its final line break
                    let body = text.strip_suffix('\n').unwrap_or(&text).to_string();
                    let drawn = match self.hooks.code(&lang, &info, &body)? {
                        Some(h) => h,
                        None => {
                            let class = if lang.is_empty() {
                                String::new()
                            } else {
                                format!(" class=\"language-{}\"", escape_html(&lang))
                            };
                            let inner = if self.opts.highlight && matches!(lang.as_str(), "olang" | "ol") {
                                crate::stdlib::tokens::highlight(&text)
                            } else {
                                escape_html(&text)
                            };
                            format!("<pre><code{class}>{inner}</code></pre>\n")
                        }
                    };
                    out.push(html_event(drawn));
                }
                Event::Start(Tag::FootnoteDefinition(label)) => {
                    let mut inner = Vec::new();
                    let mut depth = 0usize;
                    for e in it.by_ref() {
                        match &e {
                            Event::Start(Tag::FootnoteDefinition(_)) => depth += 1,
                            Event::End(TagEnd::FootnoteDefinition) if depth == 0 => break,
                            Event::End(TagEnd::FootnoteDefinition) => depth -= 1,
                            _ => {}
                        }
                        inner.push(e);
                    }
                    let inner = self.walk(inner)?;
                    self.footnotes.push(Event::Start(Tag::FootnoteDefinition(label)));
                    self.footnotes.extend(inner);
                    self.footnotes.push(Event::End(TagEnd::FootnoteDefinition));
                }
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    title,
                    id,
                }) => {
                    // an e-mail autolink's target is the address alone
                    let shown = if link_type == LinkType::Email {
                        format!("mailto:{dest_url}")
                    } else {
                        dest_url.to_string()
                    };
                    match self.target(&shown, false)? {
                        Some(u) => {
                            links.push(false);
                            in_link += 1;
                            let link_type = if link_type == LinkType::Email {
                                LinkType::Inline
                            } else {
                                link_type
                            };
                            out.push(Event::Start(Tag::Link {
                                link_type,
                                dest_url: CowStr::Boxed(u.into_boxed_str()),
                                title,
                                id,
                            }));
                        }
                        None => {
                            links.push(true);
                            in_link += 1;
                        }
                    }
                }
                Event::End(TagEnd::Link) => {
                    in_link = in_link.saturating_sub(1);
                    if !links.pop().unwrap_or(false) {
                        out.push(Event::End(TagEnd::Link));
                    }
                }
                Event::Start(Tag::Image {
                    link_type,
                    dest_url,
                    title,
                    id,
                }) => match self.target(&dest_url, true)? {
                    Some(u) => out.push(Event::Start(Tag::Image {
                        link_type,
                        dest_url: CowStr::Boxed(u.into_boxed_str()),
                        title,
                        id,
                    })),
                    None => {
                        // the image dropped, its alt text kept
                        let mut depth = 0usize;
                        for e in it.by_ref() {
                            match e {
                                Event::Start(Tag::Image { .. }) => depth += 1,
                                Event::End(TagEnd::Image) if depth == 0 => break,
                                Event::End(TagEnd::Image) => depth -= 1,
                                Event::Text(t) | Event::Code(t) => out.push(Event::Text(t)),
                                _ => {}
                            }
                        }
                    }
                },
                Event::Start(Tag::HtmlBlock) | Event::End(TagEnd::HtmlBlock) => {
                    if self.html_mode() == HtmlMode::Escape {
                        let start = matches!(e, Event::Start(_));
                        out.push(html_event(if start { "<p>".into() } else { "</p>\n".into() }));
                    } else {
                        out.push(e);
                    }
                }
                Event::Html(h) | Event::InlineHtml(h) => match self.html_mode() {
                    HtmlMode::Drop => {}
                    HtmlMode::Escape => out.push(text_event(h.to_string())),
                    HtmlMode::Keep => {
                        let rewritten = if self.hooks.has_link() || self.opts.safe {
                            self.rewrite_html(&h)?
                        } else {
                            h.to_string()
                        };
                        out.push(html_event(rewritten));
                    }
                },
                Event::Text(t) if in_link == 0 => self.autolink(&t, &mut out)?,
                other => out.push(other),
            }
        }
        Ok(out)
    }

    fn html_mode(&self) -> HtmlMode {
        if self.opts.safe && self.opts.html == HtmlMode::Keep {
            HtmlMode::Escape
        } else {
            self.opts.html
        }
    }

    /// Raw HTML with every `href` and `src` value sent through the hook.
    fn rewrite_html(&mut self, html: &str) -> Result<String, String> {
        let mut out = String::with_capacity(html.len());
        let lower = html.to_ascii_lowercase();
        let mut i = 0usize;
        while i < html.len() {
            let found = ["href=", "src="]
                .iter()
                .filter_map(|k| lower[i..].find(k).map(|p| (i + p, k.len())))
                .min();
            let Some((at, klen)) = found else {
                out.push_str(&html[i..]);
                break;
            };
            // an attribute name stands after whitespace
            let before_ok = at > 0 && html.as_bytes()[at - 1].is_ascii_whitespace();
            let vstart = at + klen;
            out.push_str(&html[i..vstart]);
            if !before_ok || vstart >= html.len() {
                i = vstart;
                continue;
            }
            let q = html.as_bytes()[vstart];
            let (inner_start, end) = if q == b'"' || q == b'\'' {
                let close = html[vstart + 1..]
                    .find(q as char)
                    .map(|p| vstart + 1 + p)
                    .unwrap_or(html.len());
                (vstart + 1, close)
            } else {
                let close = html[vstart..]
                    .find(|c: char| c.is_whitespace() || c == '>')
                    .map(|p| vstart + p)
                    .unwrap_or(html.len());
                (vstart, close)
            };
            let url = &html[inner_start..end];
            let image = klen == 4;
            let new = self.target(url, image)?.unwrap_or_else(|| "#".to_string());
            out.push_str(&html[vstart..inner_start]);
            out.push_str(&new.replace('"', "&quot;"));
            i = end;
        }
        Ok(out)
    }

    /// Text with GitHub's bare-URL autolinks made links.
    fn autolink(&mut self, text: &str, out: &mut Vec<Event<'static>>) -> Result<(), String> {
        let mut rest = text;
        let mut plain_run = String::new();
        loop {
            let Some((start, end)) = find_autolink(rest) else {
                plain_run.push_str(rest);
                break;
            };
            plain_run.push_str(&rest[..start]);
            let shown = &rest[start..end];
            let href = if shown.starts_with("www.") {
                format!("http://{shown}")
            } else {
                shown.to_string()
            };
            match self.target(&href, false)? {
                Some(u) => {
                    if !plain_run.is_empty() {
                        out.push(text_event(std::mem::take(&mut plain_run)));
                    }
                    out.push(html_event(format!(
                        "<a href=\"{}\">{}</a>",
                        escape_html(&u),
                        escape_html(shown)
                    )));
                }
                None => plain_run.push_str(shown),
            }
            rest = &rest[end..];
        }
        if !plain_run.is_empty() {
            out.push(text_event(plain_run));
        }
        Ok(())
    }
}

/// The first bare URL in `s` (`https://…`, `http://…`, `www.…`) as GFM
/// finds it: at the start or after a space or `(*_~`, running to the
/// next space or `<`, without its trailing punctuation or an unbalanced
/// `)`.
fn find_autolink(s: &str) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut from = 0usize;
    while from < s.len() {
        let cands = ["https://", "http://", "www."];
        let (at, prefix) = cands
            .iter()
            .filter_map(|p| s[from..].find(p).map(|i| (from + i, *p)))
            .min_by_key(|(i, _)| *i)?;
        let boundary = at == 0
            || matches!(bytes[at - 1], b' ' | b'\t' | b'\n' | b'(' | b'*' | b'_' | b'~' | b'"' | b'\'');
        let mut end = s[at..]
            .find(|c: char| c.is_whitespace() || c == '<')
            .map(|i| at + i)
            .unwrap_or(s.len());
        // trailing punctuation is no part of the link
        loop {
            let t = &s[at..end];
            let Some(last) = t.chars().last() else { break };
            if matches!(last, '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~' | '"' | '\'' | ';') {
                end -= last.len_utf8();
            } else if last == ')' && t.matches(')').count() > t.matches('(').count() {
                end -= 1;
            } else {
                break;
            }
        }
        let host = &s[at + prefix.len()..end];
        let valid = !host.is_empty()
            && host.chars().next().is_some_and(|c| c.is_alphanumeric())
            && (prefix != "www." || host.contains('.') || host.len() > 1);
        if boundary && valid {
            return Some((at, end));
        }
        from = at + prefix.len();
    }
    None
}

/// Render `src` as HTML.
pub fn render(src: &str, opts: Options, hooks: &mut dyn Hooks) -> Result<Rendered, String> {
    let inline = opts.inline;
    let events: Vec<Event> = Parser::new_ext(src, cm_options()).collect();
    let mut walk = Walk {
        opts,
        hooks,
        slugs: Slugs::default(),
        headings: Vec::new(),
        title: None,
        footnotes: Vec::new(),
    };
    let mut drawn = walk.walk(merge_text(events))?;
    if !walk.footnotes.is_empty() {
        drawn.push(html_event("<section class=\"footnotes\">\n".into()));
        drawn.append(&mut walk.footnotes);
        drawn.push(html_event("</section>\n".into()));
    }
    let mut html = push_html(drawn);
    if inline {
        // one paragraph: its inside, for a title or a summary in a line
        if let Some(inner) = html
            .strip_prefix("<p>")
            .and_then(|r| r.strip_suffix("</p>\n"))
            .filter(|inner| !inner.contains("<p>"))
        {
            html = inner.to_string();
        }
    }
    Ok(Rendered {
        html,
        headings: walk.headings,
        title: walk.title.unwrap_or_default(),
    })
}

/// The document's headings, without drawing the rest.
pub fn headings(src: &str) -> Vec<Heading> {
    match render(src, Options::default(), &mut NoHooks) {
        Ok(r) => r.headings,
        Err(_) => Vec::new(),
    }
}

/// The document's text: no markup, a block a line.
pub fn text(src: &str) -> String {
    let mut out = String::new();
    let mut skip = 0usize;
    for e in Parser::new_ext(src, cm_options()) {
        match e {
            Event::Start(Tag::MetadataBlock(_)) => skip += 1,
            Event::End(TagEnd::MetadataBlock(_)) => skip = skip.saturating_sub(1),
            _ if skip > 0 => {}
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::CodeBlock
                | TagEnd::Item
                | TagEnd::TableRow
                | TagEnd::TableHead,
            ) => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::TableCell) => out.push(' '),
            _ => {}
        }
    }
    out.trim_end().to_string()
}

// ── front matter ─────────────────────────────────────────────────────

/// Front matter split off: its data (a Map), the body after it, and its
/// format (`yaml`, `toml`, or `""` when there is none).
pub fn front(src: &str) -> Result<(Value, String, &'static str), String> {
    let src_nl = src.strip_prefix('\u{feff}').unwrap_or(src);
    for (fence, format) in [("---", "yaml"), ("+++", "toml")] {
        let Some(after) = src_nl
            .strip_prefix(fence)
            .and_then(|r| r.strip_prefix("\r\n").or_else(|| r.strip_prefix('\n')))
        else {
            continue;
        };
        // the closing fence: a line of the same marks (YAML also `...`)
        let mut at = 0usize;
        let mut close: Option<(usize, usize)> = None;
        for line in after.split_inclusive('\n') {
            let t = line.trim_end_matches(['\n', '\r']);
            if t == fence || (format == "yaml" && t == "...") {
                close = Some((at, at + line.len()));
                break;
            }
            at += line.len();
        }
        let Some((body_end, rest)) = close else {
            return Err(format!("front matter opened with {fence} is never closed"));
        };
        let block = &after[..body_end];
        let data = if format == "yaml" {
            yaml::parse(block)?
        } else {
            let tv = block
                .parse::<toml::Value>()
                .map_err(|e| format!("front matter (TOML): {e}"))?;
            crate::stdlib::json::json_to_olang_value(crate::stdlib::toml_mod::toml_to_json(tv))?
        };
        return Ok((data, after[rest..].to_string(), format));
    }
    Ok((
        Value::Map(Arc::new(crate::ast::ValueMap::default())),
        src.to_string(),
        "",
    ))
}

/// The YAML front matter needs: block mappings and sequences by
/// indentation, flow lists and maps of scalars, quoted and plain
/// scalars (strings, numbers, Bools, null; a date stays text), block
/// scalars (`|`, `>`), and comments.
pub mod yaml {
    use crate::ast::Value;
    use std::sync::Arc;

    #[derive(Clone)]
    struct Line {
        indent: usize,
        text: String,
        no: usize,
    }

    fn s(v: &str) -> Value {
        Value::String(Arc::new(v.to_string()))
    }

    pub fn parse(src: &str) -> Result<Value, String> {
        let lines: Vec<Line> = src
            .lines()
            .enumerate()
            .map(|(i, l)| {
                let indent = l.len() - l.trim_start_matches(' ').len();
                Line {
                    indent,
                    text: l[indent..].trim_end().to_string(),
                    no: i + 1,
                }
            })
            .collect();
        if lines.iter().all(|l| is_blank(&l.text)) {
            return Ok(Value::Map(Arc::new(crate::ast::ValueMap::default())));
        }
        if let Some(l) = lines.iter().find(|l| l.text.starts_with('\t')) {
            return Err(format!("front matter line {}: a tab indents nothing in YAML", l.no));
        }
        let (v, next) = block(&lines, 0, first_indent(&lines, 0))?;
        if let Some(l) = lines[next..].iter().find(|l| !is_blank(&l.text)) {
            return Err(format!("front matter line {}: unexpected indentation", l.no));
        }
        Ok(v)
    }

    fn is_blank(t: &str) -> bool {
        t.is_empty() || t.starts_with('#')
    }

    fn first_indent(lines: &[Line], from: usize) -> usize {
        lines[from..]
            .iter()
            .find(|l| !is_blank(&l.text))
            .map(|l| l.indent)
            .unwrap_or(0)
    }

    fn is_item(t: &str) -> bool {
        t == "-" || t.starts_with("- ")
    }

    /// The block at `indent` from line `i`: a sequence or a mapping.
    /// Answers the value and the line after it.
    fn block(lines: &[Line], mut i: usize, indent: usize) -> Result<(Value, usize), String> {
        while i < lines.len() && is_blank(&lines[i].text) {
            i += 1;
        }
        if i >= lines.len() {
            return Ok((Value::Unit, i));
        }
        if is_item(&lines[i].text) {
            let mut items = Vec::new();
            while i < lines.len() {
                let l = &lines[i];
                if is_blank(&l.text) {
                    i += 1;
                    continue;
                }
                if l.indent != indent || !is_item(&l.text) {
                    break;
                }
                let rest = l.text[1..].trim_start().to_string();
                // the item's own lines: its first, then those indented past the dash
                let mut sub = Vec::new();
                let inner = indent + (l.text.len() - rest.len());
                if !rest.is_empty() {
                    sub.push(Line {
                        indent: inner,
                        text: rest.clone(),
                        no: l.no,
                    });
                }
                let mut j = i + 1;
                while j < lines.len() && (is_blank(&lines[j].text) || lines[j].indent > indent) {
                    sub.push(lines[j].clone());
                    j += 1;
                }
                let value = if rest.is_empty() {
                    if sub.iter().all(|l| is_blank(&l.text)) {
                        Value::Unit
                    } else {
                        block(&sub, 0, first_indent(&sub, 0))?.0
                    }
                } else if sub.len() == 1 && !looks_like_key(&rest) {
                    scalar(&rest, l.no)?
                } else if looks_like_key(&rest) {
                    block(&sub, 0, inner)?.0
                } else {
                    // a scalar item continued on the next lines
                    let joined: Vec<&str> = sub.iter().map(|l| l.text.as_str()).collect();
                    scalar(&joined.join(" "), l.no)?
                };
                items.push(value);
                i = j;
            }
            return Ok((Value::List(Arc::new(items)), i));
        }
        let mut map = crate::ast::ValueMap::default();
        while i < lines.len() {
            let l = &lines[i];
            if is_blank(&l.text) {
                i += 1;
                continue;
            }
            if l.indent < indent {
                break;
            }
            if l.indent > indent {
                return Err(format!("front matter line {}: unexpected indentation", l.no));
            }
            if is_item(&l.text) {
                break;
            }
            let Some((key, rest)) = split_key(&l.text) else {
                return Err(format!(
                    "front matter line {}: expected `key: value`, got `{}`",
                    l.no, l.text
                ));
            };
            let key = unquote_key(&key);
            let rest = strip_comment(rest.trim());
            i += 1;
            let value = if rest.is_empty() {
                let next = first_line(lines, i);
                match next {
                    Some(n) if lines[n].indent > indent => {
                        let (v, after) = block(lines, n, lines[n].indent)?;
                        i = after;
                        v
                    }
                    Some(n) if lines[n].indent == indent && is_item(&lines[n].text) => {
                        let (v, after) = block(lines, n, indent)?;
                        i = after;
                        v
                    }
                    _ => Value::Unit,
                }
            } else if let Some(style) = block_scalar_style(rest) {
                let mut body = Vec::new();
                while i < lines.len() && (lines[i].text.is_empty() || lines[i].indent > indent) {
                    body.push(&lines[i]);
                    i += 1;
                }
                s(&block_scalar(&body, style))
            } else {
                // a plain scalar may continue on more-indented lines
                let mut text = rest.to_string();
                while i < lines.len()
                    && !lines[i].text.is_empty()
                    && lines[i].indent > indent
                    && !text.starts_with(['"', '\'', '[', '{'])
                {
                    text.push(' ');
                    text.push_str(strip_comment(&lines[i].text));
                    i += 1;
                }
                scalar(&text, l.no)?
            };
            map.insert(key, value);
        }
        Ok((Value::Map(Arc::new(map)), i))
    }

    fn first_line(lines: &[Line], from: usize) -> Option<usize> {
        (from..lines.len()).find(|&k| !is_blank(&lines[k].text))
    }

    fn looks_like_key(t: &str) -> bool {
        !t.starts_with(['"', '\'', '[', '{']) && split_key(t).is_some()
    }

    /// `key: rest` (or a quoted key); the colon must end the text or be
    /// followed by a space.
    fn split_key(t: &str) -> Option<(String, &str)> {
        if t.starts_with('"') || t.starts_with('\'') {
            let q = t.chars().next()?;
            let close = t[1..].find(q)? + 1;
            let after = t[close + 1..].trim_start();
            let rest = after.strip_prefix(':')?;
            if !(rest.is_empty() || rest.starts_with(' ')) {
                return None;
            }
            return Some((t[..=close].to_string(), rest));
        }
        let mut search = 0;
        while let Some(p) = t[search..].find(':') {
            let at = search + p;
            let rest = &t[at + 1..];
            if rest.is_empty() || rest.starts_with(' ') {
                let key = t[..at].trim_end();
                if key.is_empty() || key.starts_with('#') {
                    return None;
                }
                return Some((key.to_string(), rest));
            }
            search = at + 1;
        }
        None
    }

    fn unquote_key(k: &str) -> String {
        if (k.starts_with('"') && k.ends_with('"') || k.starts_with('\'') && k.ends_with('\''))
            && k.len() >= 2
        {
            k[1..k.len() - 1].to_string()
        } else {
            k.to_string()
        }
    }

    /// A trailing ` # comment` off a plain value (not inside quotes).
    fn strip_comment(t: &str) -> &str {
        let mut quote: Option<char> = None;
        let mut prev = ' ';
        for (i, c) in t.char_indices() {
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if c == '"' || c == '\'' => {
                    if i == 0 || prev == ' ' || prev == '[' || prev == ',' {
                        quote = Some(c)
                    }
                }
                None if c == '#' && prev.is_whitespace() => return t[..i].trim_end(),
                None => {}
            }
            prev = c;
        }
        t
    }

    fn block_scalar_style(t: &str) -> Option<(char, char)> {
        let mut cs = t.chars();
        let style = cs.next()?;
        if style != '|' && style != '>' {
            return None;
        }
        let chomp = cs.next().unwrap_or(' ');
        if !matches!(chomp, ' ' | '-' | '+') || cs.next().is_some() {
            return None;
        }
        Some((style, chomp))
    }

    fn block_scalar(body: &[&Line], (style, chomp): (char, char)) -> String {
        let base = body
            .iter()
            .filter(|l| !l.text.is_empty())
            .map(|l| l.indent)
            .min()
            .unwrap_or(0);
        let rows: Vec<String> = body
            .iter()
            .map(|l| {
                if l.text.is_empty() {
                    String::new()
                } else {
                    format!("{}{}", " ".repeat(l.indent - base), l.text)
                }
            })
            .collect();
        let mut text = if style == '|' {
            rows.join("\n")
        } else {
            // folded: lines of a paragraph joined by spaces, blank lines kept
            let mut out = String::new();
            for (k, r) in rows.iter().enumerate() {
                if k > 0 {
                    if r.is_empty() || rows[k - 1].is_empty() {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                }
                out.push_str(r);
            }
            out
        };
        let trimmed = text.trim_end_matches('\n').len();
        match chomp {
            '-' => text.truncate(trimmed),
            '+' => text.push('\n'),
            _ => {
                text.truncate(trimmed);
                text.push('\n');
            }
        }
        text
    }

    /// One scalar, or a flow list or map.
    fn scalar(t: &str, no: usize) -> Result<Value, String> {
        let t = strip_comment(t.trim());
        if t.starts_with('[') {
            let inner = t
                .strip_prefix('[')
                .and_then(|r| r.strip_suffix(']'))
                .ok_or_else(|| format!("front matter line {no}: a `[` list is never closed"))?;
            let items = split_flow(inner)
                .into_iter()
                .filter(|p| !p.trim().is_empty())
                .map(|p| scalar(&p, no))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Value::List(Arc::new(items)));
        }
        if t.starts_with('{') {
            let inner = t
                .strip_prefix('{')
                .and_then(|r| r.strip_suffix('}'))
                .ok_or_else(|| format!("front matter line {no}: a `{{` map is never closed"))?;
            let mut map = crate::ast::ValueMap::default();
            for part in split_flow(inner) {
                if part.trim().is_empty() {
                    continue;
                }
                let (k, v) = split_key(part.trim())
                    .ok_or_else(|| format!("front matter line {no}: expected `key: value` in `{{…}}`"))?;
                map.insert(unquote_key(&k), scalar(v, no)?);
            }
            return Ok(Value::Map(Arc::new(map)));
        }
        if let Some(r) = t.strip_prefix('"') {
            let body = r
                .strip_suffix('"')
                .ok_or_else(|| format!("front matter line {no}: a string is never closed"))?;
            let mut out = String::new();
            let mut cs = body.chars();
            while let Some(c) = cs.next() {
                if c == '\\' {
                    match cs.next() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some('r') => out.push('\r'),
                        Some('0') => out.push('\0'),
                        Some(o) => out.push(o),
                        None => {}
                    }
                } else {
                    out.push(c);
                }
            }
            return Ok(s(&out));
        }
        if let Some(r) = t.strip_prefix('\'') {
            let body = r
                .strip_suffix('\'')
                .ok_or_else(|| format!("front matter line {no}: a string is never closed"))?;
            return Ok(s(&body.replace("''", "'")));
        }
        Ok(match t {
            "" | "~" | "null" | "Null" | "NULL" => Value::Unit,
            "true" | "True" | "TRUE" => Value::Boolean(true),
            "false" | "False" | "FALSE" => Value::Boolean(false),
            _ => {
                let digits = t.trim_start_matches(['-', '+']);
                if !digits.is_empty()
                    && digits.chars().all(|c| c.is_ascii_digit() || c == '_')
                    && let Ok(n) = t.replace('_', "").parse::<i64>()
                {
                    Value::Integer(n)
                } else if t.chars().any(|c| c.is_ascii_digit())
                    && t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '-' | '+'))
                    && t.contains(['.', 'e', 'E'])
                    && let Ok(f) = t.parse::<f64>()
                {
                    Value::Float(f)
                } else {
                    s(t)
                }
            }
        })
    }

    /// A flow collection's items, split at top-level commas.
    fn split_flow(t: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut cur = String::new();
        let mut depth = 0i32;
        let mut quote: Option<char> = None;
        for c in t.chars() {
            match quote {
                Some(q) => {
                    if c == q {
                        quote = None;
                    }
                    cur.push(c);
                }
                None => match c {
                    '"' | '\'' => {
                        quote = Some(c);
                        cur.push(c);
                    }
                    '[' | '{' => {
                        depth += 1;
                        cur.push(c);
                    }
                    ']' | '}' => {
                        depth -= 1;
                        cur.push(c);
                    }
                    ',' if depth == 0 => parts.push(std::mem::take(&mut cur)),
                    _ => cur.push(c),
                },
            }
        }
        parts.push(cur);
        parts
    }
}

// ── the module ───────────────────────────────────────────────────────

pub fn create_md_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in [
        ("render", 1),
        ("headings", 1),
        ("slug", 1),
        ("front", 1),
        ("text", 1),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("md.{name}"),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

fn str_value(s: &str) -> Value {
    Value::String(Arc::new(s.to_string()))
}

fn text_arg<'a>(name: &str, args: &'a [Value], arity: &str) -> Result<&'a str, String> {
    match args.first() {
        Some(Value::String(s)) => Ok(s.as_str()),
        Some(other) => Err(format!(
            "md.{name}: the text must be a string, got {}",
            other.type_name()
        )),
        None => Err(format!("md.{name} expects {arity} argument(s), got 0")),
    }
}

/// `md.render`'s options read from its second argument: the plain ones
/// into `Options`, the hooks (functions) answered separately by key.
pub fn read_options(v: Option<&Value>) -> Result<(Options, crate::ast::ValueMap), String> {
    let mut opts = Options::default();
    let mut hooks = crate::ast::ValueMap::default();
    let fields = match v {
        None | Some(Value::Unit) => return Ok((opts, hooks)),
        Some(Value::Map(m)) => m.as_ref().clone(),
        Some(Value::Struct { fields, .. }) => fields.as_ref().clone(),
        Some(other) => {
            return Err(format!(
                "md.render: options must be a map or a record, got {}",
                other.type_name()
            ));
        }
    };
    for (k, v) in fields {
        match (k.as_str(), &v) {
            (_, Value::Unit) => {}
            ("code" | "link" | "heading", Value::Function(_) | Value::Builtin(_)) => {
                hooks.insert(k, v);
            }
            ("code" | "link" | "heading", other) => {
                return Err(format!(
                    "md.render: the `{k}` hook must be a function, got {}",
                    other.type_name()
                ));
            }
            ("shift", Value::Integer(n)) if (0..=5).contains(n) => opts.shift = *n as u8,
            ("shift", _) => return Err("md.render: `shift` must be an Int 0..=5".into()),
            ("html", Value::String(s)) => {
                opts.html = match s.as_str() {
                    "keep" => HtmlMode::Keep,
                    "escape" => HtmlMode::Escape,
                    "drop" => HtmlMode::Drop,
                    other => {
                        return Err(format!(
                            "md.render: `html` is \"keep\", \"escape\" or \"drop\", got \"{other}\""
                        ));
                    }
                }
            }
            ("safe", Value::Boolean(b)) => opts.safe = *b,
            ("highlight", Value::Boolean(b)) => opts.highlight = *b,
            ("anchors", Value::Boolean(b)) => opts.anchors = *b,
            ("inline", Value::Boolean(b)) => opts.inline = *b,
            ("html" | "safe" | "highlight" | "anchors" | "inline", other) => {
                return Err(format!(
                    "md.render: `{k}` must be {}, got {}",
                    if k == "html" { "a string" } else { "a Bool" },
                    other.type_name()
                ));
            }
            (other, _) => {
                return Err(format!(
                    "md.render: unknown option `{other}` (code, link, heading, shift, html, safe, highlight, anchors, inline)"
                ));
            }
        }
    }
    Ok((opts, hooks))
}

fn heading_value(h: &Heading) -> Value {
    let mut m = crate::ast::ValueMap::default();
    m.insert("level".into(), Value::Integer(h.level as i64));
    m.insert("text".into(), str_value(&h.text));
    m.insert("slug".into(), str_value(&h.slug));
    m.insert("html".into(), str_value(&h.html));
    Value::Map(Arc::new(m))
}

pub fn rendered_value(r: Rendered) -> Value {
    let mut m = crate::ast::ValueMap::default();
    m.insert("html".into(), Value::String(Arc::new(r.html)));
    m.insert(
        "headings".into(),
        Value::List(Arc::new(r.headings.iter().map(heading_value).collect())),
    );
    m.insert("title".into(), Value::String(Arc::new(r.title)));
    Value::Map(Arc::new(m))
}

/// The arity check every function shares.
pub fn check_arity(name: &str, args: &[Value]) -> Result<(), String> {
    let ok = match name {
        "render" => (1..=2).contains(&args.len()),
        _ => args.len() == 1,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "md.{name} expects {} argument(s), got {}",
            if name == "render" { "1 or 2" } else { "1" },
            args.len()
        ))
    }
}

/// The module's functions without hooks (`md.render` with hooks is the
/// interpreter's: src/builtin.rs).
pub fn call_md_function(name: &str, args: Vec<Value>) -> Result<Value, String> {
    check_arity(name, &args)?;
    let src = text_arg(name, &args, if name == "render" { "1 or 2" } else { "1" })?;
    match name {
        "render" => {
            let (opts, hooks) = read_options(args.get(1))?;
            if !hooks.is_empty() {
                return Err("md.render: hooks need the interpreter".into());
            }
            Ok(rendered_value(render(src, opts, &mut NoHooks)?))
        }
        "headings" => Ok(Value::List(Arc::new(
            headings(src).iter().map(heading_value).collect(),
        ))),
        "slug" => Ok(Value::String(Arc::new(slug(src)))),
        "text" => Ok(Value::String(Arc::new(text(src)))),
        "front" => Ok(match front(src) {
            Ok((data, body, format)) => {
                let mut m = crate::ast::ValueMap::default();
                m.insert("data".into(), data);
                m.insert("body".into(), Value::String(Arc::new(body)));
                m.insert("format".into(), str_value(format));
                Value::Ok(Box::new(Value::Map(Arc::new(m))))
            }
            Err(e) => Value::Err(Box::new(Value::String(Arc::new(e)))),
        }),
        other => Err(format!("md has no function `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(src: &str) -> String {
        render(src, Options::default(), &mut NoHooks).unwrap().html
    }

    #[test]
    fn slugs_follow_github() {
        assert_eq!(slug("Missing map keys return Unit, not a Result"), "missing-map-keys-return-unit-not-a-result");
        assert_eq!(slug("http — HTTP"), "http--http");
        assert_eq!(slug("What's new in 0.88"), "whats-new-in-088");
        assert_eq!(slug("ods.read_csv takes text"), "odsread_csv-takes-text");
        assert_eq!(slug("Ünïcode Größe"), "ünïcode-größe");
    }

    #[test]
    fn headings_numbered_and_titled() {
        let r = render("# Title\n\n## `http` — HTTP\n\n## Same\n\n## Same\n", Options::default(), &mut NoHooks).unwrap();
        assert_eq!(r.title, "Title");
        let slugs: Vec<_> = r.headings.iter().map(|h| h.slug.as_str()).collect();
        assert_eq!(slugs, ["title", "http--http", "same", "same-1"]);
        assert!(r.html.contains("<h2 id=\"http--http\"><code>http</code> — HTTP</h2>"), "{}", r.html);
    }

    #[test]
    fn gfm_tables_tasks_strike_footnotes() {
        let h = html("| a | b |\n|---|--:|\n| 1 | 2 |\n");
        assert!(h.contains("<table>") && h.contains("text-align: right"), "{h}");
        let h = html("- [x] done\n- [ ] todo\n");
        assert!(h.contains("checkbox"), "{h}");
        assert!(html("~~gone~~").contains("<del>gone</del>"));
        let h = html("A note.[^1]\n\n[^1]: The note.\n\nAfter.\n");
        let foot = h.find("<section class=\"footnotes\">").expect("footnotes section");
        assert!(h.find("After.").unwrap() < foot, "{h}");
    }

    #[test]
    fn bare_urls_become_links() {
        let h = html("see https://olang.dev/book. and www.example.com, (https://x.io/a_(b))");
        assert!(h.contains("<a href=\"https://olang.dev/book\">https://olang.dev/book</a>."), "{h}");
        assert!(h.contains("<a href=\"http://www.example.com\">www.example.com</a>,"), "{h}");
        assert!(h.contains("<a href=\"https://x.io/a_(b)\">"), "{h}");
        assert!(!html("`https://x.io`").contains("<a"));
        assert!(!html("[t](https://a.io) https").contains("https</a>"));
    }

    #[test]
    fn raw_html_modes_and_safe() {
        let src = "<div class=x>hi</div>\n\nand <b>bold</b> [x](javascript:alert(1))\n";
        assert!(html(src).contains("<div class=x>hi</div>"));
        let esc = render(src, Options { html: HtmlMode::Escape, ..Options::default() }, &mut NoHooks).unwrap().html;
        assert!(esc.contains("&lt;div class=x&gt;") && esc.contains("&lt;b&gt;"), "{esc}");
        let drop = render(src, Options { html: HtmlMode::Drop, ..Options::default() }, &mut NoHooks).unwrap().html;
        assert!(!drop.contains("<div") && !drop.contains("<b>"), "{drop}");
        let safe = render(src, Options { safe: true, ..Options::default() }, &mut NoHooks).unwrap().html;
        assert!(!safe.contains("javascript:") && !safe.contains("<div"), "{safe}");
    }

    struct Rewrite;
    impl Hooks for Rewrite {
        fn has_link(&self) -> bool {
            true
        }
        fn link(&mut self, url: &str) -> Result<LinkAnswer, String> {
            Ok(if url == "gone" {
                LinkAnswer::Drop
            } else if let Some(p) = url.strip_suffix(".md") {
                LinkAnswer::To(format!("/book/{p}/"))
            } else {
                LinkAnswer::Keep
            })
        }
        fn code(&mut self, lang: &str, info: &str, text: &str) -> Result<Option<String>, String> {
            Ok((lang == "x").then(|| format!("[{lang}|{info}|{text}]")))
        }
    }

    #[test]
    fn hooks_draw_links_and_code() {
        let r = render(
            "[a](ods.md) [b](gone) <img src=\"ods.md\">\n\n```x run\nlet a = 1 < 2\n```\n\n```y\nq\n```\n",
            Options::default(),
            &mut Rewrite,
        )
        .unwrap();
        assert!(r.html.contains("<a href=\"/book/ods/\">a</a>"), "{}", r.html);
        assert!(r.html.contains(" b "), "{}", r.html);
        assert!(r.html.contains("<img src=\"/book/ods/\">"), "{}", r.html);
        assert!(r.html.contains("[x|x run|let a = 1 < 2]"), "{}", r.html);
        assert!(r.html.contains("<pre><code class=\"language-y\">q\n</code></pre>"), "{}", r.html);
    }

    #[test]
    fn shift_and_no_anchors() {
        let r = render("# T\n", Options { shift: 1, anchors: false, ..Options::default() }, &mut NoHooks).unwrap();
        assert_eq!(r.html, "<h2>T</h2>\n");
        assert_eq!(r.title, "T");
    }

    #[test]
    fn front_matter_yaml_and_toml() {
        let (data, body, fmt) = front("---\ntitle: \"Hi: there\"\ndate: 2026-10-20\ndraft: false\nn: 3\nx: 1.5\ntags: [a, \"b c\"]\nauthor:\n  name: Ola\n  links:\n    - https://a.io\n    - x\nsummary: >\n  one\n  two\nlist:\n- k: 1\n  j: 2\n- plain\n---\n# Body\n").unwrap();
        assert_eq!(fmt, "yaml");
        assert_eq!(body, "# Body\n");
        let shown = format!("{}", data);
        for want in ["Hi: there", "2026-10-20", "false", "1.5", "b c", "Ola", "https://a.io", "one two", "plain"] {
            assert!(shown.contains(want), "{want} in {shown}");
        }
        let Value::Map(m) = &data else { panic!() };
        assert!(matches!(m.get("n"), Some(Value::Integer(3))));
        assert!(matches!(m.get("draft"), Some(Value::Boolean(false))));
        let (data, body, fmt) = front("+++\ntitle = \"T\"\n+++\nbody").unwrap();
        assert_eq!((fmt, body.as_str()), ("toml", "body"));
        assert!(format!("{}", data).contains("T"));
        let (_, body, fmt) = front("no front").unwrap();
        assert_eq!((fmt, body.as_str()), ("", "no front"));
        assert!(front("---\ntitle: x\n").is_err());
        assert!(front("---\n  bad\n x: 1\n---\n").is_err());
        // a rendered document skips its front matter
        assert_eq!(html("---\ntitle: x\n---\n# H\n"), "<h1 id=\"h\">H</h1>\n");
    }

    #[test]
    fn inline_drops_the_one_paragraph() {
        let o = Options { inline: true, ..Options::default() };
        assert_eq!(render("a *b* `c`", o.clone(), &mut NoHooks).unwrap().html, "a <em>b</em> <code>c</code>");
        assert_eq!(render("a\n\nb", o, &mut NoHooks).unwrap().html, "<p>a</p>\n<p>b</p>\n");
    }

    #[test]
    fn plain_text() {
        assert_eq!(text("# A *b*\n\nc `d` [e](f)\n\n- g\n- h\n"), "A b\nc d e\ng\nh");
    }
}
