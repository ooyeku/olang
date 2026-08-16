// A small olang tokenizer for the playground editor.
//
// The book's code blocks are highlighted by shiki against the TextMate
// grammar in olang-lang.js, at build time. The editor cannot use that: it
// re-highlights on every keystroke, and shipping shiki plus its Oniguruma
// wasm to do so would cost more than the whole playground bundle. This is
// the same language described by a single-pass regex instead — a few
// hundred bytes, no async, and fast enough to run on input.
//
// The two must agree on colour, so both read `olangTheme`: the classes
// below are emitted as CSS custom properties from the same token colours
// the book uses.

import { PALETTE, PALETTE_LIGHT } from './olang-lang.js';

/** The shared palettes as CSS custom properties, for the editor's styles. */
const vars = (p, suffix = '') =>
  Object.fromEntries(Object.entries(p).map(([n, v]) => [`--tok-${n}${suffix}`, v]));

export const TOKEN_COLORS = vars(PALETTE);
export const TOKEN_COLORS_LIGHT = vars(PALETTE_LIGHT);

// The fifteen reserved words, plus `in`, `spawn`, and `par` — which are
// not reserved but only ever mean one thing where they appear.
const HARD = new Set([
  'fn', 'let', 'mut', 'if', 'else', 'match', 'for', 'while', 'loop',
  'break', 'continue', 'return', 'struct', 'enum', 'in', 'spawn', 'par',
]);

// Contextual keywords (0.64): these introduce a declaration but are
// ordinary identifiers anywhere else, so `let type = row.kind` must not
// paint `type` as a keyword. They only count as one when they open a
// line and a name follows.
const CONTEXTUAL = new Set(['type', 'share', 'use', 'trait', 'impl', 'error', 'test']);

const TOKEN = new RegExp(
  [
    '(?<comment>//[^\\n]*)',
    '(?<raw>r"(?:[^"\\\\]|\\\\.)*"?)',
    // Newlines are allowed inside a string literal in olang, so this must
    // span lines — at the cost of painting the tail of the file as string
    // while a quote is still unclosed, which is what every editor does.
    '(?<str>"(?:[^"\\\\]|\\\\.)*"?)',
    '(?<tmpl>`(?:[^`\\\\]|\\\\.)*`?)',
    "(?<chr>'(?:[^'\\\\]|\\\\.)')",
    '(?<num>\\b0[xX][0-9a-fA-F_]+|\\b0[bB][01_]+|\\b0[oO][0-7_]+|\\b\\d[\\d_]*(?:\\.\\d[\\d_]*)?(?:[eE][+-]?\\d+)?)',
    '(?<word>[A-Za-z_][A-Za-z0-9_]*)',
    '(?<pipe>\\|>)',
    '(?<arrow>=>|->)',
    '(?<op>\\.\\.=?|&&|\\|\\||==|!=|<=|>=|[-+*/%<>=!?|&^~])',
  ].join('|'),
  'g'
);

const ESCAPES = /\\./g;

function esc(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/**
 * Wrap `inner` in a span of `cls` — but close and reopen it at every
 * newline, so no element ever straddles a line. The editor wraps its
 * text, which means the painted layer has to be sliceable into one block
 * per logical line; a `<span>` spanning a newline would make that
 * impossible to do without re-balancing tags.
 */
function spanPerLine(inner, cls) {
  return inner
    .split('\n')
    .map((part) => `<span class="${cls}">${part}</span>`)
    .join('\n');
}

/** A string body with its escape sequences picked out in their own colour. */
function withEscapes(text, cls) {
  let out = '';
  let last = 0;
  for (const m of text.matchAll(ESCAPES)) {
    out += esc(text.slice(last, m.index));
    out += `<span class="t-escape">${esc(m[0])}</span>`;
    last = m.index + m[0].length;
  }
  return spanPerLine(out + esc(text.slice(last)), cls);
}

/** Is `index` the first non-space position on its line? */
function atLineStart(src, index) {
  let i = index - 1;
  while (i >= 0 && (src[i] === ' ' || src[i] === '\t')) i--;
  return i < 0 || src[i] === '\n';
}

/** Does a name (or, for `test`, a string) follow the word at `end`? */
function nameFollows(src, end) {
  let i = end;
  while (i < src.length && (src[i] === ' ' || src[i] === '\t')) i++;
  return /[A-Za-z_"]/.test(src[i] ?? '');
}

/**
 * Highlight olang source as HTML. The output is plain text with `<span>`
 * wrappers and nothing else, so it lines up character-for-character with
 * the textarea drawn on top of it.
 */
export function highlightOlang(source) {
  let out = '';
  let last = 0;

  for (const m of source.matchAll(TOKEN)) {
    const g = m.groups;
    out += esc(source.slice(last, m.index));
    last = m.index + m[0].length;

    if (g.comment) out += `<span class="t-comment">${esc(g.comment)}</span>`;
    else if (g.raw) out += withEscapes(g.raw, 't-string');
    else if (g.str) out += withEscapes(g.str, 't-string');
    else if (g.tmpl) out += withEscapes(g.tmpl, 't-string');
    else if (g.chr) out += `<span class="t-string">${esc(g.chr)}</span>`;
    else if (g.num) out += `<span class="t-number">${esc(g.num)}</span>`;
    else if (g.word) {
      const w = g.word;
      let cls = null;
      if (w === 'true' || w === 'false') cls = 't-const';
      else if (HARD.has(w)) cls = 't-keyword';
      else if (CONTEXTUAL.has(w) && atLineStart(source, m.index) && nameFollows(source, last)) {
        cls = 't-keyword';
      } else if (/^[A-Z]/.test(w)) cls = 't-type';
      else if (source[last] === '(') cls = 't-fn';
      out += cls ? `<span class="${cls}">${esc(w)}</span>` : esc(w);
    } else if (g.pipe) out += `<span class="t-pipe">${esc(g.pipe)}</span>`;
    else if (g.arrow) out += `<span class="t-arrow">${esc(g.arrow)}</span>`;
    else if (g.op) out += `<span class="t-op">${esc(g.op)}</span>`;
  }

  out += esc(source.slice(last));

  // One block per logical line. The blocks are what the gutter measures:
  // a wrapped line occupies several visual rows, and its number has to be
  // as tall as all of them or the column drifts out of step.
  return out
    .split('\n')
    .map((line) => `<span class="ln">${line}</span>`)
    .join('');
}
