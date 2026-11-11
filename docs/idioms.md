# Idiomatic olang (draft)

> Draft, 2026-11-11. Not committed. Nothing in the code has been changed.
> Each point is tagged:
> **Today**: olang 1.0.0-rc.8 supports it, so you can write it now.
> **Change**: it needs a language or tool change. That can't land in the
> 1.0 freeze, so it is listed for 1.1.0 or 2.0.0, with the existing
> OL issue where there is one.
> The "before" examples are real code. The "after" examples ran on rc.8,
> or are marked as sketches.

Most of what makes today's code hard to read is habit, not the
language. Studio's `core/` and `face/` hold about 81,000 lines, with
11,300 `#{ }` literals, 5,950 `map_set` calls, no `struct`, no `enum`,
no `error` declaration and no `type` alias. There are 3,500 lines over
160 characters, and two update functions of more than 450 lines.
olang already has bare-key records, record spread, record patterns with
guards, structs, enums, named arguments and module namespaces. The
first rule is to use them.

---

## 1. Data: record, struct, enum or map

- **A value with fixed, known fields is a record `{ k: v }`, not a map
  `#{ "k": v }`.** Records read with `.k`, as maps do. They print, compare
  and serialize the same way, and `#{ "a": 1 } == { a: 1 }` is true. So
  they mix with existing map code without changes. `olang check` warns
  on a misspelt field (`` `nn` is not a key any construction of `o` has
  … did you mean `n`? ``). **Today.**

  ```olang
  // before: apps/studio/face/agentface.ol:837
  return (ah_wait(m, id, #{ "kind": "wait", "wait": "lsp", "deadline": m.now + 20000 }), [])
  // after
  return (ah_wait(m, id, { kind: "wait", wait: "lsp", deadline: m.now + 20000 }), [])
  ```

- **Keep `#{ }` for real dictionaries.** Use it where the keys are data
  (paths, ids, names typed by a person), or where a key is not an
  identifier (`"content-type"`). `VC_SRC` in `tests/convkit.ol` (file path
  → text) is a true map. A message, an effect, a model or an options
  record is not. **Today.**

- **Update with a spread, not nested `map_set`.** `{ ...r, k: v }` copies
  `r` with `k` set, works on maps and records alike, and nests.
  **Today.**

  ```olang
  // before: libraries/loom/lib/layout.ol:852
  let gop = ly_pos(map_set(map_set(map_set(map_set(rec.gop, "role", "listitem"), "selected", sel), "focusable", false), "index", idx), i, count)
  // after
  let gop = ly_pos({ ...rec.gop, role: "listitem", selected: sel, focusable: false, index: idx }, i, count)
  ```

  ```olang
  // a nested update, no map_set_in needed when the path is fixed
  fn step(m) = { ...m, sel: m.sel + 1, view: { ...m.view, top: 0 } }
  ```

  Keep `map_set(m, k, v)` for a computed key, and the
  `m = map_set(m, k, v)` rebind inside a hot loop. `performance.md` says
  that form compiles to an in-place insert.

  **Performance (measured on rc.8, 2026-11-11, 200,000 iterations):**
  - Building a record `{ }` is 2–3× faster than building a `#{ }` map,
    and reading fields from it is as fast or faster.
  - Spread a value of the same kind: `#{ ...m, "k": v }` over a map
    (152 ms), `{ ...r, k: v }` over a record. Spreading across kinds
    converts the value and costs 2–3× more (425 ms).
  - A same-kind spread beats a `map_set` chain (282 ms) for a one-off
    update of a value something else holds.
  - In a hot rebind loop, `m = map_set(m, "a", …)` on a map is in place
    (10 ms). The same on a record (138 ms) or as a spread (131–357 ms)
    copies every time. Keep hot loops on maps with `map_set` until
    records get the in-place path.
  - An arm that runs once per message pays a microsecond or two either
    way.

- **Give a shape you pass around a name with `type Name = { … }`.** A
  shape alias costs nothing at runtime. It lets the checker check every
  `.field` read against one declaration, and it documents the record
  where a reader looks for it. Studio declares none today. The model
  `m`, the message `x`, and each page record are the obvious first
  ones. **Today.**

  ```olang
  type Msg = { tag: String, arg: Any, at: Int }
  fn sa_update(m0: Model, x: Msg) = …
  ```

