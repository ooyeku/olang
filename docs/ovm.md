# OVM — the Olang Virtual Machine

Part of [the olang book](README.md) ·
[Internals](internals.md) · [Stability](stability.md)

This document describes how Olang executes code, what parts of the OVM are
real today, and what is still scaffolding. It is deliberately explicit about
the second category: knowing what *isn't* implemented is more useful than a
feature list that overstates the system.

## Execution model

Olang has three execution tiers.

| Tier | What it is | When it runs |
|---|---|---|
| **Interpreter** | Tree-walking evaluator over the AST | Always; the default and the semantics reference |
| **Bytecode** | Register-based VM (`src/ovm/bytecode.rs`) | Eligible functions, on by default (compiled at first call) |
| **JIT** | Native machine code via Cranelift (`src/ovm/jit.rs`) | Bytecode functions on the pure numeric whitelist, specialized lazily at first call |

The interpreter is the source of truth. The lower tiers are optimizations
that must be **observationally identical** to it — see
[Correctness policy](#correctness-policy) and [The JIT](#the-jit).

### Promotion

The tier is **on by default**: eligible functions are compiled to bytecode
on their first call, so a hot loop inside a function called once still runs
on the VM. There is nothing to configure.

```bash
olang program.ol                   # fast by default
olang --ovm-stats program.ol       # report promoted / rejected / call counts
olang -v program.ol                # log each promotion decision
olang --ovm-tier=50 program.ol     # raise the promotion threshold
olang --no-ovm program.ol          # pure tree-walking interpreter
```

`--no-ovm` is the escape hatch and the semantics reference: it disables the
tier entirely. `--ovm-tier=N` delays promotion until the Nth call (its value
needs the `=` so a bare flag doesn't swallow the filename). A function that
fails compilation is marked ineligible and keeps running on the interpreter —
which is why the default can never change program behavior.

When a function calls another user function, the compiler reports the
unresolved callee rather than giving up; the tier compiles that callee and
retries. Names are registered with the VM *before* compilation, which is what
lets mutually recursive functions resolve each other — and are withdrawn if
compilation fails, so nothing later resolves a call against a name that has no
bytecode.

The decision logic lives in `src/ovm/tier.rs`.

### What can be promoted

A function is eligible when its body uses only the subset the VM implements:

- integer, float, boolean, string, list, and range literals
- arithmetic, comparison, and logical operators
- `if`/`else` expressions
- `match` expressions over literals, wildcards, bindings, integer ranges,
  or-patterns, guards, and destructuring of `Ok`/`Err`, lists (including
  `...rest`), tuples, enum variants (a bare unit-variant name is an
  equality match, not a binding — the interpreter's rule), and struct /
  anonymous-object patterns — nested to any depth
- `Ok(..)` / `Err(..)` construction
- field access (`p.x`) and indexing (`xs[i]`, negatives from the end)
- enum values end to end: unit variants bake as closure constants,
  tuple-variant constructors (`Circle(2.0)`) compile to a `MakeEnum`
  instruction with arity checked at compile time, `==`/`!=` compare
  structurally, and enums round-trip the tier boundary losslessly (they
  used to be crushed into a struct shape that could not convert back)
- map literals (`#{...}`), with the interpreter's key coercion (String
  raw, Int/Float/Bool via to_string), and the map builtins (`map_get`,
  `map_set`, `entries`, `map_merge`, `group_by`, ...) — maps are
  first-class in the VM and round-trip the boundary losslessly
- struct literals and anonymous objects. Literals validate against the
  declared field set at *compile* time with the interpreter's exact rules
  (unknown type, missing field, surprise field all refuse, so the
  interpreter raises its own error); a type redeclared with a different
  shape invalidates compiled functions and its literals refuse from then
  on, keeping the interpreter's live registry the authority
- calls to the pure `math` module functions (`math.sqrt`, `math.sin`,
  `math.pow`, ...) — recognized as builtins when the module is a bare
  identifier, so a numeric kernel promotes instead of falling back on its
  first `sqrt`
- pipelines (`|>`), desugared to the equivalent call
- template strings, with the interpreter's exact interpolation rules
  (String raw; Int/Float/Bool via to_string; everything else through the
  value's display form)
- nested `fn` declarations, compiled as named closures over the current
  frame — including self-recursive ones, whose recursion resolves to
  their own compiled id (with captures carried through each recursive
  call) and whose escaped copies recurse interpreted through the
  carried name
- tuple expressions and `let` destructuring through the full pattern
  machinery — a non-matching let raises the interpreter's
  PatternMatchFailed
- free identifiers that resolve in the function's own closure — global
  constants, module-level bindings, and named functions passed as values —
  baked as constants. Sound because the interpreter installs exactly that
  closure as the call environment, and closures are declaration-time
  snapshots: a global mutated after the function's declaration is not seen
  by either tier (pinned by a test)
- lambdas, including those capturing the enclosing function's *runtime*
  state (a parameter, a local), and those referencing registered user
  functions that are not in the closure — self-reference and forward
  (mutual) recursion through a lambda both compile, with the function
  value carried in the lambda's escaped form so a bridged copy resolves
  it identically. Free variables resolving in the
  declaration-time closure travel with the lambda as that snapshot;
  runtime captures compile via a `MakeClosure` instruction that reads the
  captured registers at the lambda expression — the interpreter's own
  capture-by-value moment — with the lambda body compiled once, taking
  the captures as hidden trailing parameters. The resulting closure runs
  natively through `map`/`filter` and converts losslessly to an
  interpreter function when it crosses the tier boundary
- blocks, `let` bindings, assignment to locals, `while` and `for` loops,
  `break`, and `continue`
- method calls (`value.m(..)`) on pure receiver expressions — locals,
  field chains, literals. A `CallMethod` instruction mirrors the
  interpreter's dispatch exactly: struct fields take precedence (called
  without self), then a direct `impl` for the receiver's runtime type,
  then the type's traits in registration order for a default; the
  resolved method runs compiled when its body compiles and bridges
  otherwise. Late `impl` declarations invalidate compiled functions so
  new types dispatch correctly
- calls through function *values*: parameters and locals holding
  functions (`f(x)` where `f` is a parameter — locals shadow builtins,
  exactly as interpreted), curried calls (`g(a)(b)`), immediately invoked
  lambdas, and aliased functions from the closure. Compiled function
  values run in the VM; anything declined runs through the bridge
  interpreter, which owns arity errors, default parameters, and the
  non-callable error. Function values also round-trip the tier boundary
  now (wrapped verbatim), so higher-order user functions promote
- calls to ANY native stdlib module function (`db.query`, `fs.read`,
  `json.parse`, `col.frequencies`, `re.find`, ...): the module resolves
  in the closure to its Module value, the function's existence is
  validated at compile time against the module's own field set, and the
  call bridges under the builtin's own dispatch name — the same
  implementation the interpreter runs. `math.*` and `str.*` additionally
  have fast paths; `show` bridges like `to_string`
- calls to itself (recursion), to other user functions (compiled on demand,
  including mutual recursion), and to a broad builtin set — the core
  builtins plus the pure `math` and `str` modules (see [Builtins](#builtins))

Anything else causes the function to stay interpreted:

- *assigning* to a global (reads bake as snapshot constants; a write
  would need the interpreter's environment)
- method calls on *effectful* receiver expressions (`make_thing().m(..)`):
  the interpreter's dispatch fallthrough re-evaluates the receiver, which
  the VM will not replicate for an expression with side effects — pure
  receivers compile (see above)
- a free identifier absent from the function's closure — the interpreter
  would resolve it through the caller's runtime scope chain, which no
  compile-time snapshot can represent
- async (`spawn`, `await`, promises), and struct-variant enum
  construction (unit and tuple variants compile)
- `return` and `break value` — both unwind in ways the bytecode loops
  don't model
- a lambda capturing a *local* the enclosing function binds only *later*
  (no register holds it yet at the lambda expression) — a *function*
  declared later resolves through the registry and compiles
- or-patterns that bind variables (each alternative would otherwise
  leave different bindings on the success path)
- calling a function that itself cannot be compiled (the rejection propagates
  to every caller)
- default parameter values
- arguments or return values that don't round-trip through the OVM value
  model (promises, builtins, module values — and any struct holding one
  of those). Structs, objects, parsed JSON objects, maps, enums, and
  function values all round-trip when their contents do

None of these are errors. They are compile-time rejections that fall back to
the interpreter, which is why enabling the tier can never break a program.

## Correctness policy

**Promotion may never change what a program does.** Everything the VM cannot
handle is rejected at compile time rather than approximated at runtime. This
rule is enforced by two test suites:

- `tests/bytecode_differential_test.rs` (32 groups) runs programs through
  *both* the interpreter and the VM and asserts identical results —
  arithmetic and overflow errors, float semantics, comparisons, branches,
  recursion, loops, strings, lists, ranges, arity errors, and a set of
  aliasing cases specific to the register-window design. It also asserts the
  inverse: unsupported features must be *rejected*, never miscompiled.
- `tests/bytecode_tier_test.rs` (130+ tests) runs whole programs with and
  without the tier enabled and asserts the observable results match,
  including mixed programs where some functions are promoted and others are
  not, transitive and mutual recursion, and function redefinition.

If you extend the VM, extend the differential suite in the same change. A
divergence found by these tests is a bytecode bug by definition — the
interpreter defines the language.

## Value model

`OvmValue` (`src/ovm/value.rs`) is the VM's runtime value: **exactly 16
bytes, pinned by a size test**. Immediate values (integer, float, boolean,
unit) are stored inline; heap values (string, list, tuple, function,
struct, range, native handles) hold `Arc` payloads. The former per-value
header (type tag, tier, lazy state) was measured fully dead and removed
in 0.40 — the 32 → 16 byte slimming alone bought 13% on the most
value-bound workload, since every register move copies a value.

**Memory is managed by reference counting.** There is no tracing collector.
Values are reclaimed deterministically when the last reference drops, which
suits a language whose values are overwhelmingly immutable and acyclic.

`src/ovm/gc.rs` holds only the safepoint flags the interpreter polls in
loops; the former tracing-GC machinery, region allocator, and the `:gc` REPL
command were deleted in the 0.24 cleanup — with reference counting there is
nothing to force.

The next slimming rung — NaN-boxing to 8 bytes — has its primitives
landed and proven (`src/ovm/nanbox.rs`: canonicalized floats, 48-bit
small integers and pointers, boundary-tested) but is **deliberately not
wired**: the 16-byte measurement re-priced the win at roughly another
10%, against the cost of manual refcounting at every register move — the
exact raw-pointer failure mode this codebase already measured, found
leaking, and deleted in 0.23. The verdict is recorded in
[the roadmap](roadmap.md#the-performance-campaign-039) so the rung is
not re-attempted without new data.

## Bytecode VM

A register machine. Key design points:

- **Register windows.** Variables *are* registers: parameters occupy registers
  `0..n` in declaration order and `let` bindings allocate their own register.
  Reading a variable compiles to nothing at all — it names the register the
  value already lives in. There is no separate locals array to copy through.
- **Two-pass label resolution.** Jumps are emitted against label ids, then a
  resolution pass patches every target to an instruction offset before
  execution. A jump to an unplaced label is a compilation error.
- **One register slab, frame windows.** All live frames share one
  contiguous register array: a call bumps a window past the caller's and a
  return restores two integers, so calling costs no allocation, no frame
  swap, and no per-call vector bookkeeping (the Lua register-stack
  design). Arguments copy straight from the caller's window into the
  callee's. Bounds checks are against the window, so a compiler bug can
  never read another frame's registers.
- **Interned struct shapes with inline caches.** A struct is an interned
  shape (one per type + field set, with a stable id) plus a values vector
  in shape order. Every field-access site carries a one-entry inline
  cache packing (shape id → field index) into a single atomic word: a
  repeat read of the same shape is an integer compare and an array index,
  no hashing. Misses take a cold outlined path that refills the cache.
- **Immediate operands.** A binary operation with a numeric literal
  operand (`x + 1`, `n < 2`) carries the literal in the instruction —
  no constant load, no constant register — through the same fast path
  and fallback as the register form, so error behavior is identical.
- **Native operand dispatch.** Arithmetic and comparisons match on the value
  representation directly; there is no conversion to and from the AST value
  type inside the loop.
- **Bounded recursion.** The VM enforces the same 1000-frame call-depth limit
  as the interpreter, so runaway recursion reports an error instead of
  overflowing the host stack.
- **Index-based iteration.** `for` compiles to an index loop over `IterLen`
  and `IterGet`, which handle both lists and ranges. A range is never
  materialized, so iterating `0..10000000` costs no memory — matching the
  interpreter.
- **Guarded extraction.** Destructuring emits a shape test before any
  extraction, so an extractor can never see a value it doesn't fit. Nested
  patterns recurse on the extracted register.
- **Lambdas as constants.** A lambda whose free variables all resolve in
  the enclosing function's declaration-time closure is built once at compile
  time and stored in the constant pool as an `AstFunction` — an interpreter
  function held verbatim, which converts back losslessly. It carries a
  closure filtered to exactly its free set: attaching the full snapshot
  would defeat the interpreter's empty-closure fast path on every trivial
  lambda call.
- **Total pattern tests.** `match` uses `PatternEq` and `PatternInRange`
  rather than the ordinary comparison instructions, because a pattern of the
  wrong type must simply not match where an ordinary comparison would raise a
  type error. Falling past every arm emits `MatchFail`, the interpreter's
  pattern-match failure.

Optimizations land as compile-time instruction *selection* (immediate
operands, resolved call targets, interned shapes), not as passes over
emitted bytecode. The previous pass pipeline (dead code elimination,
register renaming, peephole rewrites) was removed because each was
incorrect. One measured negative result is on record: fusing
compare+branch pairs retired 3.5% of executed instructions and ran 3–4%
*slower* — growing the instruction set perturbs the dispatch loop's code
layout more than the saved dispatches earn back — so instruction-count
reduction is not pursued for its own sake (see the changelog for the
details).

## The JIT

The third tier (`src/ovm/jit.rs`, 0.40): hot bytecode compiled to native
machine code via Cranelift, extending the correctness ladder unchanged —
"can't compile identically → stay interpreted" gained a rung: "can't
compile natively → stay on bytecode".

A function prequalifies at promotion time when every instruction falls
in a **pure numeric/boolean whitelist**: arithmetic, comparisons, logic,
branches, struct field reads, calls to other olang functions, and
return. Compilation is
**type-specialized, lazy, and call-graph aware**: on a function's first
call, the JIT plans every function reachable through its call sites,
runs kind inference to a global fixpoint across the group (callee
return kinds feed caller registers; masks only grow, so it converges),
and compiles the whole group with direct native-to-native calls —
helpers, chains, and mutual recursion all stay native. Each compiled
function guards its entry on the exact Int/Float argument kinds it was
specialized for (one specialization per function); any other shape runs
on bytecode. Register kinds are proven by the same fixpoint inference
(i64 or f64 per register; mixed int/float arithmetic promotes the
integer side exactly as the VM does; a register may hold mixed kinds
only if nothing ever reads it — the dead result slot of an `if`
statement, say — with liveness flowing backwards through copies).
Struct arguments pass into native code as *borrowed* pointers — JIT
calls are synchronous and the caller's slot outlives the call, so no
refcount is ever touched — specialized per interned shape with field
indices resolved at compile time; every read goes through one guarded
host helper that deopts on any surprise, so no layout assumption leaks
into the VM. Everything else stays on bytecode with zero overhead beyond
one table lookup per call.

Purity is the load-bearing property. A qualifying function has no side
effects, so every guard failure — argument-kind mismatch at entry,
integer overflow, division by zero (integer *and* float — olang errors
there rather than producing inf), `i64::MIN` edge cases, recursion-depth
exhaustion — simply **deopts**: the native run is abandoned and the same
call re-executes on bytecode, which produces the exact result or error
the VM would have produced anyway. The JIT never reproduces an error
message; it only ever declines (float modulo, for instance, is declined
outright: fmod has no exact IR equivalent, and guessing is how
divergence starts). Every native call — self- or cross-function —
carries a depth budget clamped to the VM's own `max_call_depth`, so
runaway recursion errors exactly as it does on bytecode instead of
overflowing the native stack, and a deopt anywhere in a native call
chain unwinds the whole chain back to the bytecode entry.

What this buys, measured: fib(30) 89 ms → **5 ms** (18×, now level with
the JavaScript JITs) — and fib split across two mutually recursive
functions runs at the same 5 ms where the self-call-only JIT managed
94 ms; integer loop kernels 20–30×; float kernels (Mandelbrot-style
orbit loops) ~4.5×; a struct-field kernel 143 ms → 18 ms (8×); and
`par_map` over a jitted kernel compounds both campaigns — workers carry
their own tier and JIT. Strings and other heap values are future
expansions, as is whitelisting the pure `math` builtins: N-body's inner
kernel awaits `math.sqrt`.

`tests/jit_test.rs` holds the parity suite: every guard edge runs tiered
and interpreted and must agree byte-for-byte.

## Builtins

The VM mostly does not reimplement builtins — it calls the interpreter's
own implementations through `BuiltinFunctions::call`. Reimplementation
would be a second source of truth that could drift from the semantics the
differential tests hold the VM to; delegating makes them identical by
construction. The one measured exception: ten collection builtins (`len`,
`head`, `tail`, `cons`, `concat`, `skip`, `map_get`, `map_set`,
`map_has_key`, `entries`) run natively on the VM value model with zero
boundary conversion, each mirroring the interpreter's checks in the same
order with the same messages — the boundary tax on `map_get`/`map_set`
was what kept environment-threading code slow (see the 0.38 changelog).

The enabled set spans the core builtins below plus the pure `math`
module (33 functions), the pure `str` module (30 functions), and `show`:

| Group | Builtins |
|---|---|
| Conversion | `to_string`, `show`, `to_int`, `to_float`, `typeof`, `len` |
| List access | `head`, `tail`, `cons`, `concat`, `reverse`, `sort`, `take`, `skip`, `flatten`, `zip`, `enumerate`, `chunk`, `range` |
| Aggregation | `sum`, `min`, `max`, `average`, `contains` |
| Higher-order | `map`, `filter`, `reduce`, `fold`, `find`, `map_filtered`, `result_map`, `result_map_err`, `unwrap_or_else` |
| Strings | `split`, `join`, `starts_with`, `ends_with` |
| Maps | `map_get`, `map_set`, `map_has_key`, `map_remove`, `map_keys`, `map_values`, `map_len`, `map_merge`, `map_clear`, `entries`, `group_by` |
| Results | `is_ok`, `is_err`, `unwrap`, `unwrap_or` |
| Numeric | `clamp` |
| Output | `print`, `println` |

The higher-order builtins became available when compiled lambdas gained
closures: a lambda the VM builds itself can be handed to a delegated builtin
as a function value. `map`, `filter`, and `sum` go further: when the
collection is a list and the function argument compiles (checked once and
cached per function value), the loop runs *inside* the VM — one bytecode
call per element, no conversion at the boundary — and a mapped list flows
into `sum` without leaving the VM's value model. Anything declined bridges
to the interpreter, which stays the semantic authority; a native loop never
falls back mid-flight, so element errors propagate exactly as the
interpreter would.

`BytecodeVm::round_trips` is the single definition of which values survive the
boundary; the tier uses it to decide whether a call's arguments and result can
cross. Keeping a second copy in the tier caused a silent regression once — it
omitted `Ok`/`Err`, so every call passing a `Result` fell back to the
interpreter despite Results converting fine.

A user function shadows a builtin of the same name, matching the interpreter's
environment lookup: declaring `fn clamp(...)` makes calls to `clamp` resolve to
the user's definition in compiled code too.

## Performance

Measured on an Apple Silicon laptop, release build, as of 0.40. All
workloads are algorithm-identical across languages and checksum-verified
(the N-body sample position matches across every implementation to the
last digit). Since the tier is on by default, the olang numbers are what
a plain `olang program.ol` gets — no flags.

Four representative workloads against the field (as of 0.40, with the
baseline JIT):

| Workload | Rust | Node | Bun | CPython | Ruby | **olang** |
|---|---|---|---|---|---|---|
| N-body (120 bodies × 150 steps) | 2.5 ms | 6 ms | 8 ms | 404 ms | 474 ms | **~322 ms** |
| Word frequency (50k tokens × 20) | 5 ms | 17 ms | 12 ms | 16 ms | 62 ms | **26 ms** |
| `map(λ) \|> sum` pipeline, 1M elements | ~0 ms | 9 ms | 4 ms | 24 ms | 21 ms | **13 ms** |
| fib(30) (2.7M recursive calls) | 1.4 ms | 4 ms | 4 ms | 46 ms | 44 ms | **5 ms** |

(Interpreter-only mode runs the same programs at 23.6 s / — / 370 ms /
1.1 s; the word-frequency workload exceeds the interpreter's allocation
guard entirely, so the tier is what makes it runnable at this size.)

The shape of the result: on pure numeric work the JIT puts olang
**level with the JavaScript JITs** — fib(30) at 5 ms sits beside Node
and Bun's 4 ms and runs 9× ahead of CPython and Ruby; the pipeline
workload (a jitted lambda inside the VM's native map loop) leads CPython
and Ruby outright. N-body — structs, floats, and `math.sqrt` in a hot
loop — went 400 → ~322 ms across 0.40 (the 16-byte value model plus JIT
struct field access) and stays ahead of both; its inner kernel is one
whitelist entry away from native (`math.sqrt` is not yet a JIT builtin).
The remaining gap to Rust is the price of guards, boxing at tier
boundaries, and the not-yet-whitelisted heap paths — the rungs the
roadmap sequences next.

Against its own interpreter, the tiers are worth roughly 73× (N-body)
to 220× (fib, bytecode + JIT): the whole N-body simulation —
construction, stepping, capturing lambdas, struct building, field
access, `math.sqrt` — runs as 6 promoted functions, 0 rejected, with 7
tier crossings.

`--ovm-stats` prints promotions, rejections, tier crossings, and
instructions retired; `--verbose` names each promoted or refused
function with the reason. `otc ovm --compare <file>` runs a program both
ways, verifies the results agree, and reports timings.

`cargo bench` measures the interpreter itself (`benches/interpreter_bench.rs`,
ten representative programs). Use it when changing the evaluator; use
`tier_compare` when changing the VM.

## Known limitations

These are real gaps, not oversights:

1. **Method calls on *effectful* receiver expressions stay interpreted.**
   `make_thing().m(..)` refuses because the interpreter's dispatch
   fallthrough re-evaluates the receiver, which the VM will not replicate
   for an expression with side effects. Pure receivers — locals, field
   chains, literals — compile (see below).
2. ***Assigning* to a global is uncompiled** (reads bake as snapshot
   constants). Async constructs (`spawn`, `await`, promises) stay
   interpreted.
3. **Bridged builtin calls cost a value round trip.** Builtins outside
   the native set convert arguments and results between the OVM and AST
   value models per call. The native set now spans the higher-order
   loops (`map`/`filter`/`sum`) and the collection core (`len`, `head`,
   `tail`, `cons`, `concat`, `skip`, `map_get`, `map_set`,
   `map_has_key`, `entries`), which is what environment-threading and
   list-building code lives on; `reduce`, `fold`, and the rest still
   bridge.

## Not implemented

- **Tracing garbage collection.** Memory is reference-counted; `gc.rs` holds
  only the safepoint flags the interpreter polls.
- **Full NaN-boxing.** The 8-byte value scheme's primitives are landed
  and proven (`src/ovm/nanbox.rs`) but deliberately not wired — the
  measured verdict is in [the roadmap](roadmap.md#the-performance-campaign-039).
- **Automatic SIMD vectorization** and pipeline fusion: the speculative
  engines were deleted rather than finished — explicit bulk stdlib
  operations are the honest route if vectorization matters later.

## Source map

| File | Role |
|---|---|
| `src/interpreter/` | Tree-walking evaluator; the semantics reference |
| `src/resolve.rs` | Slot resolution for function bodies |
| `src/ovm/tier.rs` | Promotion decisions and eligibility |
| `src/ovm/bytecode.rs` | Compiler, instruction set, and dispatch loop |
| `src/ovm/value.rs` | `OvmValue`, the 16-byte reference-counted value model |
| `src/ovm/jit.rs` | The Cranelift JIT: whitelist, kind inference, guards, deopt |
| `src/ovm/nanbox.rs` | 8-byte NaN-boxed value primitives (proven, not wired) |
| `src/ovm/gc.rs` | Safepoint flags the interpreter polls in loops |
| `src/native.rs` | Module registry for native values, spanning both tiers |
| `src/ods/`, `olang-ods/` | The data stack: language surface and pure-Rust engine |

Deleted in the 0.24 cleanup (~12,000 lines): the `OlangVirtualMachine`
routing layer and `ovm_integration` (measured ~70% slower than the plain
interpreter, and the default path until 0.24), the placeholder JIT
scaffolding, the tracing-GC/region-allocator remnants, and the pipeline,
SIMD, lazy, fusion, and adaptive engines — none of which were wired into
execution.
