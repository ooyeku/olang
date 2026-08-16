<script>
  import { onMount, onDestroy } from 'svelte';
  import { highlightOlang, TOKEN_COLORS, TOKEN_COLORS_LIGHT } from '$lib/olang-highlight.js';

  const EXAMPLES = [
    {
      name: "hello",
      blurb: "First steps: bindings, loops, and blocks as expressions.",
      code: `// Welcome. This is the real olang — the interpreter and its bytecode
// tier compiled to WebAssembly, running on your machine, not a server.
// Edit anything and press ⌘⏎ (or ctrl⏎).
println("hello from olang")

// \`let\` binds immutably; \`let mut\` is the reassignable kind.
let name = "playground"
let mut greeted = 0

for who in ["you", name, "the browser"] {
    println("hi, " + who)
    greeted = greeted + 1
}

println("")
println("greeted " + show(greeted) + " times")

// A block is an expression, so its last line is its value — and the
// last value of the whole program shows up under the output.
let answer = { let a = 6; let b = 7; a * b }
answer`,
    },
    {
      name: "pipelines",
      blurb: "Data flows left to right through filter, map, and sum.",
      code: `// Pipelines: data flows left to right, in execution order.
let sales = [
    #{ "region": "east", "amount": 120, "rep": "ada" },
    #{ "region": "west", "amount": 340, "rep": "grace" },
    #{ "region": "east", "amount": 88,  "rep": "alan" },
    #{ "region": "west", "amount": 512, "rep": "ada" },
    #{ "region": "east", "amount": 205, "rep": "grace" },
]

let big = sales
    |> filter((s) => map_get(s, "amount") > 100)
    |> map((s) => map_get(s, "rep") + ": " + show(map_get(s, "amount")))

for line in big { println(line) }

let total = sales |> map((s) => map_get(s, "amount")) |> sum
println("")
println("total: " + show(total))
println("mean:  " + show(to_float(total) / to_float(len(sales))))`,
    },
    {
      name: "pattern matching",
      blurb: "Enums that construct, match with guards, and one binding trap.",
      code: `// Enums construct, structs validate their shape, and match destructures
// with guards and ranges.
type Shape = enum { Circle(Float), Rect(Float, Float), Tri(Float, Float, Float) }

fn area(s) = match s {
    Circle(r) => 3.14159265 * r * r,
    Rect(w, h) => w * h,
    Tri(a, b, c) => {
        let p = (a + b + c) / 2.0
        math.sqrt(p * (p - a) * (p - b) * (p - c))
    }
}

fn size(s) = match area(s) {
    a if a < 5.0 => "small",
    a if a < 25.0 => "medium",
    a => "large"
}

let shapes = [Circle(1.0), Rect(3.0, 4.0), Tri(3.0, 4.0, 5.0), Circle(4.0)]

for s in shapes {
    let a = area(s)
    println(str.pad_end(show(s), 26, " ")
            + str.pad_start(show(to_float(math.round(a * 100.0)) / 100.0), 8, " ")
            + "  " + size(s))
}

println("")
println("total area: " + show(math.round(sum(map(shapes, area)))))

// A bare lowercase name in a match arm binds anything — including a
// mistyped variant. Declared unit variants are the exception.
type Level = enum { Debug, Info, Warn }
fn label(l) = match l { Debug => "dbg", Info => "inf", other => "?" + show(other) }
println(map([Debug, Info, Warn], label) |> join(" "))`,
    },
    {
      name: "structs + traits",
      blurb: "Validated shapes, and one call site with three implementations.",
      code: `// Structs validate their shape; traits give one name to many
// implementations, dispatched on the value's own type.
type Point = struct { x: Float, y: Float }
type Circle = struct { at: Point, r: Float }
type Box = struct { lo: Point, hi: Point }

trait Shape {
    fn area(self)
    fn name(self)
}

impl Shape for Circle {
    fn area(self) = 3.14159265 * self.r * self.r
    fn name(self) = "circle r=" + show(self.r)
}

impl Shape for Box {
    fn area(self) = (self.hi.x - self.lo.x) * (self.hi.y - self.lo.y)
    fn name(self) = "box " + show(self.hi.x - self.lo.x) + "x" + show(self.hi.y - self.lo.y)
}

let origin = Point { x: 0.0, y: 0.0 }
let shapes = [
    Circle { at: origin, r: 2.0 },
    Box { lo: origin, hi: Point { x: 3.0, y: 4.0 } },
    Circle { at: Point { x: 1.0, y: 1.0 }, r: 0.5 },
]

// One call site, three implementations.
for s in shapes {
    println(str.pad_end(s.name(), 18, " ")
            + str.pad_start(show(to_float(math.round(s.area() * 100.0)) / 100.0), 8, " "))
}

let total = shapes |> map((s) => s.area()) |> sum
println("")
println("total area " + show(to_float(math.round(total * 100.0)) / 100.0))

// A struct with the wrong shape is refused rather than silently built.
// Uncomment to see it:
//     Point { x: 1.0 }
//     => struct 'Point' is missing field 'y' (declared fields: x, y)`,
    },
    {
      name: "errors as values",
      blurb: "Result and ? for expected failure; a bug stops the program.",
      code: `// Expected failure travels as a Result; \`?\` short-circuits a chain of
// them. A bug, by contrast, stops the program — that distinction is the
// whole error model.
type Order = struct { id: Int, qty: Int, unit_cents: Int }

fn parse_field(row, key) = {
    if map_has_key(row, key) => str.parse_int(map_get(row, key))
    else => Err("missing field: " + key)
}

fn to_order(row) = {
    let id = parse_field(row, "id")?
    let qty = parse_field(row, "qty")?
    let unit = parse_field(row, "unit_cents")?
    if qty <= 0 => Err("order " + show(id) + ": qty must be positive")
    else => Ok(Order { id: id, qty: qty, unit_cents: unit })
}

fn money(cents) = "$" + show(cents / 100) + "." + str.pad_start(show(cents % 100), 2, "0")

let rows = [
    #{ "id": "1", "qty": "3", "unit_cents": "1250" },
    #{ "id": "2", "qty": "0", "unit_cents": "900" },
    #{ "id": "3", "unit_cents": "400" },
    #{ "id": "4", "qty": "12", "unit_cents": "75" },
]

let mut total = 0
for row in rows {
    match to_order(row) {
        Ok(o) => {
            let line = o.qty * o.unit_cents
            total = total + line
            println("ok    order " + show(o.id) + "  " + str.pad_start(show(o.qty), 3, " ")
                    + " x " + money(o.unit_cents) + " = " + money(line))
        },
        Err(e) => println("skip  " + e)
    }
}
println("")
println("banked: " + money(total))`,
    },
    {
      name: "the data stack",
      blurb: "Typed columns, group_by, and a correlation \u2014 no imports.",
      code: `// The data stack: typed columns, tables, and inference — built in, no
// import, and it runs right here in the browser.
let readings = [
    #{ "site": "north", "hour": 0, "temp": 12.4, "load": 310.0 },
    #{ "site": "north", "hour": 1, "temp": 11.8, "load": 288.0 },
    #{ "site": "north", "hour": 2, "temp": 11.1, "load": 301.0 },
    #{ "site": "south", "hour": 0, "temp": 18.9, "load": 455.0 },
    #{ "site": "south", "hour": 1, "temp": 19.4, "load": 470.0 },
    #{ "site": "south", "hour": 2, "temp": 20.2, "load": 512.0 },
]

let frame = ods.frame_from_records(readings)
println("frame: " + show(ods.n_rows(frame)) + " rows x " + show(ods.n_cols(frame)) + " cols")
println("columns: " + show(ods.columns(frame)))
println("")

fn r2(x) = to_float(math.round(x * 100.0)) / 100.0

// group_by aggregates whole columns at a time — no per-row loop.
let by_site = ods.group_by(frame, ["site"], [
    ["avg_temp", "mean", "temp"],
    ["peak_load", "max", "load"],
    ["total_load", "sum", "load"],
])

println(str.pad_end("site", 8, " ") + str.pad_start("avg temp", 10, " ")
        + str.pad_start("peak", 9, " ") + str.pad_start("total", 9, " "))
println(str.repeat("-", 36))
for row in ods.to_records(by_site) {
    println(str.pad_end(map_get(row, "site"), 8, " ")
        + str.pad_start(show(r2(map_get(row, "avg_temp"))), 10, " ")
        + str.pad_start(show(map_get(row, "peak_load")), 9, " ")
        + str.pad_start(show(map_get(row, "total_load")), 9, " "))
}

// A column is a typed vector; a correlation is one call.
let temps = ods.column(frame, "temp")
let loads = ods.column(frame, "load")
println("")
println("temp  mean " + show(r2(ods.mean(temps))) + "  sd " + show(r2(ods.std(temps))))
println("corr(temp, load) = " + show(r2(stats.corr(temps, loads))))`,
    },
    {
      name: "text to structure",
      blurb: "Regex captures, string handling, and a JSON round-trip.",
      code: `// Text in, structure out: regex captures, string handling, and JSON —
// the shape of most real scripts.
let log = "
2026-08-16T09:14:02Z INFO  api    GET  /orders     200  12ms
2026-08-16T09:14:03Z WARN  api    GET  /orders/99  404   4ms
2026-08-16T09:14:09Z ERROR db     POST /orders     500 812ms
2026-08-16T09:15:00Z INFO  api    POST /orders     201  31ms
"

let line_re = "(\\\\w+)\\\\s+(\\\\w+)\\\\s+(\\\\w+)\\\\s+(\\\\S+)\\\\s+(\\\\d{3})\\\\s+(\\\\d+)ms"

let mut events = []
for line in str.lines(str.trim(log)) {
    match re.captures(line_re, line) {
        Ok(c) => {
            events = events + [#{
                "level":  c[1],
                "system": c[2],
                "method": c[3],
                "route":  c[4],
                "status": unwrap(str.parse_int(c[5])),
                "ms":     unwrap(str.parse_int(c[6])),
            }]
        },
        Err(e) => println("unparsed: " + line)
    }
}

let slow = events |> filter((e) => map_get(e, "ms") > 20)
let bad = events |> filter((e) => map_get(e, "status") >= 400)

println("events:  " + show(len(events)))
println("slow:    " + show(len(slow)) + "  (" + (slow |> map((e) => map_get(e, "route")) |> join(", ")) + ")")
println("failing: " + show(len(bad)))
println("p-worst: " + show(max(events |> map((e) => map_get(e, "ms")))) + "ms")
println("")

// Straight out to JSON, and back in again.
let routes = col.unique(events |> map((e) => map_get(e, "route")))
let report = #{ "total": len(events), "failing": len(bad), "routes": routes }
let text = unwrap(json.stringify(report))
println(text)
let back = unwrap(json.parse(text))
println("round-trips: " + show(map_get(back, "total") == len(events)))`,
    },
    {
      name: "the program as data",
      blurb: "meta.parse walks a program's own syntax tree.",
      code: `// The Open Language: a program's structure is a public data format.
// meta.parse hands you the syntax tree as ordinary olang values, so a
// linter is a pipeline — not a compiler change.
let src = "
fn tally(xs) = fold(xs, 0, (a, b) => a + b)
let nums = [1, 2, 3, 4]
let mut total = 0
for n in nums { total = total + n }
println(show(tally(nums)) + \\" \\" + show(total))
"

let program = unwrap(meta.parse(src))

// Recurse into every nested node, counting what each one is.
fn count_kinds(node, acc) = {
    let mut out = acc
    if typeof(node) == "List" => {
        for item in node { out = count_kinds(item, out) }
    }
    if typeof(node) == "Map" => {
        if map_has_key(node, "kind") => {
            let k = map_get(node, "kind")
            let n = if map_has_key(out, k) => map_get(out, k) else => 0
            out = map_set(out, k, n + 1)
        }
        for (key, v) in entries(node) { out = count_kinds(v, out) }
    }
    out
}

let kinds = count_kinds(program, #{})
println("node kinds in the program:")
for (kind, n) in entries(kinds) {
    println("  " + str.pad_end(kind, 12, " ") + show(n))
}

println("")
let names = program
    |> filter((n) => map_get(n, "kind") == "fn")
    |> map((n) => map_get(n, "name"))
println("functions declared: " + show(names))`,
    },
    {
      name: "state and capture",
      blurb: "Closures capture by value; cell is the one mutable location.",
      code: `// Values are immutable and closures capture by value — which is what
// makes olang's parallelism lock-free. \`cell\` is the one deliberate
// exception: a mutable location, confined to its own thread.

// A closure keeps its own snapshot, so rebinding never reaches inside.
let base = 10
let add_base = (n) => n + base
let base = 99
println("closure kept its snapshot: " + show(add_base(5)))   // 15, not 104

// Assigning to a captured binding is refused before the program runs:
//     let mut hits = 0
//     let bump = () => { hits = hits + 1 }   // error, not a dead write
// The alternative the error points at:
let hits = cell(0)
let bump = () => cell.update(hits, (n) => n + 1)
bump()
bump()
bump()
println("cell after three bumps: " + show(cell.get(hits)))

// A cell is a *location*, so binding it to another name aliases it.
let same = hits
cell.set(same, 100)
println("aliased: " + show(cell.get(hits)))

// Where you can, prefer returning the new value — a fold says the same
// thing with no mutable state at all.
let events = ["add", "add", "remove", "add", "remove"]
let net = events |> fold(0, (n, e) => if e == "add" => n + 1 else => n - 1)
println("net via fold: " + show(net))

let tally = events |> fold(#{}, (acc, e) =>
    map_set(acc, e, (if map_has_key(acc, e) => map_get(acc, e) else => 0) + 1))
for (event, n) in entries(tally) { println("  " + event + " x" + show(n)) }`,
    },
    {
      name: "tiered execution",
      blurb: "Hot functions promote to bytecode automatically. Watch the clock.",
      code: `// Hot functions promote to register bytecode automatically. Nothing here
// asks for it; the tier is an optimization, never a semantic.
fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)

fn collatz_len(n) = {
    let mut steps = 0
    let mut x = n
    while x != 1 {
        x = if x % 2 == 0 => x / 2 else => 3 * x + 1
        steps = steps + 1
    }
    steps
}

let t0 = time.monotonic_ms()
let f = fib(30)
let t1 = time.monotonic_ms()
println("fib(30) = " + show(f) + "  in " + show(t1 - t0) + "ms")

let t2 = time.monotonic_ms()
let longest = col.max_by(1..150000 |> map((n) => #{ "n": n, "steps": collatz_len(n) }), (r) => map_get(r, "steps"))
let t3 = time.monotonic_ms()
println("longest Collatz under 150000: n=" + show(map_get(longest, "n")) + " takes "
        + show(map_get(longest, "steps")) + " steps  in " + show(t3 - t2) + "ms")

// The same functions, checked against a slower obvious version.
fn fib_iter(n) = {
    let mut a = 0
    let mut b = 1
    for i in 0..n { let next = a + b; a = b; b = next }
    a
}
println("")
println("recursive and iterative agree: " + show(fib(30) == fib_iter(30)))`,
    },
  ];
  // 10s: long enough for the tiered-execution example and a real
  // workload, short enough that a runaway loop does not hang the tab.
  const TIMEOUT_MS = 10000;

  let selected = EXAMPLES[0].name;
  let source = EXAMPLES[0].code;
  let output = '';
  let value = null;
  let error = null;
  let crash = null;
  let ms = null;
  let version = '';
  let running = false;
  let ranOnce = false;
  let booting = true;
  let copied = false;
  let elapsed = 0;

  let worker = null;
  let runId = 0;
  let timeoutHandle = null;
  let tickHandle = null;
  let startedAt = 0;

  $: current = EXAMPLES.find((e) => e.name === selected) ?? EXAMPLES[0];
  $: dirty = current && source !== current.code;
  $: lineCount = source.split('\n').length;
  $: gutter = Array.from({ length: lineCount }, (_, i) => i + 1).join('\n');
  $: painted = highlightOlang(source);

  // Both palettes ride on the wrapper; the stylesheet below picks one by
  // theme. Emitting only the active set would need the theme in component
  // state, which is a second source of truth for something CSS already
  // knows.
  const tokenStyle = [
    ...Object.entries(TOKEN_COLORS).map(([k, v]) => `${k}:${v}`),
    ...Object.entries(TOKEN_COLORS_LIGHT).map(([k, v]) => `${k}-lt:${v}`),
  ].join(';');

  function spawnWorker() {
    worker?.terminate();
    worker = new Worker('/playground/worker.js');
    worker.onmessage = (event) => {
      const { id, result, crash: crashed, ready } = event.data;
      if (ready) {
        booting = false;
        return;
      }
      if (id !== runId) return;
      settle();
      if (crashed) {
        crash = crashed;
        output = '';
        value = null;
        error = null;
        spawnWorker(); // a trap poisons the instance
        return;
      }
      crash = null;
      output = result.output;
      value = result.value;
      error = result.error;
      ms = result.ms;
      version = result.version;
    };
  }

  function settle() {
    clearTimeout(timeoutHandle);
    clearInterval(tickHandle);
    running = false;
    ranOnce = true;
  }

  function run() {
    if (running || !worker) return;
    running = true;
    booting = false;
    runId += 1;
    startedAt = performance.now();
    elapsed = 0;
    tickHandle = setInterval(() => (elapsed = (performance.now() - startedAt) / 1000), 100);
    worker.postMessage({ id: runId, source });
    timeoutHandle = setTimeout(() => {
      if (!running) return;
      settle();
      crash = `stopped after ${TIMEOUT_MS / 1000}s — the sandbox bounds a run so a loop cannot hang the tab`;
      output = '';
      value = null;
      error = null;
      spawnWorker();
    }, TIMEOUT_MS);
  }

  function stop() {
    if (!running) return;
    settle();
    crash = 'stopped';
    output = '';
    value = null;
    error = null;
    spawnWorker();
  }

  // Look the example up from `selected` rather than the reactive
  // `current`: bind:value updates `selected` before this handler runs,
  // but the derivation has not recomputed yet, so `current` is still the
  // example we just navigated away from.
  function pickExample() {
    const ex = EXAMPLES.find((e) => e.name === selected);
    if (!ex) return;
    source = ex.code;
    crash = null;
    resetScroll();
  }

  /** A new program starts at the top; the layers must agree on that too. */
  function resetScroll() {
    for (const el of [gutterEl, paintEl]) {
      if (el) {
        el.scrollTop = 0;
        el.scrollLeft = 0;
      }
    }
  }

  function reset() {
    const ex = EXAMPLES.find((e) => e.name === selected);
    if (ex) source = ex.code;
  }

  function copy() {
    navigator.clipboard?.writeText(source);
    copied = true;
    setTimeout(() => (copied = false), 1400);
  }

  function onKeydown(event) {
    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault();
      run();
      return;
    }
    if (event.key === 'Tab') {
      event.preventDefault();
      const el = event.target;
      const { selectionStart: s, selectionEnd: e, scrollTop, scrollLeft } = el;
      const next = source.slice(0, s) + '    ' + source.slice(e);
      source = next;
      // Write through and place the caret in the same turn. Deferring to
      // a frame would leave the caret at the end of the document whenever
      // rAF is throttled — a background tab, or a reduced-motion setting.
      el.value = next;
      el.setSelectionRange(s + 4, s + 4);
      el.scrollTop = scrollTop;
      el.scrollLeft = scrollLeft;
      syncScroll({ target: el });
    }
  }

  // The textarea owns the scroll; the gutter follows it vertically and
  // the painted layer follows on both axes.
  let gutterEl;
  let paintEl;
  function syncScroll(event) {
    const { scrollTop, scrollLeft } = event.target;
    if (gutterEl) gutterEl.scrollTop = scrollTop;
    if (paintEl) {
      paintEl.scrollTop = scrollTop;
      paintEl.scrollLeft = scrollLeft;
    }
  }

  onMount(spawnWorker);
  onDestroy(() => {
    worker?.terminate();
    clearTimeout(timeoutHandle);
    clearInterval(tickHandle);
  });