- **Use a `struct` when construction should be checked.** A struct
  refuses a missing or surprise field, and its field types are checked
  at runtime. It suits values made in many places that must all agree,
  such as work-scheduler's `CtEnvelope` or a Track issue. A record shape
  alias is lighter and enough for internal state. **Today.**

- **Use an `enum` for a closed set of cases, not `"kind": "x"`
  strings.** Studio has 306 `#{ "kind": … }` literals and many
  `if p.kind == "diff" … else if p.kind == "history"` chains. An enum
  gets exhaustiveness warnings from `olang check` and patterns that bind
  the payload. Its limit is payloads: they are positional only
  (`Diff(root, big_ok)`). That pushes people back to maps when a case
  has more than two fields. **Today**, with that limit. Named-field
  variants need a **change**: OL-313 (2.0.0).

  ```olang
  // before: apps/studio/face/gitpage.ol:427
  if p.kind == "diff" => (m, [gd_read_fx(p, p.root, p.big_ok == true)])
  else if p.kind == "history" => (m, [gd_hist_fx(p, 0)]) else => (m, [])
  // after (today, with a record pattern; or an enum Page { Diff(..), History(..) })
  match p {
      { kind: "diff" } => (m, [gd_read_fx(p, p.root, big_ok: p.big_ok ?? false)]),
      { kind: "history" } => (m, [gd_hist_fx(p, 0)]),
      _ => (m, []),
  }
  ```

- **Don't hand-roll records as positional lists.** `mailbox/mb_mailbox.ol`
  keeps "one flat list of nine slots" for speed, and a header comment
  explains what `b[3]` means. Where the speed is measured and needed,
  name the slots (`let MB_DEADLINE = 3`) so the code reads `b[MB_DEADLINE]`.
  Everywhere else, use a record. **Today.**

- **Gaps.**
  - Field punning in literals: `{ holes }` for `{ holes: holes }`. It
    parses in patterns but not in literals. **Change**: 1.1.0 candidate
    (additive). It fits OL-307.
  - A single record syntax with `with` update (`m with { sel: 3 }`).
    **Change**: OL-307 (2.0.0). Most of it is already possible with
    `{ ...m, sel: 3 }`, so the remaining case for OL-307 is nested paths
    (`m with { view.top: 0 }`) and fixed shapes on the native tier.
  - `olang fix records`, which rewrites a `#{ }` whose keys are all literal
    identifiers into `{ }`, and a chain of literal-key `map_set` into one
    spread. This is a tool, not a language change, and today's
    semantics already allow it. **Change**: 1.1.0 candidate. OL-307 has
    it only as a 2.0 migration step. It is the highest-leverage single
    change for readability.
  - Function-typed struct fields with more than one parameter must be
    `Any` (OL-515, 1.0-freeze bug). That is why work-scheduler's
    contracts read `struct { new: Any, register: Any, … }`.

## 2. Naming and prefixes

- **Prefixes like `hl_`, `pz_`, `tz_` and `sr_` hurt.** They were added
  to stop collisions between files imported flat with `use m { names }`.
  The price is that every name starts with two meaningless letters.
  Studio's `VC_PREFIXES` has more than 150 of them: `tz_`/`tv_` (tests),
  `pz_`/`pw_` (profiler), `cf_` (settings). `sr_` is given to both
  `core/search.ol` and `face/wsrun.ol`, so they don't even guarantee
  uniqueness.

- **olang already has qualified access.** `use core.search {}` binds the
  module's namespace, and calls read `search.find(q)`. **Today.**

  ```olang
  // before: apps/studio/face/agentface.ol:36 (a 300+ character use line)
  use core.agent { ag_lat_rows, ag_agents, ag_agent_key, ag_agent_name, ag_sweep, … }
  ag_resolve(root, p)
  // after
  use core.agent {}
  agent.resolve(root, p)
  ```

  Three things stop this from replacing prefixes yet, so it is only
  partly ready:
  - **There is no module alias.** `use core.search as s` fails with a
    misleading runtime error, `Undefined variable: as`. That error
    message is a 1.0 bug worth an OL issue. The alias itself is
    **Change**: OL-312 / OL-279 (2.0.0).
  - **Two modules with the same last segment collide silently.**
    `use core.search {}` then `use face.search {}` makes `search` the
    second one, and `olang check` reports the file clean.
  - **A project module named like a stdlib module shadows it
    silently.** `use core.task {}` replaces `task`.

  Both silent collisions should be `olang check` warnings: a 1.1.0
  candidate, or a 1.0 bug fix if the owner counts silent shadowing as
  one.

