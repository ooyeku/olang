# Documentation style guide

This guide defines how the olang documentation is written and structured. It
applies to `README.md`, every chapter in the [book](README.md), and any new
documentation added to the repository. Its purpose is to keep the
documentation consistent, complete, and readable as professional technical
reference material.

Part of [the olang book](README.md).

## Audience and purpose

The documentation is written for a broad audience: newcomers learning the
language and experienced programmers who want to use it well. It is reference
material, not marketing. A reader should be able to learn any part of the
language, standard library, or toolchain from the documentation alone,
including the details and edge cases that a working programmer eventually
needs.

## Voice

- Write in neutral, precise, declarative prose. Describe how the language
  behaves and why it is designed that way.
- Do not use jokes, rhetorical questions, cute metaphors, or first-person
  asides. Do not address the reader with sales language ("you'll love", "the
  part no one else has").
- State facts, not claims of superiority. Comparisons to other languages are
  acceptable when they are concrete and accurate ("olang closures capture by
  value; Python closures capture by reference"), but avoid sweeping
  assertions ("no other language offers this").
- Present measurements as data. Benchmark numbers belong in tables with the
  hardware, input size, and comparison baseline stated. Do not editorialize
  them ("blazing fast", "measured, not asserted").
- Prefer short, complete sentences over long sentences joined by dashes.
  Reserve the em dash for genuine parenthetical asides, not for stacking
  clauses.

## Completeness

- Cover the ordinary case, then the boundaries: error behavior, empty and
  null inputs, overflow, ordering, and interactions with other features.
- When a behavior has a rationale that affects how it should be used, state
  the rationale briefly and move on. Design essays belong in the Internals
  and design-record chapters, not in reference chapters.
- Every public function, operator, or command that a reader can use should be
  documented with its signature, its return value (including the error case),
  and at least one example where a signature alone is ambiguous.

## Structure

- **One H1 per file**, matching the chapter title in the book index exactly.
- **Section headings use sentence case**: "Pattern matching", not "Pattern
  Matching". Capitalize only the first word and proper nouns. The language
  name is always lowercase `olang`, including in titles.
- **Chapters longer than roughly 150 lines include a `## Table of contents`**
  immediately after the overview. Shorter chapters omit it.
- Every chapter opens with a one-paragraph overview: what the chapter covers
  and who it is for.
- Every chapter includes a breadcrumb line linking back to the book:
  `Part of [the olang book](README.md).`, optionally followed by sibling
  links.
- Cross-reference other chapters by relative link. Verify that anchors
  resolve to real headings.

## Code examples

- Every ` ```olang ` block in `README.md` and the chapters listed in
  `tests/doc_examples_test.rs` is compiled and executed by the test suite. A
  block that cannot run in that environment (it needs files, a network, a
  browser, or multiple modules) must be marked ` ```olang no-run `, which is
  parse-checked only.
- Keep examples minimal and focused on the feature being described. Show the
  expected output in a comment or with `println` when it aids understanding.
- Use `//` for olang comments. olang has no `#` comment syntax.

## Terminology

- The language is **olang**, lowercase, in all prose and headings.
- The three execution tiers are the **interpreter**, the **bytecode VM**
  (the OVM), and the **JIT**.
- The standard library is organized as **native modules** (implemented in
  Rust), the **data stack** (`ods`, `stats`, `plot`), and **embedded olang
  modules and packages** (compiled into the binary from olang source). When
  stating a module count, enumerate the modules rather than asserting a bare
  number, and keep any stated count consistent with the enumeration.
