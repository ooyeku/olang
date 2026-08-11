// The zero-drift content pipeline. Everything on the site that describes
// olang is read AT BUILD TIME from the repository's canonical sources:
//
//   - book pages      <- ../docs/*.md            (the olang book)
//   - example gallery <- ../examples/README.md   (names + descriptions)
//   - example code    <- ../examples/**/*.ol     (the real programs)
//   - hero snippet    <- ../README.md            (its first tested code block)
//
// Nothing is copied by hand, so the site cannot say something the repo
// doesn't. If a doc changes, the next build changes with it.
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { marked } from 'marked';
import { createHighlighter } from 'shiki';
import { olangGrammar, olangTheme } from './olang-lang.js';

// Locate the repository root by walking up to Cargo.toml — robust no matter
// where the bundler relocates this module (it runs at build time only).
function findRepo() {
  let dir = process.cwd();
  for (let i = 0; i < 6; i++) {
    if (existsSync(join(dir, 'Cargo.toml')) && existsSync(join(dir, 'docs'))) return dir;
    dir = dirname(dir);
  }
  throw new Error('olang repo root not found from ' + process.cwd());
}
const repo = findRepo();
const docs = join(repo, 'docs');
const examplesDir = join(repo, 'examples');

export const GITHUB = 'https://github.com/ooyeku/olang';

// ── syntax highlighting ────────────────────────────────────────────────
let highlighterPromise;
function getHighlighter() {
  highlighterPromise ??= createHighlighter({
    themes: [olangTheme],
    langs: [olangGrammar, 'bash', 'toml', 'json', 'text']
  });
  return highlighterPromise;
}

const LANG_ALIASES = { olang: 'olang', ol: 'olang', bash: 'bash', sh: 'bash', shell: 'bash', toml: 'toml', json: 'json' };

export async function highlight(code, lang = 'olang') {
  const hl = await getHighlighter();
  const resolved = LANG_ALIASES[lang] ?? 'text';
  return hl.codeToHtml(code.trimEnd(), { lang: resolved, theme: 'olang-dark' });
}

// ── the book ───────────────────────────────────────────────────────────
// Chapter order mirrors docs/README.md's reading order.
export const BOOK = [
  { slug: 'tour', file: 'tour.md', title: 'The Tour' },
  { slug: 'language', file: 'language.md', title: 'The Language' },
  { slug: 'types', file: 'types.md', title: 'Types' },
  { slug: 'stdlib', file: 'stdlib.md', title: 'The Standard Library' },
  { slug: 'ods', file: 'ods.md', title: 'The Data Stack' },
  { slug: 'wasm', file: 'wasm.md', title: 'olang in the Browser' },
  { slug: 'packages', file: 'packages.md', title: 'Packages' },
  { slug: 'editors', file: 'editors.md', title: 'Editors' },
  { slug: 'internals', file: 'internals.md', title: 'Internals' },
  { slug: 'ovm', file: 'ovm.md', title: 'The OVM' },
  { slug: 'stability', file: 'stability.md', title: 'Stability' },
  { slug: 'tooling', file: 'tooling.md', title: 'Tooling' },
  { slug: 'roadmap', file: 'roadmap.md', title: 'Roadmap' }
];

const CHAPTER_LINKS = Object.fromEntries([
  ['README.md', '/book'],
  ...BOOK.map((c) => [c.file, `/book/${c.slug}`])
]);

function slugify(text) {
  // GitHub's anchor algorithm: strip punctuation (keeping word chars,
  // spaces, hyphens), then turn EACH space into a hyphen without collapsing
  // — "str \u2014 strings" becomes "str--strings", matching the anchors the
  // docs were written against.
  return text.toLowerCase().replace(/[^\w\- ]/g, '').replace(/ /g, '-');
}

/// Resolve a relative href against a base directory, collapsing `.`
/// and `..` segments (pure string work — these paths never touch the fs).
function resolvePath(baseDir, path) {
  const joined = baseDir ? `${baseDir}/${path}` : path;
  const out = [];
  for (const seg of joined.split('/')) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..') {
      if (out.length && out[out.length - 1] !== '..') out.pop();
      else out.push('..');
    } else out.push(seg);
  }
  return out.join('/');
}

