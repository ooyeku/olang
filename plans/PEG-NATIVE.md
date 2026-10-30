# `peg` — the native matching core

*Local-only (branch `rc6`, olang 1.0.0-rc.6). Follows plans/PEG.md (G2,
the budget).*

## Why

`peg` compiled each grammar (with `meta.eval`) into olang closures, one
recursive-descent function a rule. Every olang call costs 0.3–1 µs and
the JIT refuses the generated code, so on a 151 KB JSON document `peg`
recognised in 22× `json.parse`'s time, built its tree in 100× and its
values in 115×; olang-track's query bar took 83 µs to its hand-written
parser's 51. The budget is JSON within 3× of `json.parse` and the query
bar within 2× (here: within 1×).

## The split

**Olang keeps** everything about the grammar as text and as a design:
reading the notation (rules, tokens and `skip`, labels, `Name "words"`,
precedence blocks), every grammar error and its caret, `validate`, the
analyses (`nullables`, `firsts`, the left-recursion graph and its heads,
the rules an ordered choice may try twice at one place, tokens written
in place inside tokens), `check`, `examples`, `dump`, the error value
(`show()` is a closure) and every public shape. Nothing in the API
changed.

**Rust does the matching** — src/stdlib/peg, an internal module `__peg`
(names beginning `__` are internal hooks; `peg.ol` is the only caller):

