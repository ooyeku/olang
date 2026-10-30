# Micro-languages on olang — `peg`, `lang`, and what joins them

*Local-only (branch `rc6`, olang 1.0.0-rc.6) until it is polished. The
remote stays on rc.5.*

## The aim

olang as a platform where people build their own small languages (a
rules language, a query language, a service language like Aval, a
pricing language with units) that:

- **read naturally:** written as the language's own notation, in an olang file or a file of their own, never escaped inside strings;
- **cost nothing at run time:** a micro-language compiles to the same olang a person would have written by hand, so it runs on the bytecode tier and the JIT like any olang;
- **get the tools for free:** syntax colour, errors pointing inside the micro-language's text, completion, hover, go-to-definition, formatting, a test kit, all from the grammar and a few declarations;
- **mix with olang both ways:** olang values flow into the micro-language through typed holes, and what it defines is callable from olang.

## What exists, and what is missing

olang already has half of this:
- the **Open AST** (`meta.parse`, programs as values);
- **macros** (`meta fn`, `@` sites, hygiene, pure expansion, source-mapped errors);
- **three tiers**;
- a **language server**;
- **lexers** for highlighting (`src/lexers.rs`, Loom's `code.ol`).

The missing pieces are what turn a parser library into a platform:

| | missing piece | why it matters |
|---|---|---|
| 1 | **a parser toolkit** (`peg`) | grammars as olang values: packrat with left recursion, precedence tables, good errors, typed trees with spans |
| 2 | **an embedding literal** | today foreign notation "rides inside a string literal" (macros rule 3): escaping, no colour, errors at the site rather than the line. A raw block lets a micro-language be written as itself |
| 3 | **quasi-quotation into the Open AST** | lowering a micro-language by building olang trees with holes, each node carrying the micro-language's span, rather than gluing text |
| 4 | **spans through the whole pipeline** | an error, a profile sample or a debugger stop inside generated code points at the micro-language's own line and column, not the `@` site |
| 5 | **a language kit** (`lang`) | scopes and name resolution, diagnostics with labels and fixes, checking, lowering, an interpreter path for programs loaded at run time |
| 6 | **standalone files** | `olang run rules.aval`: a micro-language with its own extension, registered in `olang.toml`, with `check`, `fmt` and `test` |
| 7 | **tooling from the grammar** | semantic colour, completion from the parser's expected set and the scopes, hover, go-to-definition, folding, a formatter derived from the grammar |
| 8 | **expansion fast and cached** | expanding an imported macro costs about a second today (measured on rc.5: one `@timed` use, 0.95 s of olang-track's start). A platform of micro-languages needs expansion in milliseconds and cached across runs |
| 9 | **a test kit** | a corpus, golden diagnostics, sentences generated from the grammar for fuzzing, lints for ambiguity and unreachable rules |

## The art of what is possible

### A rules language, inside an olang file
```olang
use lang.rules      // a micro-language: its grammar, checks and lowering, shipped as an olang package

let escalate = @rules ```
  when issue.priority is urgent and issue.age > 2h and not issue.assigned
    then assign to on_call(issue.project)
         notify #ops "${issue.key} is urgent and unowned"
  when issue.state becomes done and issue.blocks any
    then comment on each blocked "unblocked by ${issue.key}"
```
```
- **Written as itself:** no quotes to escape, and the block is coloured as rules.
- **Typed holes:** `${…}` is olang, so `issue.key` is checked as olang.
- **Its own errors:** `2h` is a duration, and `issue.agee` would be "no field agee on issue (did you mean age?)", pointing at the word inside the block.
- **No interpreter at run time:** `@rules` lowers at expansion time to plain olang functions, which run on the bytecode tier like hand-written code.

### Aval as a standalone language
```toml
# olang.toml
[languages.aval]
extensions = [".aval"]
front = "lib/aval/front.ol"     # grammar, scopes, checks, lowering
```
```bash
olang run billing.aval          # lowered to olang, cached, run on the tiers
olang check billing.aval        # the language's own diagnostics, with fixes
olang fmt billing.aval          # the formatter derived from the grammar
```
Studio and every editor using `olang lsp` colour `.aval` files, complete
them, show hover and jump to definitions, because the front file declares
the grammar and its scopes.

### A language read at run time, fast
```olang
let calc = peg.grammar(CALC_GRAMMAR)?                      // or @grammar at expansion time
let tree = calc.parse(user_text)                           // Ok(tree) | Err(diagnostics with spans)
```
A grammar given at run time compiles to olang functions once and is
promoted to the bytecode tier like any hot code. Configuration
languages, query bars (olang-track's query language could be one) and
templating all use this path.

## 1. `peg` — the parser toolkit

**The grammar notation.** It's written as the grammar's own text, in an embedding block, or as olang values when built by code:
```olang
@grammar Calc ```
  expr    = sum
  sum     = sum ("+" / "-") product      # left recursion, handled
          / product
  product = product ("*" / "/") unit / unit
  unit    = Number Unit? / "(" expr ")"
  Number  = [0-9]+ ("." [0-9]+)?        # a capitalised rule is a token: no whitespace inside
  Unit    = "km" / "h" / "kg" / "usd"
  skip    = [ \t\n]+ / "//" [^\n]*       # what tokens may be separated by
```
```

**Parsing.**
- PEG with packrat memoisation (on demand, only where it pays), and direct and indirect left recursion (the Warth et al. algorithm).
- Precedence tables for expression grammars, so a language with a dozen operators is one declaration, not a dozen rules:
  `ops = precedence { left "+" "-"; left "*" "/"; right "^"; prefix "-"; postfix "!" }`
- Tokens versus syntax rules, Unicode classes, case-insensitive literals, keywords that are not identifiers, and lookahead (`&`, `!`).

**Trees.**
- Each rule gives a node with its kind, children and span (file, byte range, line and column).
- Labels name the parts: `decl = "service" name:Name "{" members:member* "}"` gives a record with `name` and `members`.
- Actions turn a match into any olang value directly, for the run-time path.

**Errors.**
- The furthest failure, with the set of what was expected there, as people read it ("expected an expression or `)` after `+`").
- Labelled failures for custom messages, recovery points (skip to `;`, `}` or a line end) so one parse reports many errors, and suggestions ("did you mean `service`?").

**Incremental parsing.** An edit reparses only the region it touched. The memo table survives edits, so language servers stay fast.

**Compiled.**
- At expansion time (`@grammar`), the grammar becomes olang functions: one per rule, written as recursive descent with the memo where needed. The bytecode tier and the JIT take them like any code, so there is no grammar interpreter in the hot path.
- At run time (`peg.grammar(text)`), the same generator runs once and the result is cached.

**Performance budget, tested.** A JSON grammar within 3× of the built-in `json.parse`, and an expression grammar at ≥ 1 million tokens a second on the bytecode tier.

## 2. The embedding literal (a language change, small and total)

````olang
@name ```
  any text, kept as written (indentation trimmed to the block's)
  ${olang_expression} holes, parsed as olang
```
````

- **Total parse kept (macros rule 3).** `grammar.pest` gains one literal form, so the file still parses completely before any macro runs. The block's body is opaque text with byte offsets, and its holes are olang expressions the parser reads now.
- **Only for `@` sites.** The site's macro receives the block as a value: its text, its holes (as Open AST nodes) and its spans. Without an `@`, a block is an error. Nothing invisible happens, so rule 1 holds.
- **Highlighting.** Editors colour the block by the micro-language registered for `name` (from the grammar), and fall back to plain text.

## 3. Quasi-quotation and spans

- `meta.quote` builds olang trees with holes, so lowering is structural, never text gluing:
  ```olang
  meta.quote { if ${cond} => ${action} else => () }
  ```
- `meta.at(span, node)` stamps a generated node with the micro-language span it came from. The expander keeps those spans through expansion, the interpreter and the bytecode tier keep them in their position tables, and so:
  - a runtime error points at `rules:3:25`, inside the block;
  - a profiler sample is charged to the micro-language line;
  - the debugger steps through the micro-language's lines.

## 4. `lang` — the language kit

| part | what it gives |
|---|---|
| `lang.front(grammar, passes)` | a front end: parse → passes → lowered Open AST, with the diagnostics of every pass |
| scopes | `lang.scopes`: declare and look up names with spans, nested and shadowing, imports between files; go-to-definition and rename come from it |
| diagnostics | `lang.error(span, message, #{ labels, fixes })`; fixes are edits an editor can apply |
| checks | passes over typed trees with visitors and folds; a type checker kit for simple nominal and structural types, units of measure as a ready example |
| lowering | to Open AST through `meta.quote` with spans (compile time), or `lang.interpret` for the run-time path (a tree-walking evaluator the micro-language author writes as a pass, promoted to bytecode like any olang) |
| formatting | `lang.pretty(tree, layout)`: a Wadler-style pretty printer, with layout hints in the grammar (`group`, `indent`, `line`) |
| registration | `[languages.<name>]` in `olang.toml`: the extensions, the front file. `olang run/check/fmt/test` and `olang lsp` discover it |

## 5. Tooling from the grammar

- **`olang lsp`:** semantic tokens from the token rules and the scopes, diagnostics from the front end, completion from the parser's expected set merged with the names in scope, hover from declarations and doc comments, go-to-definition and references, rename, folding, and document symbols. This works inside embedding blocks in `.ol` files and in standalone files.
- **Studio and Loom:** the code editor's lexer is derived from the grammar (`code_mode` gains a grammar-backed mode); a tree view of a block's parse; the "expanded" view of a block (the olang it lowers to).
- **`olang expand`** shows each block's lowering with its spans.

## 6. The test kit

- **Corpus:** `peg.corpus("tests/corpus")`, where each file holds its source, the expected tree or its diagnostics, and golden updates on request.
- **Fuzzing:** `peg.sentences(grammar, n, seed)` generates valid sentences from the grammar, and grammar-aware mutants generate invalid ones (the parser must answer with a diagnostic, never raise).
- **Lints:**
  - rules nothing reaches;
  - a PEG ordered choice whose later alternative can never match;
  - left recursion the algorithm cannot resolve;
  - tokens overlapping keywords.
- **Benchmarks:** `peg.bench(grammar, corpus)`.

## 7. Performance prerequisites (P0)

1. **Expansion cost.** Find why using an imported `meta fn` costs about a second (the rc.5 measurement) and fix it. Target: under 10 ms per site, with a benchmark that holds it.
2. **An expansion cache.** Key each expansion by the site's text, the macro's module hash and the olang version, reusing `src/compile_cache.rs`; a warm run expands nothing.
3. **Generated parsers on the fast tiers.** Check that the generated rule functions promote (`olang check --tier` clean on the generated code), and tune what refuses.

## Milestones

| | milestone | done when |
|---|---|---|
| P0 | expansion fast and cached | one imported macro site costs < 10 ms cold and ~0 warm; olang-track's start drops by the macro costs (measured) |
| P1 | `peg` at run time | grammars as values, packrat, left recursion, precedence tables, spans, labelled trees, errors with expected sets and recovery; JSON, CSV and expression grammars in the corpus; within budget |
| P2 | `@grammar` | grammars compiled at expansion time to olang functions on the bytecode tier; the same corpus passes; the budget met on the bytecode tier |
| P3 | blocks, quotes, spans | the embedding literal in `grammar.pest`, holes as olang, `meta.quote` and `meta.at`, spans carried to errors, the profiler and the debugger |
| P4 | `lang` | scopes, diagnostics with fixes, passes, a type-checker kit with units of measure as the example, pretty printing, the run-time interpreter path |
| P5 | standalone languages | `[languages]`, with `olang run/check/fmt/test` on registered extensions; imports between a micro-language and olang |
| P6 | tooling | the language server for blocks and files (tokens, diagnostics, completion, hover, definitions, rename, folding); Studio's and Loom's editors colour by grammar; the test kit (corpus, fuzzing, lints) |
| P7 | proof | Aval's front end on `peg` + `lang`; olang-track's query language rebuilt on `peg` with the same answers and speed; a rules language as an example package; the book chapter "Building a language" |

## Risks, and how to meet them

- **The embedding literal touches the grammar.** It is one new token kind, opaque to the parser; the formatter and the existing lexers learn to skip it; the language reference adds one section.
- **Spans through the bytecode tier.** The OVM's position tables gain a span source. That's a tier change, so the tier's own tests (and `--verify-tiers`) cover it.
- **Packrat memory.** Memoise only where a rule is re-entered at one position (measured), with a cap and a fallback.
- **Scope creep.** P7 is the proof. Every earlier milestone ends in something shippable alone (P1 is already a parser library people will use).

## Done, and polished, when
- Every milestone's "done when" holds.
- Aval and olang-track's query language run on the platform at the speed of hand-written code.
- An editor gives a fresh micro-language colour, errors, completion and formatting with nothing but its front file.
- The book teaches building one from scratch in one chapter.