</script>

<svelte:head>
  <title>playground — olang</title>
  <meta
    name="description"
    content="Run olang in your browser. The whole language — interpreter and bytecode tier — compiled to WebAssembly, sandboxed on your machine."
  />
</svelte:head>

<main class="container playground">
  <header class="intro">
    <div>
      <h1>playground</h1>
      <p>
        The real thing — the interpreter and its bytecode tier compiled to
        WebAssembly, running entirely in your browser. Nothing you type leaves
        the page.
      </p>
    </div>
    <ul class="sandbox">
      <li><span>no i/o</span> <code>fs</code>, <code>http</code>, <code>os</code>, <code>db</code> are native-only</li>
      <li><span>no threads</span> <code>spawn</code> needs an OS thread; <code>par_map</code> runs sequentially</li>
      <li><span>bounded</span> a run is cut off after {TIMEOUT_MS / 1000} seconds</li>
    </ul>
  </header>

  <div class="toolbar">
    <label class="picker">
      <span class="visually-hidden">example programs</span>
      <select bind:value={selected} on:change={pickExample}>
        {#each EXAMPLES as ex}
          <option value={ex.name}>{ex.name}</option>
        {/each}
      </select>
    </label>

    {#if running}
      <button class="run stop" on:click={stop}>stop ■</button>
    {:else}
      <button class="run" on:click={run} disabled={booting}>
        {booting ? 'loading wasm…' : 'run ▸'}
      </button>
    {/if}
    <span class="hint">⌘⏎</span>

    <div class="spacer"></div>
    {#if dirty}
      <button class="ghost" on:click={reset}>reset</button>
    {/if}
    <button class="ghost" on:click={copy}>{copied ? 'copied' : 'copy'}</button>
  </div>

  <p class="blurb">{current.blurb}</p>

  <div class="panes">
    <div class="editor-wrap" style={tokenStyle}>
      <pre class="gutter" bind:this={gutterEl} aria-hidden="true">{gutter}</pre>
      <!-- The painted copy sits under a textarea whose own text is
           transparent, so the caret, selection, and editing behaviour are
           the browser's while the colour is ours. Both layers share the
           same font metrics, padding, and `white-space: pre`, so they
           line up character for character. -->
      <div class="code">
        <pre class="paint" bind:this={paintEl} aria-hidden="true">{@html painted}</pre>
        <textarea
          class="editor"
          bind:value={source}
          on:keydown={onKeydown}
          on:scroll={syncScroll}
          spellcheck="false"
          autocomplete="off"
          autocapitalize="off"
          autocorrect="off"
          aria-label="olang source"
        ></textarea>
      </div>
    </div>

    <section class="result" class:empty={!ranOnce && !running}>
      <div class="result-head">
        <span class="title">output</span>
        {#if running}
          <span class="status busy">running {elapsed.toFixed(1)}s</span>
        {:else if ranOnce && crash}
          <span class="status bad">stopped</span>
        {:else if ranOnce && error}
          <span class="status bad">error</span>
        {:else if ranOnce}
          <span class="status ok">ok</span>
        {/if}
      </div>

      <div class="result-body">
        {#if !ranOnce && !running}
          <p class="placeholder">Press <b>run</b> — or ⌘⏎ — and the output lands here.</p>
        {:else if running}
          <p class="placeholder">running…</p>
        {:else if crash}
          <pre class="crash">{crash}</pre>
        {:else}
          {#if output}<pre class="stdout">{output}</pre>{/if}
          {#if error}<pre class="err">{error}</pre>{/if}
          {#if value != null}<pre class="value">=&gt; {value}</pre>{/if}
          {#if !output && !error && value == null}
            <p class="placeholder">(the program printed nothing)</p>
          {/if}
        {/if}
      </div>

      {#if ranOnce && !running && !crash}
        <div class="meta">
          {#if ms != null}<span>{ms} ms</span>{/if}
          {#if version}<span>olang {version}</span><span>wasm</span>{/if}
        </div>
      {/if}
    </section>
  </div>

  <p class="footnote">
    Everything here is documented in <a href="/book/language">the language
    reference</a>; the sandbox itself is described in
    <a href="/book/wasm">olang in the browser</a>.
  </p>
</main>

<style>
  .playground { padding: 2.4rem 1.5rem 4rem; }

  .intro {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 2.5rem;
    align-items: start;
    margin-bottom: 1.8rem;
  }
  .intro h1 {
    color: var(--text);
    font-family: var(--mono);
    font-size: 1.9rem;
    margin: 0 0 0.5rem;
    letter-spacing: -0.01em;
  }
  .intro p { max-width: 40rem; margin: 0; }

  .sandbox {
    list-style: none;
    margin: 0.2rem 0 0;
    padding: 0.85rem 1rem;
    display: grid;
    gap: 0.4rem;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-2);
    font-size: 0.82rem;
    color: var(--text-3);
    min-width: 21rem;
  }
  .sandbox span {
    font-family: var(--mono);
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--accent);
    margin-right: 0.5rem;
  }
  .sandbox code { font-family: var(--mono); color: var(--text-2); font-size: 0.92em; }

  @media (max-width: 900px) {
    .intro { grid-template-columns: 1fr; gap: 1.4rem; }
    .sandbox { min-width: 0; }
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.7rem;
    margin-bottom: 0.7rem;
    flex-wrap: wrap;
  }
  .toolbar .spacer { flex: 1 1 auto; }
  .picker select {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--line-strong);
    border-radius: var(--radius);
    font-family: var(--mono);
    font-size: 0.88rem;
    padding: 0.5rem 0.7rem;
    cursor: pointer;
  }
  .picker select:hover { border-color: var(--text-3); }
  .run {
    background: var(--accent);
    color: #04211c;
    border: 1px solid transparent;
    border-radius: var(--radius);
    font-family: var(--mono);
    font-weight: 700;
    font-size: 0.9rem;
    padding: 0.5rem 1.15rem;
    cursor: pointer;
    transition: background var(--fast) var(--ease);
  }
  .run:hover:not(:disabled) { background: var(--accent-strong); }
  .run:disabled { opacity: 0.5; cursor: progress; }
  .run.stop { background: var(--danger); color: #2b0b0b; }
  .ghost {
    background: none;
    border: 1px solid var(--line-strong);
    color: var(--text-3);
    border-radius: var(--radius);
    font-family: var(--mono);
    font-size: 0.8rem;
    padding: 0.42rem 0.75rem;
    cursor: pointer;
    transition: color var(--fast) var(--ease), border-color var(--fast) var(--ease);
  }
  .ghost:hover { color: var(--accent); border-color: var(--accent-strong); }
  .hint { font-family: var(--mono); font-size: 0.78rem; color: var(--text-4); }

  .blurb {
    margin: 0 0 0.9rem;
    font-size: 0.88rem;
    color: var(--text-3);
  }

  .panes {
    display: grid;
    grid-template-columns: minmax(0, 1.15fr) minmax(0, 0.85fr);
    gap: 0.9rem;
    /* Fill what is left of the viewport rather than guessing a height. */
    height: clamp(28rem, calc(100vh - 22rem), 46rem);
  }
  @media (max-width: 900px) {
    .panes { grid-template-columns: 1fr; height: auto; }
    .editor-wrap, .result { height: 28rem; }
  }
  @media (max-width: 640px) {
    /* Without the spacer the row wraps naturally; with it, `copy` gets
       pushed onto a line of its own. */
    .toolbar .spacer { display: none; }
  }

  /* editor: a gutter, a painted layer, and a transparent textarea, all
     sharing one box and one scroll position */
  .editor-wrap {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    overflow: hidden;
    min-height: 0;
    transition: border-color var(--fast) var(--ease);
  }
  .editor-wrap:focus-within { border-color: var(--accent-strong); }

  .gutter {
    margin: 0;
    padding: 1rem 0.7rem 1rem 1rem;
    overflow: hidden;
    text-align: right;
    color: var(--text-4);
    background: rgba(0, 0, 0, 0.18);
    border-right: 1px solid var(--line);
    user-select: none;
    font-family: var(--mono);
    font-size: 0.88rem;
    line-height: 1.62;
    font-variant-numeric: tabular-nums;
  }

  .code { position: relative; min-width: 0; }

  /* Every metric below this line must match between .paint and .editor,
     or the coloured text drifts out from under the caret. */
  .paint,
  .editor {
    position: absolute;
    inset: 0;
    margin: 0;
    border: 0;
    padding: 1rem 1.1rem;
    font-family: var(--mono);
    font-size: 0.88rem;
    line-height: 1.62;
    letter-spacing: normal;
    tab-size: 4;
    white-space: pre;
    word-spacing: normal;
    text-rendering: auto;
    font-variant-ligatures: none;
  }
  .paint {
    overflow: hidden;
    pointer-events: none;
    color: var(--tok-plain);
    /* A scrollbar on the textarea steals width; the painted layer keeps
       the same content box so columns still line up. */
    scrollbar-gutter: stable;
  }
  .editor {
    background: transparent;
    color: transparent;
    -webkit-text-fill-color: transparent;
    caret-color: var(--text);
    resize: none;
    overflow: auto;
    scrollbar-gutter: stable;
  }
  .editor:focus { outline: none; }
  .editor::selection { background: rgba(45, 212, 191, 0.28); }

  /* token colours, fed from the same theme the book's code blocks use */
  .paint :global(.t-comment) { color: var(--tok-comment); font-style: italic; }
  .paint :global(.t-string) { color: var(--tok-string); }
  .paint :global(.t-escape) { color: var(--tok-escape); }
  .paint :global(.t-number) { color: var(--tok-number); }
  .paint :global(.t-keyword) { color: var(--tok-keyword); }
  .paint :global(.t-const) { color: var(--tok-const); font-weight: 700; }
  .paint :global(.t-type) { color: var(--tok-type); }
  .paint :global(.t-fn) { color: var(--tok-fn); }
  .paint :global(.t-pipe) { color: var(--tok-pipe); font-weight: 700; }
  .paint :global(.t-arrow) { color: var(--tok-arrow); }
  .paint :global(.t-op) { color: var(--tok-op); }

  /* light: the OS preference, unless the reader overrode it */
  @media (prefers-color-scheme: light) {
    :global(:root:not([data-theme='dark'])) .paint :global(.t-comment) { color: var(--tok-comment-lt); }
    :global(:root:not([data-theme='dark'])) .paint :global(.t-string) { color: var(--tok-string-lt); }
    :global(:root:not([data-theme='dark'])) .paint :global(.t-escape) { color: var(--tok-escape-lt); }
    :global(:root:not([data-theme='dark'])) .paint :global(.t-number) { color: var(--tok-number-lt); }
    :global(:root:not([data-theme='dark'])) .paint :global(.t-keyword) { color: var(--tok-keyword-lt); }
    :global(:root:not([data-theme='dark'])) .paint :global(.t-const) { color: var(--tok-const-lt); }
    :global(:root:not([data-theme='dark'])) .paint :global(.t-type) { color: var(--tok-type-lt); }
    :global(:root:not([data-theme='dark'])) .paint :global(.t-fn) { color: var(--tok-fn-lt); }
    :global(:root:not([data-theme='dark'])) .paint :global(.t-pipe) { color: var(--tok-pipe-lt); }
    :global(:root:not([data-theme='dark'])) .paint :global(.t-arrow) { color: var(--tok-arrow-lt); }
    :global(:root:not([data-theme='dark'])) .paint :global(.t-op) { color: var(--tok-op-lt); }
  }
  @media (prefers-color-scheme: light) {
    :global(:root:not([data-theme='dark'])) .paint { color: var(--tok-plain-lt); }
  }
  :global(:root[data-theme='light']) .paint { color: var(--tok-plain-lt); }
  /* and an explicit light pick, which wins in either OS scheme */
  :global(:root[data-theme='light']) .paint :global(.t-comment) { color: var(--tok-comment-lt); }
  :global(:root[data-theme='light']) .paint :global(.t-string) { color: var(--tok-string-lt); }
  :global(:root[data-theme='light']) .paint :global(.t-escape) { color: var(--tok-escape-lt); }
  :global(:root[data-theme='light']) .paint :global(.t-number) { color: var(--tok-number-lt); }
  :global(:root[data-theme='light']) .paint :global(.t-keyword) { color: var(--tok-keyword-lt); }
  :global(:root[data-theme='light']) .paint :global(.t-const) { color: var(--tok-const-lt); }
  :global(:root[data-theme='light']) .paint :global(.t-type) { color: var(--tok-type-lt); }
  :global(:root[data-theme='light']) .paint :global(.t-fn) { color: var(--tok-fn-lt); }
  :global(:root[data-theme='light']) .paint :global(.t-pipe) { color: var(--tok-pipe-lt); }
  :global(:root[data-theme='light']) .paint :global(.t-arrow) { color: var(--tok-arrow-lt); }
  :global(:root[data-theme='light']) .paint :global(.t-op) { color: var(--tok-op-lt); }

  /* result */
  .result {
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }
  .result-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.5rem 0.9rem;
    border-bottom: 1px solid var(--line);
    background: var(--surface-2);
    font-family: var(--mono);
    font-size: 0.74rem;
    text-transform: uppercase;
    letter-spacing: 0.1em;
  }
  .result-head .title { color: var(--text-3); }
  .status { letter-spacing: 0.06em; }
  .status.ok { color: var(--accent); }
  .status.bad { color: var(--danger); }
  .status.busy { color: var(--text-3); }
  .result-body {
    flex: 1 1 auto;
    overflow: auto;
    padding: 0.9rem 1.1rem;
    font-family: var(--mono);
    font-size: 0.86rem;
    line-height: 1.55;
  }
  .result-body pre {
    margin: 0 0 0.6rem;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .result-body pre:last-child { margin-bottom: 0; }
  .stdout { color: var(--text); }
  .value { color: var(--accent); }
  .err, .crash { color: var(--danger); }
  .placeholder { color: var(--text-4); margin: 0; font-family: var(--sans); font-size: 0.9rem; }
  .placeholder b { color: var(--text-2); }
  .meta {
    display: flex;
    gap: 1rem;
    padding: 0.5rem 1.1rem;
    border-top: 1px solid var(--line);
    color: var(--text-4);
    font-family: var(--mono);
    font-size: 0.74rem;
  }

  .footnote { margin: 1.4rem 0 0; font-size: 0.85rem; color: var(--text-3); }

  .visually-hidden {
    position: absolute;
    width: 1px; height: 1px;
    padding: 0; margin: -1px;
    overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0;
  }
</style>
