<script>
  import { onMount, onDestroy } from 'svelte';

  const EXAMPLES = [
    {
      name: 'hello',
      code: `println("hello from olang")\n\nlet who = "playground"\nprintln("running in the " + who)`,
    },
    {
      name: 'pipelines',
      code: `let sales = [120, 45, 310, 88, 260, 15, 190]\n\nlet big = sales\n    |> filter((x) => x > 100)\n    |> map((x) => x * 2)\n\nprintln("big sales, doubled: " + show(big))\nprintln("total: " + show(sum(big)))`,
    },
    {
      name: 'pattern matching',
      code: `type Shape = enum { Circle(Float), Rect(Float, Float) }\n\nfn area(s) = match s {\n    Circle(r) => 3.14159 * r * r,\n    Rect(w, h) => w * h\n}\n\nlet shapes = [Circle(1.0), Rect(3.0, 4.0), Circle(2.5)]\nfor s in shapes {\n    println(show(s) + " -> " + show(area(s)))\n}`,
    },
    {
      name: 'structs + traits',
      code: `type Point = struct { x: Int, y: Int }\n\ntrait Show {\n    fn describe(self)\n}\n\nimpl Show for Point {\n    fn describe(self) = "Point(" + show(self.x) + ", " + show(self.y) + ")"\n}\n\nlet p = Point { x: 3, y: 7 }\nprintln(p.describe())`,
    },
    {
      name: 'recursion (tiered)',
      code: `// This promotes to the bytecode tier on first call.\nfn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)\n\nlet t0 = time.monotonic_ms()\nlet result = fib(27)\nlet elapsed = time.monotonic_ms() - t0\n\nprintln("fib(27) = " + show(result) + " in " + show(elapsed) + "ms")`,
    },
    {
      name: 'maps',
      code: `let counts = #{}\nlet words = ["the", "quick", "the", "fox", "the", "quick"]\n\nlet mut tally = counts\nfor w in words {\n    let n = if map_has_key(tally, w) => map_get(tally, w) else => 0\n    tally = map_set(tally, w, n + 1)\n}\n\nfor (word, n) in entries(tally) {\n    println(word + ": " + show(n))\n}`,
    },
  ];

  let source = EXAMPLES[0].code;
  let selected = EXAMPLES[0].name;
  let output = '';
  let value = null;
  let error = null;
  let crash = null;
  let ms = null;
  let version = '';
  let running = false;
  let ranOnce = false;

  const TIMEOUT_MS = 5000;
  let worker = null;
  let runId = 0;
  let timeoutHandle = null;

  function spawnWorker() {
    worker?.terminate();
    worker = new Worker('/playground/worker.js');
    worker.onmessage = (event) => {
      const { id, result, crash: crashed } = event.data;
      if (id !== runId) return;
      clearTimeout(timeoutHandle);
      running = false;
      ranOnce = true;
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

  function run() {
    if (running || !worker) return;
    running = true;
    runId += 1;
    const id = runId;
    worker.postMessage({ id, source });
    timeoutHandle = setTimeout(() => {
      if (!running) return;
      running = false;
      ranOnce = true;
      crash = `timed out after ${TIMEOUT_MS / 1000}s — the worker was terminated`;
      output = '';
      value = null;
      error = null;
      spawnWorker();
    }, TIMEOUT_MS);
  }

  function pickExample() {
    const ex = EXAMPLES.find((e) => e.name === selected);
    if (ex) source = ex.code;
  }

  function onKeydown(event) {
    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault();
      run();
    }
    if (event.key === 'Tab') {
      event.preventDefault();
      const el = event.target;
      const { selectionStart: s, selectionEnd: e } = el;
      source = source.slice(0, s) + '    ' + source.slice(e);
      requestAnimationFrame(() => el.setSelectionRange(s + 4, s + 4));
    }
  }

  onMount(spawnWorker);
  onDestroy(() => worker?.terminate());
</script>

<svelte:head>
  <title>playground — olang</title>
  <meta
    name="description"
    content="Run olang in your browser. The whole language — interpreter and bytecode tier — compiled to WebAssembly, sandboxed on your machine."
  />
</svelte:head>

<main class="container playground">
  <div class="intro">
    <h1>playground</h1>
    <p>
      This is the real thing — the interpreter and its bytecode tier compiled to
      WebAssembly, running entirely in your browser. Nothing you type leaves the
      page: the sandbox has no filesystem, network, process, or database access
      (<code>fs</code>, <code>http</code>, <code>os</code>, <code>db</code> are
      native-only), and runaway programs are cut off after 5 seconds.
    </p>
  </div>

  <div class="toolbar">
    <select bind:value={selected} on:change={pickExample} aria-label="example programs">
      {#each EXAMPLES as ex}
        <option value={ex.name}>{ex.name}</option>
      {/each}
    </select>
    <button class="run" on:click={run} disabled={running}>
      {running ? 'running…' : 'run ▸'}
    </button>
    <span class="hint">⌘⏎ / ctrl⏎</span>
  </div>

  <div class="panes">
    <textarea
      class="editor"
      bind:value={source}
      on:keydown={onKeydown}
      spellcheck="false"
      autocomplete="off"
      autocapitalize="off"
      aria-label="olang source"
    ></textarea>

    <div class="result" class:empty={!ranOnce}>
      {#if !ranOnce}
        <span class="placeholder">output appears here — hit run</span>
      {:else}
        {#if crash}
          <pre class="crash">{crash}</pre>
        {:else}
          {#if output}<pre class="stdout">{output}</pre>{/if}
          {#if error}<pre class="err">{error}</pre>{/if}
          {#if value != null}<pre class="value">=> {value}</pre>{/if}
          {#if !output && !error && value == null}
            <span class="placeholder">(no output)</span>
          {/if}
          <div class="meta">
            {#if ms != null}{ms}ms{/if}
            {#if version}· olang {version} · wasm{/if}
          </div>
        {/if}
      {/if}
    </div>
  </div>
</main>

<style>
  .playground {
    padding: 2.5rem 1.5rem 4rem;
  }
  .intro h1 {
    color: var(--paper);
    font-family: var(--mono);
    font-size: 1.9rem;
    margin: 0 0 0.6rem;
  }
  .intro p {
    max-width: 46rem;
    margin: 0 0 1.6rem;
  }
  .intro code {
    font-family: var(--mono);
    color: var(--paper);
    font-size: 0.92em;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    margin-bottom: 0.9rem;
  }
  .toolbar select {
    background: var(--surface);
    color: var(--body);
    border: 1px solid var(--line-bright);
    border-radius: var(--radius);
    font-family: var(--mono);
    font-size: 0.9rem;
    padding: 0.45rem 0.6rem;
  }
  .run {
    background: var(--teal);
    color: #04211c;
    border: none;
    border-radius: var(--radius);
    font-family: var(--mono);
    font-weight: 700;
    font-size: 0.92rem;
    padding: 0.5rem 1.1rem;
    cursor: pointer;
  }
  .run:hover:not(:disabled) {
    background: var(--teal-deep);
  }
  .run:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .hint {
    font-family: var(--mono);
    font-size: 0.8rem;
    color: var(--slate);
  }

  .panes {
    display: grid;
    grid-template-columns: 1.1fr 0.9fr;
    gap: 1rem;
    min-height: 26rem;
  }
  @media (max-width: 860px) {
    .panes {
      grid-template-columns: 1fr;
    }
  }

  .editor {
    background: var(--ink-2);
    color: var(--paper);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    font-family: var(--mono);
    font-size: 0.92rem;
    line-height: 1.6;
    padding: 1.1rem 1.2rem;
    resize: vertical;
    min-height: 26rem;
    tab-size: 4;
  }
  .editor:focus {
    outline: none;
    border-color: var(--teal-deep);
  }

  .result {
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 1.1rem 1.2rem;
    font-family: var(--mono);
    font-size: 0.9rem;
    line-height: 1.55;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  .result pre {
    margin: 0 0 0.6rem;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .stdout {
    color: var(--paper);
  }
  .value {
    color: var(--teal);
  }
  .err,
  .crash {
    color: #f87171;
  }
  .placeholder {
    color: var(--slate);
  }
  .meta {
    margin-top: auto;
    padding-top: 0.8rem;
    color: var(--slate);
    font-size: 0.78rem;
  }
</style>