- **Until then, prefer whole words for short prefixes in new code.** A
  file `core/trust.ol` exporting `trust_allows` reads better than
  `tr_allows`, and the convention test can check a word prefix as
  easily as two letters. **Today**, as a convention change (convkit's
  `VC_PREFIXES` accepts any string).

- **Name functions as verbs and values as nouns. Drop numbered
  rebinds.** `m0`/`m1`/`m2`/`m3` and `f1`…`f4` in `sa_update` tell the
  reader nothing; `let m = …` shadowing is allowed and usually clearer
  (see §5). **Today.**

- **Booleans read as questions, and absence defaults with `??`.**
  `p.big_ok == true` is there because a missing field is `()`, and a
  condition must be a Bool. `p.big_ok ?? false` says that directly.
  **Today.**

## 3. Long lines versus blocks

- **One idea per line, and about 100 columns.** `olang fmt` does not
  re-wrap, so the formatter will not do it for you. The 30 lines in
  `face/` over 300 characters are each several statements in one
  expression. **Today**, by hand.

  ```olang
  // before: apps/studio/face/agentface.ol:1548
  let head = if len(rows) == 0 => "No bench ran: " + ag_cut(ag_plain(out), 4000) else => "olang bench " + ag_rel(base, path) + (if fs.exists(basef) => " against the baseline" else => " (no baseline yet: Studio's Profiler › Benches sets one)") + (if b.regressed => " · SLOWER" else => "")
  // after
  let head = if len(rows) == 0 => "No bench ran: " + ag_cut(ag_plain(out), 4000) else => {
      let against = if fs.exists(basef) => " against the baseline"
          else => " (no baseline yet: Studio's Profiler › Benches sets one)"
      let slower = if b.regressed => " · SLOWER" else => ""
      `olang bench ${ag_rel(base, path)}${against}${slower}`
  }
  ```

- **Break a long record over lines, one field per line, with a trailing
  comma.** Literals, `use` lists and argument lists all accept that.
  **Today.**

- **Use template strings for text made of parts.**
  `` `${a} at ${b}` `` instead of `a + " at " + to_string(b)`. A template
  formats a value itself, so the `to_string` calls go too. **Today.**
  (Templates have no escapes and do not nest: OL-316, 2.0.0.)

- **Gap.** A width-aware `olang fmt` that breaks long calls, literals
  and `if` chains at a set width. **Change**: 1.1.0 candidate
  (`stability.md` lets fmt output change in a minor release).
  Meanwhile, a line-length rule in the conventions suite (convkit
  already reads every line) would hold new code to it **today**.

## 4. `if … =>` chains versus `match`

- **Use `match` when every branch tests the same value.** A `match` with
  record patterns and guards does what the long early-return chains do,
  in one place. **Today.**

  ```olang
  // before: apps/studio/face/app.ol:2041 (13 such lines in a row)
  if typeof(x.tag) == "String" && str.starts_with(x.tag, "term.") => return tf_update(m, x)
  if typeof(x.tag) == "String" && str.starts_with(x.tag, "nb.d.") => return nd_update(m, x)
  if typeof(x.tag) == "String" && str.starts_with(x.tag, "nb.w.") => return ny_update(m, x)
  …
  // after (today: guards)
  let tag = if typeof(x.tag) == "String" => x.tag else => ""
  match tag {
      t if str.starts_with(t, "term.") => tf_update(m, x),
      t if str.starts_with(t, "nb.d.") => nd_update(m, x),
      t if str.starts_with(t, "nb.w.") => ny_update(m, x),
      …
  }
  // or, better still, a table: [("term.", tf_update), ("nb.d.", nd_update), …]
  // looked up once with first(filter(…))
  ```

