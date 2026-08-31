# xlang — the cross-language benchmark suite

Eight benchmarks, nine languages, one protocol. Every benchmark
implements the same algorithm the same way in olang, C++, Rust, Go,
Java, JavaScript (Node), Lua, Python, and R, and prints two lines:

```
CHECK <value>
MS <milliseconds>
```

`CHECK` is a deterministic result the runner validates for agreement
across every language that finishes — a benchmark that computes the
wrong answer is not fast, it is wrong. `MS` is the program's own
timing of its measured section (a monotonic or CPU clock; startup,
toolchain warmup outside the section, and I/O are excluded the same
way everywhere). The runner medians the reps and prints one table.

```bash
benchmarks/xlang/run.sh [reps] [benchmarks...]
```

Defaults: 5 reps (a run slower than 20 s gets 1), all benchmarks, a
120 s timeout per run (`XLANG_TIMEOUT` overrides). A missing toolchain
skips its column; a timed-out run reports DNF.

## Protocol

- **Same algorithm, plain loops, natural containers.** Arrays/lists
  and hash maps as the language provides them; no vectorization
  libraries (no NumPy, no R vector tricks), no threads, no FFI. The
  point is to measure the language's own execution of the same
  scalar code. This is deliberately unflattering to Python and R,
  whose idiomatic numeric style is vectorized — that comparison is a
  different (also fair) question, answered for data pipelines by
  `benchmarks/run.sh` (ods vs pandas vs Polars).
- **String accumulation uses each language's blessed idiom**
  (StringBuilder in Java, strings.Builder in Go, table.concat in Lua,
  a vector + paste in R, `+=` elsewhere) — quadratic-append trivia is
  not the question strbuild asks.
- **Float determinism.** Implementations use explicit temporaries so
  no compiler may legally fuse multiply-adds (C++ additionally builds
  with `-ffp-contract=off`); with identical IEEE doubles in identical
  order, even the chaotic n-body checksum agrees bit-for-bit.
- **Randomness is a shared MINSTD LCG** (`seed = seed * 48271 mod
  2147483647`, seed 42) whose arithmetic stays exact in every
  language's number type, including JavaScript doubles.
- Compiled languages build with optimizations (`-O3` / `-O` /
  default release); olang runs its default tiered mode — the product
  as shipped, interpreter to bytecode VM to Cranelift JIT.

## The benchmarks

| Bench | Workload | What it stresses |
|---|---|---|
| fib | naive fib(32) | call overhead, recursion |
| collatz | total steps, 1..300 000 | tight integer while loops |
| sieve | Eratosthenes to 20 000 000 | bulk indexed writes |
| nbody | 5 bodies × 2 000 000 steps | float arithmetic, small-array read/write |
| matmul | 300×300 dense multiply | nested-container reads |
| strbuild | 2 000 000 appends | string accumulation |
| wordfreq | 3 000 000 words into a map | hash-map get/insert |
| kmeans | 200 000 points, k=10, 15 iters | the combined loop nest: reads, writes, compares |

For olang specifically the set was chosen to exercise the OVM's
optimization surface end to end: hot-loop promotion and OSR
(every bench), typed-list backing and native `col.set` writes
(sieve, nbody, kmeans), nested-list reads (matmul), the fused string
append (strbuild), native map builtins (wordfreq), guarded native
domain math (nbody, kmeans), and the self-call fast path (fib).
