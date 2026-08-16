<script>
  export let data;
  let copied = false;
  const install = 'git clone https://github.com/ooyeku/olang && cd olang && cargo install --path .';
  function copy() {
    navigator.clipboard?.writeText(install);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<svelte:head><title>olang — the Open Language</title></svelte:head>

<section class="hero">
  <div class="container">
    <div>
      <h1><img src="/brand/logo.svg" alt="olang" /></h1>
      <p class="lede">
        <b>The Open Language.</b> A program's structure is a public data format
        (<code>meta.parse</code>); a compiled binary carries its own source and
        declares what it may touch (<code>olang inspect</code>, capabilities);
        any run records and replays bit-for-bit. Batteries-included and
        functional underneath — pipelines, pattern matching, a built-in data
        stack, no-GIL parallelism, a three-tier runtime that always agrees with
        the interpreter or refuses.
      </p>
      <dl class="spec">
        <div>
          <dt>open code</dt>
          <dd><code>meta.parse(src)</code> — the program as walkable data; linters and codemods in olang</dd>
        </div>
        <div>
          <dt>open artifacts</dt>
          <dd><code>olang inspect ./tool</code> — source, checksum, and per-dependency capabilities; no black boxes</dd>
        </div>
        <div>
          <dt>open execution</dt>
          <dd><code>olang --record bug.olt</code> then <code>replay</code> — deterministic, portable, to the last digit</dd>
        </div>
        <div>
          <dt>playground</dt>
          <dd>the whole language compiled to WebAssembly, <a href="/playground">sandboxed in your browser</a></dd>
        </div>
      </dl>
      <div class="cta-row">
        <a class="btn btn-primary" href="/playground">Run it in the browser</a>
        <a class="btn btn-ghost" href="/book">Read the book</a>
      </div>
      <div class="copybox">
        <div class="lines">
          <div><span class="prompt">$</span> git clone https://github.com/ooyeku/olang</div>
          <div><span class="prompt">$</span> cd olang && cargo install --path .</div>
        </div>
        <button on:click={copy}>{copied ? 'copied' : 'copy'}</button>
      </div>
    </div>
    <div class="hero-code">
      <div class="code-head">
        <span class="file">shapes.ol</span>
        <span class="note">from README.md — executed by the test suite</span>
      </div>
      {@html data.heroHtml}
    </div>
  </div>
</section>

<section class="section" id="numbers">
  <div class="container">
    <p class="kicker">measured, not asserted</p>
    <h2>The numbers</h2>
    <p class="sub">
      Defaults, no flags — what a plain <code>olang program.ol</code> gets.
      Methodology and the full tables are in
      <a href="/book/ovm">the OVM chapter</a>.
    </p>
    <div class="table-scroll">
    <table class="bench-table">
      <thead>
        <tr>
          <th>workload</th>
          <th class="num">olang</th>
          <th class="num">node</th>
          <th class="num">bun</th>
          <th class="num">cpython</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <td class="label">fib(30) — 2.7M recursive calls</td>
          <td class="num self">4 ms</td>
          <td class="num">4 ms</td>
          <td class="num">4 ms</td>
          <td class="num">46 ms</td>
        </tr>
        <tr>
          <td class="label">N-body — 120 bodies × 150 steps</td>
          <td class="num self">26 ms</td>
          <td class="num">6 ms</td>
          <td class="num">8 ms</td>
          <td class="num">396 ms</td>
        </tr>
        <tr>
          <td class="label">map(λ) |&gt; sum — 1M elements</td>
          <td class="num self">20 ms</td>
          <td class="num">9 ms</td>
          <td class="num">4 ms</td>
          <td class="num">23 ms</td>
        </tr>
        <tr>
          <td class="label">par_map / par for on compute-heavy kernels</td>
          <td class="num self wide" colspan="4">9–13× across cores, no GIL</td>
        </tr>
        <tr>
          <td class="label">10M-row, 1k-group aggregation</td>
          <td class="num self wide" colspan="4">27.2 ms single-threaded — Polars: 24.0 ms on 18 threads</td>
        </tr>
      </tbody>
    </table>
    </div>
    <p class="bench-note">
      fib(30) went 89 ms → 4 ms and N-body 400 ms → 26 ms as the Cranelift
      JIT lane landed and closed — level with the JavaScript JITs on numeric
      work, ~11× ahead of CPython on fib. Interpreter-only mode runs the
      same programs; the tiers are an optimization, never a semantic.
    </p>
  </div>
</section>

<section class="section alt" id="principles">
  <div class="container">
    <p class="kicker">overview</p>
    <h2>Design principles</h2>
    <p class="sub">
      Each of these properties is enforced by the test suite: documentation
      examples execute in CI, all execution tiers are required to agree,
      and the standard library is differential-tested against itself.
    </p>
    <div class="grid-3">
      <div class="card">
        <h3><span class="glyph">|&gt;</span> Pipeline-oriented</h3>
        <p>Data flows left to right through <code>map</code>, <code>filter</code>,
        and <code>fold</code> — transformation chains read in execution order.</p>
      </div>
      <div class="card">
        <h3><span class="glyph">::</span> Algebraic data types</h3>
        <p>Enums that construct, structs that validate their shape, and
        pattern matching with guards, ranges, and destructuring.</p>
      </div>
      <div class="card">
        <h3><span class="glyph">?</span> Errors are values</h3>
        <p><code>Result</code> and <code>?</code> carry expected failure; a bug
        stops the program. Even a failed <code>spawn</code> task is just an
        <code>Err</code> you can match on.</p>
      </div>
      <div class="card">
        <h3><span class="glyph">~</span> One concurrency model</h3>
        <p><code>spawn</code> runs on OS threads and <code>task.join</code>
        collects — no async colouring. <code>http.serve</code> uses a bounded
        worker pool, load-tested under concurrent writes.</p>
      </div>
      <div class="card">
        <h3><span class="glyph">&gt;&gt;</span> Tiered execution</h3>
        <p>Hot functions promote to register bytecode, hot numeric functions
        to native code via Cranelift; unsupported constructs fall back,
        never diverging.</p>
      </div>
      <div class="card">
        <h3><span class="glyph">##</span> Batteries included</h3>
        <p>22 stdlib modules: JSON, CSV, SQLite, HTTP client + server, regex,
        crypto, time, files, the data stack — plus a package manager with
        lockfiles.</p>
      </div>
    </div>
  </div>
</section>

<section class="section" id="book">
  <div class="container">
    <p class="kicker">documentation</p>
    <h2>The olang book</h2>
    <p class="sub">
      A complete reference, from first program to interpreter internals.
      Every code block in the book is executed by the test suite on every
      change.
    </p>
    <div class="grid-3">
      {#each data.book as ch}
        <a class="card" href={'/book/' + ch.slug}>
          <h3>{ch.title}</h3>
          <p>docs/{ch.file}</p>
        </a>
      {/each}
    </div>
  </div>
</section>

<section class="section alt" id="examples">
  <div class="container">
    <p class="kicker">examples</p>
    <h2>Working programs</h2>
    <p class="sub">
      The language is validated by building complete programs against every
      release — parsers, servers, simulations, tooling. Browse the full
      source of each.
    </p>
    <div class="grid-3">
      {#each data.examples as ex}
        <a class="card" href={'/examples/' + ex.name}>
          <h3>{ex.name}/</h3>
          <p>{ex.description.length > 150 ? ex.description.slice(0, 147) + '…' : ex.description}</p>
        </a>
      {/each}
    </div>
    <p style="margin-top:1.6rem"><a href="/examples">See all examples →</a></p>
  </div>
</section>