- **Keep `if … =>` for one or two conditions on different things**, and
  for early returns at the top of a function (`if !gd_shown(m) =>
  return (m, [])`). Those read well. **Today.**

- **End a `match` with `_` only when you mean "anything else".** Over an
  enum, leave it off so `olang check` tells you about a missing case.
  **Today.**

- **Gap.** String-prefix patterns (`"term." ++ rest => …`) would remove
  the guard noise above. **Change**: OL-292 (2.0.0).

## 5. Pipes

- **Pipe a value through steps when you would otherwise read inside out.**
  **Today.**

  ```olang
  // before: apps/studio/tests/convkit.ol
  fn vc_glob(pat) = filter(map(sort(unwrap_or(fs.glob(fs.join(VC_ROOT, pat)), [])), vc_rel), vc_keep)
  // after
  fn vc_glob(pat) = unwrap_or(fs.glob(fs.join(VC_ROOT, pat)), [])
      |> sort
      |> map(vc_rel)
      |> filter(vc_keep)
  ```

- **Don't pipe one step, and don't pipe into a lambda that only
  reorders arguments.** `x |> f` is no clearer than `f(x)`, and
  `|> (m) => map_set(m, "a", 1)` should be a spread. **Today.**

- **Thread `(model, effects)` with a helper, not numbered names.** The
  mode switch in `sa_update` threads `m1/f1 … m3/f3`. A two-line helper
  makes each step one line. **Today.**

  ```olang
  // sketch
  fn then((m, fx), step) = { let (m2, more) = step(m); (m2, fx + more) }
  (m, [])
      |> then(sa_win_remember)
      |> then((m) => if left_game => gp_leave(m) else => (m, []))
  ```

- **Gap.** A placeholder for the piped value inside a call
  (`|> f(a, _)`, `map(_.name)`). **Change**: OL-310 (2.0.0).

## 6. Errors: `Result`, `?`, `attempt`, `raise`

- **Return `Result` for a failure the caller can handle, and propagate
  it with `?`.** Studio's `core/` has 66 `?` and 243 arms like
  `Err(e) => ""`, which swallow the error at the first step. Let it rise
  to the arm that can tell the person something. **Today.**

  ```olang
  // before: apps/studio/core/project.ol:66
  let parsed = match fs.read_file(toml_path) { Ok(t) => toml.parse(t), Err(e) => Err(e) }
  // after
  let parsed = toml.parse(fs.read_file(toml_path)?)   // inside a fn returning Result
  ```

- **When a default really is right, say so in one call.**
  `match os.get_env("HOME") { Ok(h) => h, Err(e) => "" }`
  (core/project.ol:24) is `unwrap_or(os.get_env("HOME"), "")`. And
  `Err(_)` instead of `Err(e)` when the error is ignored on purpose.
  **Today.**

- **`attempt` is a boundary, not a `try`.** Use it once, around a
  request handler, a plugin call or one record of a batch, where a bug
  must not end the program. Not around ordinary calls. **Today.**

- **`raise(text)` is for misuse by the caller** (a bug), never for a
  condition a caller should handle. **Today.**

- **Use `error` declarations for errors callers will match on.** Nothing
  in Studio, Loom or work-scheduler declares one. Strings work for
  messages shown to a person, but an `error StoreError { Locked,
  Missing: { path: String } }` lets the caller tell cases apart without
  parsing text. **Today.**

- **Gaps.** `??` over `Err` and `let Ok(x) = e else { … }`: **Change**,
  OL-308 (2.0.0). Error context chains: OL-294 (2.0.0).

## 7. Function size

- **An update function is a router, not the work.** `sa_update`
  (face/app.ol, 543 lines) and `wt_update` (face/writeface.ol, 469 lines)
  hold dozens of handlers inline. Route by tag to named functions, one
  per message or per group, each short enough to read on one screen
  (about 40 lines). The routing table is then the map of what the app
  handles. **Today.**

- **Use a block for a few local steps, and a function when the steps
  have a name.** A block keeps its intermediates from leaking (see
  "Blocks are expressions"). If you find yourself writing a comment to
  say what the block does, make that comment the function's name.
  **Today.**