| call | answers |
|---|---|
| `__peg.compile(rules, memo, heads, actions)` | an opaque, immutable grammar handle (`PegGrammar`), shareable across threads |
| `__peg.parse(h, input, start)` | `Ok(tree or value)` or `Err(error data)` |
| `__peg.accepts(h, input, start)` | `Bool` |
| `__peg.recover(h, input, start, syncs)` | `Ok(tree)` or `Err([tree, [error data]])` |
| `__peg.listing(h)` | the compiled program as text (a grammar's `source`) |

The rules go over as `peg.ol` already holds them (maps tagged by `"t"`),
with the memoising rules in slot order and the heads of left recursion;
the actions go with them, so a parse passes only the input. Error data
is the error value's fields worded exactly as `peg.ol` words them, plus
the text of `show()`; `parse_error` in `peg.ol` adds the closure.

## The machine, and why not an LPeg VM

The plan suggested an LPeg-style instruction machine (choice / commit /
call / return with a backtrack stack). I built recursive descent over a
flattened program instead:

- **Exactness is the hard part, not speed.** The olang engine's errors
  come from a precise discipline: three families a rule — building
  (`b_`), recognising (`k_`), recognising while noting failures (`tk_`)
  — with tokens muted inside (`pg_fail_in`), a token or described rule
  said as itself where it could not begin (`pg_mute_end`), the piece
  matched before each position (`pg_okk`), a memo slot pair shared by
  `k_` and `tk_`, a seed list keyed by slot and position, and dispatch
  on the next character only where nothing is noted. Recursive descent
  over the same expression tree reproduces each of these line for line;
  a backtracking VM would have to re-derive them from a different
  control structure, and every divergence is a different error message.
- **It is not the bottleneck.** Recognising the JSON document takes
  less time than `json.parse` (below); the tree's cost is making olang
  values.

So: `grammar.rs` flattens every rule body into one array of `Expr`
(names resolved to indices, classes to a 128-bit ASCII bitmap plus
ranges past it, case-insensitive literals lowered once, each ordered
choice with its first-character guards when no alternative can match
nothing, each label with whether it wraps a rule and which of its node's
labels hold lists, each precedence table with its operators sorted
longest literal first, stably, as `prec_ops` sorts them). Nullability,
first sets and the label analysis are recomputed there by `peg.ol`'s own
definitions, so the two agree by construction rather than by
transmission.

`machine.rs` runs it. The three families are two const parameters,
`ev::<B, T>` (B: building, T: noting), so each is its own monomorphised
code with the others' tests folded away. The context of an expression
(skips between elements; inside a token; a noting token's start) is a
small `Copy` value, as `cx` was in the generator. Shared with the olang
engine, exactly: where skip runs; the next-character dispatch of an
ordered choice; the one-entry memo per slot; seed growing for direct and
indirect left recursion (the heads a feedback set, as `left_recursion`
picks them); the precedence climber (prefix operators at the operand,
postfix then infix in the loop, `left` one level tighter, `right` the
same); the three run shapes the generator inlined (`class*` in a token,
`(class / rest)*` in a token, a `skip` that is one class), and the
noting bookkeeping above.

Nodes are compact during the parse — kind, span, children in a shared
store, labels or a table's operator — in an arena; a child carries its
label as the generator's `(name, node)` tuples did. Backtracking
truncates a stack of children. A silent rule's children are spliced
into its caller, a label on a rule marks the children it gathered, a
label on anything else makes its own node, and `pg_unlabel`'s field
order (list labels first, then labels as met, a later single one
overwriting) is kept, since a map's key order is visible.

Positions are code-point offsets: the input becomes its code points with
-1 after the last (as `codes` did), and when it is not ASCII a table of
byte offsets slices the text. Lines and columns are counted by a cursor
moving forward through the nodes, as `advance` did.

**Depth.** Rule calls nest at most 100,000 deep (the interpreter's own
call limit); past it a parse raises "the input nests too deeply". The
stack grows as it goes through the interpreter's `with_stack_headroom`
(stacker), checked every 64 levels. The browser build, whose stack does
not grow, stops at 1,000. No clock, no threads, nothing blocking: the
playground builds it unchanged.

## Trees, actions

`values.rs` turns the arena into olang values bottom up: each node a
map in the order `kind, text, span, children`, then its labels (or a
table's `op`, `fixity`, `operand` / `left`, `right`), exactly as `conv`
made them. A node over the same text as the node made just before it
(a rule wrapping one other, as `value` wraps `String`) shares that
node's `text` and `span` values, and short texts of a large input are
interned; values are immutable, so sharing is invisible.

Actions are called from Rust on the interpreter the builtin was called
with (`call_function`), bottom up, each node's children and labels
already turned. Measured against applying them in olang over a returned
tree: that walk would have to make every map a second time in olang (the
olang engine's `conv`, 120 ms of its 151), so calling the closures from
Rust is the faster of the two by far. What remains is the call itself:
about 0.5 µs an action through the interpreter's call path and the
bytecode tier, comparable to what the olang engine paid (≈ 0.42 µs) from
compiled olang. Two thirds of the values benchmark is those 52,000 calls.

## Files

| file | what |
|---|---|
| src/stdlib/peg/mod.rs | the `__peg` module: the handle, the five calls, recovery's loop |
| src/stdlib/peg/grammar.rs | rule data → the flattened program; the analyses; `listing` |
| src/stdlib/peg/machine.rs | the matcher: three families, memo, seeds, precedence, noting |
| src/stdlib/peg/values.rs | the input as code points; the tree as olang values; error data |
| src/stdlib/peg/tests.rs | the machine's unit tests |
| src/stdlib/embedded/peg.ol | `grammar` hands the rules over; `run_parse`, `accepts` and recovery call the matcher; the olang engine kept behind `engine: "olang"` |
| src/stdlib/embedded/peg_native_test.ol | the differential suite |
| src/stdlib/mod.rs, src/builtin.rs | `__peg` registered and dispatched (with the interpreter, for actions) |
| src/repl.rs | `__` modules kept out of completion |
| benchmarks/peg.ol | both engines against the budget, in one run |

## Recovery

`__peg.recover` runs `parse_recovering`'s loop natively: build from the
current position, keep the start node's children, and at an error run
the noting pass, skip past the next sync string after the furthest
failure and go on. The `Err` carries the tree of what parsed and every
error's data; `peg.ol` makes the error values and the combined `show()`.

## Testing

- **The suites unchanged**: peg_test.ol's 26 blocks and peg.ol's 5 pass
  on the native engine, the olang-track goldens included (398 generated
  queries and 46 curated, hashed answers).
- **Differential** (src/stdlib/embedded/peg_native_test.ol, 6 blocks,
  ~4 s): every grammar built on both engines (`#{ "engine": "olang" }`
  keeps the generator for exactly this), and each input parsed and
  accepted by both, comparing the `Ok` values with `==` and `peg.dump`,
  and the `Err`s field by field and by the text of `show()` (recovery's
  `errors` and `tree` too). Inputs: `peg.examples` sentences under
  several seeds, each with mutations (a character deleted, inserted from
  the grammar's alphabet, replaced, the text cut short, two halves
  swapped), plus hand-picked edge cases.

  | corpus | inputs | parsed | errors |
  |---|---|---|---|
  | JSON | 2,423 | 1,172 | 1,251 |
  | CSV | 1,207 | 838 | 369 |
  | expressions (precedence table, calls) | 2,415 | 1,296 | 1,119 |
  | left recursion (two heads on a cycle, a left-recursive token) | 1,207 | 619 | 588 |
  | olang-track's query bar (1,200 generated queries + examples) | 1,800 | 1,800 | — (it accepts any text) |
  | labels, silent rules, lookahead, Unicode, comments in skip | 1,449 | 729 | 720 |
  | recovery (`recover: [";"]`) | 1,505 | 663 | 842 |
  | **total** | **12,006** | **7,117** | **4,889** |

  and again with actions (600 JSON values, 500 expression values), with
  another start rule (300), with `memo: "all"` and `"none"` (300 each),
  and recovery asked of one parse (2): about 14,000 comparisons, all
  equal.
- **Rust unit tests** (src/stdlib/peg/tests.rs): the machine without an
  interpreter — sequences, choices, skip, tokens, the compact tree,
  error wording, left recursion, precedence, lookahead, case-insensitive
  literals, code points, the memo.

## Measurements

`target/release/olang benchmarks/peg.ol`, Apple M-series, best of
several runs; the olang engine measured in the same run. A tree's time
counts freeing it (a run makes several, each freed during the next):
building alone is 14–15 ms, freeing 3.5. Before this work the brief
measured rc.5's engine at 36 / 163 / 184 ms and 83 µs; the same engine
measures as in the first column on this build.

| | olang engine | native | budget |
|---|---|---|---|
| `json.parse` (151 KB) | 1.6 ms | 1.6 ms | |
| JSON, recognise | 31 ms (19×) | 1.2 ms (0.7×) | 2× |
| JSON, tree | 158 ms (99×) | 18 ms (11.5×) | 3× — **missed** |
| JSON, values (actions) | 178 ms (112×) | 44 ms (27×) | |
| query bar, recognise | 27 µs | 1.3 µs | |
| query bar, tree | 77 µs (1.5× hand) | 8.5 µs (0.17×) | 1× hand |
| query bar, tree + its meaning | 139 µs (2.8×) | 51 µs (1.0×) | |
| olang-track's hand-written parser | 50 µs | | |

(The hand-written parser was measured in the same harness from a copy
of packages/olang-track-query/lib; "its meaning" is peg_test.ol's port
of olang-track's clause rules over the tree, which is olang code and the
same in both columns.)

### Where the tree's time goes

The JSON tree misses its budget, and not in the matcher. Sampled
(macOS `sample`, 5 s of repeated parses of the 151 KB document):

| where | share |
|---|---|
| the allocator (malloc, free) | 45% |
| inserting into the maps (`IndexMap`, keys hashed) | 16% |
| freeing the tree (`Value`'s drop) | 13% |
| making the nodes (values.rs, besides the above) | 12% |
| matching (machine.rs) | 8% |
| other | 6% |

Each of the 52,000 nodes is an olang map (an `Arc`, an index table and
an entry vector), with four `String` keys, a `span` that is another map
with four more, its text (an `Arc<String>`) and its children list: about
fifteen allocations a node, and as many frees when the tree is dropped
— roughly 330 ns a node made and freed on this machine's allocator,
against about 25 ns of matching. Sharing the wrapper nodes' text and
span and interning short texts took the build from 17 ms to 14; what is
left is the shape of `Value` itself. Within 3× would need a node
representation with shared keys — a record whose keys are interned once
per kind, or a map type with static keys — which is a change to the
runtime's value model, not to `peg`, and is left for that.

## What is left

- The JSON tree budget (above): it needs a cheaper map in the runtime.
- Action calls are ~0.5 µs each through `call_function`; a direct
  entry into the bytecode tier for a closure called many times from
  native code would help every higher-order builtin, not only `peg`.
- `__peg` holds the actions with the handle, so a grammar's actions are
  fixed when it is built (as they always were: `peg.grammar` takes
  them).