function rewriteHref(href, baseDir = '') {
  if (!href || /^(https?:|#|mailto:)/.test(href)) return href;
  const [path, anchor] = href.split('#');
  // Resolve the link relative to the chapter it appears in, so a
  // cross-chapter href like "types.md" from tooling.md lands on its
  // chapter page.
  const docRel = resolvePath(baseDir, path);
  if (CHAPTER_LINKS[docRel] !== undefined) {
    return CHAPTER_LINKS[docRel] + (anchor ? `#${anchor}` : '');
  }
  // everything else in the repo links to GitHub, resolved from the
  // chapter's real location (docs/<baseDir>/) so subdirectory docs and
  // ../-escapes both produce correct repo paths
  const repoPath = resolvePath(baseDir ? `docs/${baseDir}` : 'docs', path);
  return `${GITHUB}/tree/main/${repoPath}`;
}

/** Render a docs markdown file to { html, toc, title }. `baseDir` is the
 * file's directory relative to docs/ ('' for top-level chapters), used to
 * resolve its relative links. */
export async function renderMarkdown(markdown, baseDir = '') {
  const toc = [];

  // Lex once; pre-highlight every fenced block (shiki is async, marked's
  // renderer is not), then render with a custom renderer.
  const tokens = marked.lexer(markdown, { gfm: true });
  const codeTokens = [];
  const collect = (list) => {
    for (const t of list) {
      if (t.type === 'code') codeTokens.push(t);
      if (t.tokens) collect(t.tokens);
      if (t.items) collect(t.items);
    }
  };
  collect(tokens);
  const highlighted = new Map();
  await Promise.all(
    codeTokens.map(async (t) => {
      const lang = (t.lang || '').split(/\s+/)[0] || 'text';
      highlighted.set(t.text, await highlight(t.text, lang));
    })
  );

  const renderer = new marked.Renderer();
  renderer.code = (code) =>
    highlighted.get(code) ?? `<pre><code>${escapeHtml(code)}</code></pre>`;
  renderer.heading = (text, level, raw) => {
    const id = slugify(String(raw ?? text).replace(/<[^>]*>/g, ''));
    if (level === 2) toc.push({ id, text: String(raw ?? text).replace(/<[^>]*>/g, '') });
    return `<h${level} id="${id}"><a class="anchor" href="#${id}">${text}</a></h${level}>`;
  };
  renderer.link = (href, title, text) =>
    `<a href="${rewriteHref(href, baseDir)}"${title ? ` title="${title}"` : ''}>${text}</a>`;

  const html = marked.parser(tokens, { renderer, gfm: true });
  const titleMatch = markdown.match(/^#\s+(.+)$/m);
  return { html, toc, title: titleMatch ? titleMatch[1] : '' };
}

function escapeHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

export async function renderChapter(slug) {
  const chapter = slug === null
    ? { slug: '', file: 'README.md', title: 'The olang Book' }
    : BOOK.find((c) => c.slug === slug);
  if (!chapter) return null;
  const markdown = readFileSync(join(docs, chapter.file), 'utf-8');
  const baseDir = chapter.file.includes('/')
    ? chapter.file.slice(0, chapter.file.lastIndexOf('/'))
    : '';
  const rendered = await renderMarkdown(markdown, baseDir);
  return { ...chapter, ...rendered };
}

// ── examples ───────────────────────────────────────────────────────────
// Names and descriptions parse straight out of examples/README.md, so the
// gallery lists exactly what the repo documents.
export function examplesCatalog() {
  const md = readFileSync(join(examplesDir, 'README.md'), 'utf-8');
  const entries = [];
  // Package bullets: - [`name/`](name/) — description...
  const bulletRe = /^- \[`([\w/]+?)\/?`\]\([\w/]+?\/?\)\s+—\s+([\s\S]*?)(?=^- \[|^```|^##|\Z)/gm;
  let m;
  while ((m = bulletRe.exec(md))) {
    const name = m[1].replace(/\/$/, '');
    if (name.startsWith('packages/')) continue;
    const desc = m[2].replace(/\s+/g, ' ').replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
      .replace(/`([^`]+)`/g, '$1').trim();
    entries.push({ name, description: desc });
  }
  return entries;
}

/** All .ol/.toml source files for one example, real bytes from the repo. */
export async function exampleSource(name) {
  const dir = join(examplesDir, name);
  const files = [];
  const walk = (d, prefix = '') => {
    for (const entry of readdirSync(d).sort()) {
      const full = join(d, entry);
      if (statSync(full).isDirectory()) {
        if (entry !== 'data' && entry !== 'templates') walk(full, `${prefix}${entry}/`);
      } else if (entry.endsWith('.ol')) {
        files.push({ path: `${prefix}${entry}`, code: readFileSync(full, 'utf-8') });
      }
    }
  };
  walk(dir);
  // main.ol first, then libs
  files.sort((a, b) => (a.path === 'main.ol' ? -1 : b.path === 'main.ol' ? 1 : a.path.localeCompare(b.path)));
  return Promise.all(
    files.map(async (f) => ({ ...f, html: await highlight(f.code, 'olang') }))
  );
}

// ── the hero snippet: the README's first (CI-tested) code block ────────
export function heroSnippet() {
  const md = readFileSync(join(repo, 'README.md'), 'utf-8');
  const m = md.match(/```olang\n([\s\S]*?)```/);
  return m ? m[1].trimEnd() : 'println("hello, olang")';
}
