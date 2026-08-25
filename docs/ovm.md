# The execution model: OVM and JIT

Part of [the olang book](README.md) ·
[Architecture and internals](internals.md) ·
[Stability and compatibility](stability.md)

This chapter describes the two lower execution tiers in detail: the bytecode
virtual machine (the OVM) and the Cranelift JIT. It covers what each tier
compiles, how each stays faithful to the interpreter, and what each does not
compile. The set of constructs a tier does not support is documented as
precisely as the set it does, because a program's behavior depends on knowing
where a tier falls back. For the rationale behind the tiered architecture, see
[Architecture and internals](internals.md#why-an-interpreter-is-the-authority).

## Table of contents

- [Execution model](#execution-model)
- [Correctness policy](#correctness-policy)
- [Value model](#value-model)
- [Bytecode VM](#bytecode-vm)
- [The JIT](#the-jit)
- [Builtins](#builtins)
- [Performance](#performance)
- [Known limitations](#known-limitations)
- [Not implemented](#not-implemented)
- [Source map](#source-map)

## Execution model

olang has three execution tiers.

| Tier | What it is | When it runs |
|---|---|---|
| **Interpreter** | Tree-walking evaluator over the AST | Always; the default and the semantics reference |
| **Bytecode** | Register-based VM (`src/ovm/bytecode.rs`) | Eligible functions, on by default (compiled at first call) |
| **JIT** | Native machine code via Cranelift (`src/ovm/jit.rs`) | Bytecode functions on the JIT whitelist — numbers, math builtins, structs, lists, tuples, strings — specialized lazily at first call |

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
  structurally, and enums round-trip the tier boundary losslessly
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
  non-callable error. Function values round-trip the tier boundary
  (wrapped verbatim), so higher-order user functions promote
- calls to ANY native stdlib module function (`db.query`, `fs.read_file`,
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
- background tasks (`spawn`), and struct-variant enum
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
rule is enforced by three test suites:

- `tests/bytecode_differential_test.rs` runs programs through
  *both* the interpreter and the VM and asserts identical results —
  arithmetic and overflow errors, float semantics, comparisons, branches,
  recursion, loops, strings, lists, ranges, arity errors, and a set of
  aliasing cases specific to the register-window design. It also asserts the
  inverse: unsupported features must be *rejected*, never miscompiled.
- `tests/bytecode_tier_test.rs` runs whole programs with and
  without the tier enabled and asserts the observable results match,
  including mixed programs where some functions are promoted and others are
  not, transitive and mutual recursion, and function redefinition.
- `tests/tier_agreement_test.rs` runs the *corpus* — every runnable
  example in this book and every standalone example program — under
  `--no-ovm` and under `--ovm-tier=1`, comparing stdout, stderr, and
  exit status. It exists because the two suites above call functions
  that a person thought to write down, and the first divergence it
  found was one nobody would have: the interpreter read `()` as Unit
  while the compiler built a zero-element tuple, so a hot function
  comparing `x == ()` answered differently from a cold one. Nothing
  failed. The fast tier simply won, and it was noticed by accident
  while adding an unrelated feature.

The contract covers more than results: **runtime error traces are
tier-identical too**. When a program dies at depth, the reported error
carries the same call frames, the same spans, and the same message
whether it ran interpreted, on bytecode, or through a JIT deopt — the
VM tracks statement spans and rebuilds interpreter-shaped frames on
the way out, so a stack trace never reveals which tier executed the
code. `tests/bytecode_tier_test.rs` holds `traces_agree_*` cases
asserting exactly that.

If you extend the VM, extend the differential suite in the same change. A
divergence found by these tests is a bytecode bug by definition — the
interpreter defines the language.

## Value model

`OvmValue` (`src/ovm/value.rs`) is the VM's runtime value: **exactly 16
bytes, pinned by a size test**. Immediate values (integer, float, boolean,
unit) are stored inline; heap values (string, list, tuple, function,
struct, range, native handles) hold `Arc` payloads. Value size is a
first-order performance input for a register machine — every register
move copies a value — which is why the representation carries no header,
no type tag beyond the enum discriminant, and no per-value bookkeeping:
anything that widens the value slows every instruction.

**Memory is managed by reference counting.** There is no tracing collector.
Values are reclaimed deterministically when the last reference drops, which
suits a language whose values are overwhelmingly immutable and acyclic.
(`src/ovm/gc.rs` holds only the safepoint flags the interpreter polls in
loops.)

The next slimming rung — NaN-boxing to 8 bytes — has its primitives
implemented and boundary-tested (`src/ovm/nanbox.rs`: canonicalized
floats, 48-bit small integers and pointers) but is **deliberately not
wired**. Measurement priced the remaining win at roughly 10% on the most
value-bound workloads, against the cost of manual reference counting at
every register move — a raw-pointer discipline with a demonstrated
leak-prone failure mode. The verdict and its evidence are recorded in
the [CHANGELOG](../CHANGELOG.md) and the repository history so the
question is not reopened without new data.

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
- **Bounded recursion.** Both tiers spend from one shared call-depth
  budget — 100,000 frames by default, `--max-depth` to change it. The
  budget is seeded across the tier boundary (interpreter frames count
  against the VM's budget and vice versa), so a recursion that promotes
  mid-descent errors at exactly the same depth it would have errored
  interpreted. The cap is physically reachable: each user call grows the
  Rust stack in segments when headroom runs low, so the limit is the
  number, never the stack.
- **Tail calls are eliminated on every tier.** A self-call in tail
  position — an `if` branch, a match arm, a block's final expression,
  a `return`'s expression — runs in the caller's frame: the
  interpreter trampolines, the VM rebinds the parameter registers and
  jumps to the entry, and the JIT compiles that jump as a native loop.
  Tail recursion therefore runs at any depth on O(1) stack and never
  meets the depth cap; elided frames are noted in error traces. Self
  is decided by function identity, so a same-named shadow is an
  ordinary call, and every elided frame still crosses the full call
  boundary — arity, trait bounds, and parameter annotations. A runaway
  *tail* recursion is therefore an infinite loop, not a depth error —
  tail calls are iteration, and it non-halts exactly as `while true {}`
  does; runaway recursion that needs its frames back still meets the
  cap.
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
emitted bytecode. Instruction-count reduction is not pursued for its own
sake, on recorded evidence: fusing compare+branch pairs retired 3.5% of
executed instructions and ran 3–4% *slower*, because growing the
instruction set perturbs the dispatch loop's code layout more than the
saved dispatches earn back. The dispatch loop's performance is a property
of its shape, and changes to it are judged by measurement, not by
instruction arithmetic.

## The JIT

The third tier (`src/ovm/jit.rs`): hot bytecode compiled to native
machine code via Cranelift, extending the correctness ladder unchanged.
"Can't compile identically → stay interpreted" has a second rung:
"can't compile natively → stay on bytecode".

A function prequalifies at promotion time when every instruction falls
in a **pure whitelist**: arithmetic, comparisons, logic, branches,
calls to other olang functions, return, the float `math.*` builtins,
struct field reads and construction, list indexing and `for` iteration,
tuple returns, and strings. Compilation is
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
Everything else stays on bytecode with zero overhead beyond one table
lookup per call.

Heap values cross into native code on a **borrowed-pointer discipline**
(JIT calls are synchronous and the caller's slot outlives the call, so
no refcount is ever touched on the way in):

- **Struct field access**: struct arguments pass as borrowed pointers,
  specialized per interned shape with field indices resolved at compile
  time; every read goes through one guarded host helper that deopts on
  any surprise, so no layout assumption leaks into the VM.
- **The float `math` builtins**: all 25 compile — `sqrt`, `floor`,
  `ceil`, and `trunc` as native IEEE instructions (bit-exact by
  definition), the other 21 through an imported helper that calls the
  VM's own float-math evaluator — exactness by construction, not by
  reimplementation.
- **List indexing, construction, and `for` iteration**: lists enter
  native code as borrowed pointers, classified at specialization by
  element kind (int, float, one struct shape, or string); `xs[i]`, list
  literals, concatenation, and the iteration instructions run through
  guarded host helpers reproducing the VM's exact semantics —
  subscripts wrap negative indices, iteration does not, bounds
  violations deopt to bytecode's canonical error. Built lists follow
  the scratch discipline and can be returned, with ownership
  transferred once at the entry boundary.
- **Results and maps**: `Ok`/`Err` values and string-keyed maps carry
  their payload kind in the specialization (int, float, bool, or
  string), so Result-speaking helpers and map-reading kernels compile;
  construction and access run through the same guarded-helper,
  scratch-owned model as everything else.
- **Tuple returns**: functions returning tuples of up to four scalar
  elements become multi-value native returns; destructuring callers
  receive the elements directly in registers, and a deopt deep in a
  tuple-returning callee still yields the VM's canonical error.
- **Struct construction**: `MakeStruct` compiles under the
  [scratch-ownership model](internals.md#the-scratch-ownership-model) —
  every struct a native call builds is owned by a VM-side list for
  exactly that call, so a deopt at any point can never leak or dangle;
  struct returns transfer ownership once, at the entry boundary, where
  a retain helper resolves the borrowed pointer to an owned Arc.
- **Strings**: parameters, fields, and constants enter as borrowed
  pointers; equality and lexicographic ordering run through a helper
  executing the VM's own comparison operators; concatenation is an
  allocation and follows the struct discipline exactly.

Allocation inside loops is governed by the **scratch watermark**. The
scratch model owns every value a native call builds, which naively
means an allocating loop holds all its garbage until the call returns
— so early versions simply refused to compile them. The watermark
lifts that: when liveness analysis proves that no heap value built in
the loop body survives into the next iteration, the compiled loop
records the scratch depth at entry and truncates back to it at every
taken back-edge — per-iteration values die per iteration, exactly as
they would on bytecode. Loops whose allocations *do* cross iterations
(accumulators that grow a list, say) still refuse and drive the native
constructors call by call. One deliberate refusal rule remains: mixed
string/number `+` (formatting) stays on bytecode.

Before any of that, the plan pass rewrites the bytecode it was handed
(a clone — the VM's own bytecode is untouched, so a deopt re-runs the
original): small call-free callees (≤24 instructions) **inline** at
their call sites, single-definition copies propagate, and a struct or
constant-index list that never escapes dissolves into its fields —
**scalar replacement** — so a `pt(x, y)`-style helper in a hot loop
costs nothing at all: no call, no allocation, just registers. These
transforms are what let idiomatic small-aggregate code run at
hand-flattened speed.

Purity is the load-bearing property. A qualifying function has no side
effects, so every guard failure — argument-kind mismatch at entry,
integer overflow, division by zero (integer *and* float — olang errors
there rather than producing inf), `i64::MIN` edge cases, recursion-depth
exhaustion — simply **deopts**: the native run is abandoned and the same
call re-executes on bytecode, which produces the exact result or error
the VM would have produced anyway. The JIT never reproduces an error
message; it only declines. Float modulo, for instance, is declined outright,
because `fmod` has no exact equivalent in the JIT's intermediate
representation and an approximation could diverge from the interpreter. Every
native call — self- or cross-function —
carries a depth budget clamped to the VM's own `max_call_depth`, so
runaway recursion errors exactly as it does on bytecode instead of
overflowing the native stack, and a deopt anywhere in a native call
chain unwinds the whole chain back to the bytecode entry.

What this buys, measured: fib(30) 89 ms → **4 ms** (level with the
JavaScript JITs) — and fib split across two mutually recursive
functions runs at the same speed, where a self-call-only design managed
94 ms; N-body — structs, floats, lists, tuples, and `math.sqrt` in a
hot loop — 400 ms → **26 ms**, its force loops compiling whole; integer
loop kernels 20–30×; float kernels (Mandelbrot-style orbit loops)
~4.5×; a struct-field kernel 143 ms → 18 ms (8×); and `par_map` (or
`par for`) over a jitted kernel multiplies further — workers carry
their own tier and JIT.

`tests/jit_test.rs` holds the parity suite: every guard edge runs tiered
and interpreted and must agree byte-for-byte.

## Compile-time knowledge

Two things the compiler knows for the whole run, exploited by default.

**Capabilities.** Under a static manifest (`--deny`, or a
`[capabilities]` block), the grant is fixed and attenuates by each
function's provenance — which every compiled function carries. So
`caps.allowed("net")` with a literal name compiles to a Boolean
constant, the branch it guards folds to a plain jump, and an
unreachable-code sweep retires the side that can never run. The
practical consequence inverts the usual sandbox trade: the degradation
branch a program writes for the denied case vanishes from the compiled
function, so its calls stop counting against JIT qualification of the
code that does run — sandboxed code gets faster, not slower. Gated
builtin calls the manifest provably grants also skip the runtime
gate's per-call table walk; denied calls keep the full gate, whose
error message is the point, and `--trace-caps` recording is untouched.

**Warm start.** A finished file run records which named functions ran
native, on which scalar kinds, and how often, keyed by a hash of the
source (`~/.olang/warm/`). The next run of byte-identical source
compiles and specializes those functions at declaration time instead
of at first call. Profiles are hints: the tier's qualification and
specialization run exactly as they would have, just earlier, so a
stale or corrupted profile can cost a wasted attempt and nothing else.
`OLANG_WARM=0` disables it for measurement.

**Tier counters.** `OLANG_TIER_STATS=1` prints the run's tier counters
to stderr after a successful `olang run`: one aggregate line
(`tier-stats: promoted=… rejected=… bytecode_calls=… instructions=…
native_calls=…`) and one `tier-fn:` line per VM-callable function with
the native calls its JIT entry served and its specialization kinds.
The counters are deterministic — call counts, not samples — which is
what makes them assertable: `tests/tier_floor_test.rs` runs
representative hot shapes under this flag and fails if they stop
reaching the compiled tiers, so a regression in tier *engagement*
turns a test red instead of surfacing later as unexplained slowness. A
function listed with `native_calls=0` is the interesting row: compiled,
but every call declined to native.

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
is what environment-threading code lives on, so these ten earn their
second implementation and are held to the differential suite like
everything else.

The enabled set spans the core builtins below, plus the pure functions
of `math` (33 of its 42) and all 30 of `str`, plus `show`:

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

The higher-order builtins work because compiled lambdas carry closures:
a lambda the VM builds itself can be handed to a delegated builtin
as a function value. `map`, `filter`, and `sum` go further: when the
collection is a list and the function argument compiles (checked once and
cached per function value), the loop runs *inside* the VM — one bytecode
call per element, no conversion at the boundary — and a mapped list flows
into `sum` without leaving the VM's value model. Anything declined bridges
to the interpreter, which stays the semantic authority; a native loop never
falls back mid-flight, so element errors propagate exactly as the
interpreter would.

`BytecodeVm::round_trips` is the single definition of which values survive
the boundary; the tier uses it to decide whether a call's arguments and
result can cross. It is deliberately the *only* copy of that knowledge —
a duplicate list in the tier is the kind of thing that silently drifts
(omit `Ok`/`Err` from it and every call passing a `Result` quietly falls
back to the interpreter, costing speed while changing nothing observable —
the hardest regression class to notice).

A user function shadows a builtin of the same name, matching the interpreter's
environment lookup: declaring `fn clamp(...)` makes calls to `clamp` resolve to
the user's definition in compiled code too.

## Performance

Measured on an Apple Silicon laptop, release build — best-of-3, inner
timings. All workloads are algorithm-identical across languages and
checksum-verified (the N-body sample position matches across every
implementation to the last digit). Since the tier is on by default, the
olang numbers are what a plain `olang program.ol` gets — no flags.

Four representative workloads against the field:

| Workload | Rust | Node | Bun | CPython | Ruby | **olang** |
|---|---|---|---|---|---|---|
| N-body (120 bodies × 150 steps) | 2.5 ms | 6 ms | 8 ms | 396 ms | 470 ms | **26 ms** |
| Word frequency (50k tokens × 20) | 5 ms | 16 ms | 12 ms | 15 ms | 61 ms | **26 ms** |
| `map(λ) \|> sum` pipeline, 1M elements | ~0 ms | 9 ms | 4 ms | 23 ms | 21 ms | **20 ms** |
| fib(30) (2.7M recursive calls) | 1.2 ms | 4 ms | 4 ms | 46 ms | 43 ms | **4 ms** |

(Interpreter-only mode runs the same programs at 23.6 s / — / 370 ms /
1.1 s; the word-frequency workload exceeds the interpreter's allocation
guard entirely, so the tier is what makes it runnable at this size.)

The shape of the result: on pure numeric work the JIT puts olang
**level with the JavaScript JITs** — fib(30) at 4 ms sits beside Node
and Bun's 4 ms and runs ~11× ahead of CPython and Ruby; the pipeline
workload (a jitted lambda inside the VM's native map loop) leads CPython
and Ruby. N-body — structs, floats, lists, tuples, and `math.sqrt` in a
hot loop — runs 15× ahead of CPython and 18× ahead of Ruby. The
remaining gap to Rust is the price of guards, boxing at tier
boundaries, and the deliberate refusal rules around allocation in
loops.

Against its own interpreter, the tiers are worth roughly 275× (fib,
bytecode + JIT) to ~900× (N-body): the whole N-body simulation —
construction, stepping, capturing lambdas, struct building, field
access, `math.sqrt` — runs as 6 promoted functions, 0 rejected, with 7
tier crossings.

Two later additions moved the remaining hot spots: the scratch
watermark plus scalar replacement put allocating kernels ahead of the
JavaScript JITs on the project's benchmark suite (a struct-building
kernel runs ~4× ahead of V8; list-building within striking distance),
and the VM's in-place `+=` string append (an `AddAssign` fusion the
compiler applies when the right-hand side cannot observe the target)
turned quadratic string building linear — a 150×-class win on
build-a-report workloads. Reproduce any of this with
[`olang bench`](tooling.md#olang-bench), which isolates each kernel in
a subprocess and compares runs against saved baselines.

`--ovm-stats` prints promotions, rejections, tier crossings, and
instructions retired; `--verbose` names each promoted or refused
function with the reason. To compare tiers on a program, run it twice —
`olang run file.ol` and `olang --no-ovm run file.ol` — and diff the
output; the differential test suites automate exactly this comparison
across the corpus.

`cargo bench` measures the interpreter itself (`benches/interpreter_bench.rs`,
ten representative programs). Use it when changing the evaluator; use
the differential suites when changing the VM.

## Known limitations

These are real boundaries, stated so you can predict them:

1. **Method calls on *effectful* receiver expressions stay interpreted.**
   `make_thing().m(..)` refuses because the interpreter's dispatch
   fallthrough re-evaluates the receiver, which the VM will not replicate
   for an expression with side effects. Pure receivers — locals, field
   chains, literals — compile.
2. ***Assigning* to a global is uncompiled** (reads bake as snapshot
   constants). Background tasks (`spawn`) and
   `par for` stay interpreted — the compiler refuses the parallel loop
   fail-closed; it is interpreter-owned by design.
3. **Bridged builtin calls cost a value round trip.** Builtins outside
   the native set convert arguments and results between the OVM and AST
   value models per call. The native set spans the higher-order
   loops (`map`/`filter`/`sum`) and the collection core (`len`, `head`,
   `tail`, `cons`, `concat`, `skip`, `map_get`, `map_set`,
   `map_has_key`, `entries`), which is what environment-threading and
   list-building code lives on; `reduce`, `fold`, and the rest bridge.

## Not implemented

- **Tracing garbage collection.** Memory is reference-counted; `gc.rs` holds
  only the safepoint flags the interpreter polls. olang values are
  immutable and acyclic, so there are no cycles to collect.
- **Full NaN-boxing.** The 8-byte value scheme's primitives are
  implemented and tested (`src/ovm/nanbox.rs`) but deliberately not
  wired — the measured verdict is recorded in the
  [CHANGELOG](../CHANGELOG.md).
- **Automatic SIMD vectorization** and pipeline fusion. If bulk numeric
  throughput matters, the honest route is explicit vectorized
  operations — which is what [the ods data stack](stdlib.md#ods--series-and-frames)
  provides — rather than a speculative auto-vectorizer inside the VM.
- **Adaptive optimization** — profiling feedback, on-stack replacement,
  polymorphic inline caches, speculation with side-exit deopt,
  feedback-driven inlining, background tiering. These are deliberate
  omissions, not accidental gaps. Under the current plan, adaptive
  optimization is deferred until after 1.0
  ([roadmap](roadmap.md#post-10-candidates)); a tracing JIT and a
  tracing garbage collector are rejected outright.

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