- **Use named arguments for flags and optional values.**
  `gd_read_fx(p, p.root, big_ok: true)` says what `true` means. Named
  arguments and defaults run on every tier. **Today.** (OL-314 in 2.0.0
  is about compiling them faster and about option records.)

## 8. Comments

- **Use `///` doc comments on everything a file shares,** in whole
  sentences: what it is, what it returns, and what `()` means. convkit.ol
  does this well. **Today.**

- **Comment why, not what.** Studio's inline comments are often a
  fragment of what the next line does ("the book's example put at the
  caret of the file in front"). Keep the ones that explain a reason or a
  workaround (`// OL-516: …`), and turn the rest into names.

- **Put a module header `//!` at the top of every file**, saying what
  the file is for and what it must not depend on. mb_mailbox.ol's header
  is a good model. **Today.**

## 9. Module layout

- **Order a file top to bottom.** First the `//!` header, then `use`
  lines (one module per `use`, lists over lines), then shared types and
  constants, then shared functions, then private helpers, then `test`
  blocks. Functions are hoisted and values are not, so constants go
  above their first use. **Today.**

- **Make a file one idea, about 1,000 lines at most.** `face/agentface.ol`
  is past 3,000 lines. Its message handlers, its tool performers and its
  waits are three files. **Today.**

- **Share less.** Every `share` is a name the convention test must
  police and a reader must find. Keep helpers private, and re-export
  with `share use` from one front file (`index.ol`, as work-scheduler
  does) when a package has a public face. **Today.**

- **Gap.** Private-by-default modules with qualified access only.
  **Change**: OL-279 (2.0.0).

## 10. Smaller things

- `first(xs)` / `last(xs)` instead of `xs[0]` / `xs[len(xs) - 1]` with a
  length guard; `x.f ?? d` instead of `if x == () => d else => x.f`. The
  four `olang fix` passes (`guards`, `defaults`, `first-last`, `units`) and
  `olang fix maps` (`map_get(x, "k")` → `x.k`) do these mechanically.
  Run them. **Today.**
- `for (k, v) in entries(m)` to walk a map. **Today.**
- No `+=`: write `n = n + 1`. **Change**: OL-311 (2.0.0).
- No slices: `take(skip(ls, a), n)` (convkit's `vc_def_text`) would be
  `ls[a..a + n]`. **Change**: OL-309 (2.0.0).
- No set type: `contains(VC_OPEN, t)` over a list. **Change**: OL-315
  (2.0.0).
- Use `typeof(x) == "String"` checks once, at the edge where untyped data
  comes in (a message, parsed JSON), not on every line after it. Better
  still, annotate the parameter. **Today.**

---

## Language and tool candidates, summarized

| Candidate | Kind | Release | Existing issue |
|---|---|---|---|
| `olang fix records`: literal-key `#{ }` → `{ }`, `map_set` chains → spread | tool | 1.1.0 | OL-588 (split from OL-307) |
| Width-aware `olang fmt` | tool | 1.1.0 | OL-589 |
| `olang check` warns on two namespace imports with the same last name, and on a module shadowing a stdlib module | checker | 1.1.0 (or a 1.0 fix) | OL-590 |
| `use x as y` gives "Undefined variable: as" instead of a parse error | error message | 1.0 bug | OL-591 |
| Field punning in literals `{ holes }` | syntax | 1.1.0 | OL-592 |
| Module alias `use core.search as search` | syntax | 2.0.0 | OL-312, OL-279 |
| Record `with` update, nested paths | syntax | 2.0.0 | OL-307 |
| Enum variants with named fields | syntax | 2.0.0 | OL-313 |
| String-prefix and richer patterns | syntax | 2.0.0 | OL-292 |
| Pipe placeholder `_` | syntax | 2.0.0 | OL-310 |
| `??` on `Err`, `let … else` | syntax | 2.0.0 | OL-308 |
| `+=` | syntax | 2.0.0 | OL-311 |
| Slices | syntax | 2.0.0 | OL-309 |
| Function types with 0 or 2+ params | bug | 1.0 freeze | OL-515 |

The candidates new in this draft were filed 2026-11-11 as OL-588 to OL-592.
