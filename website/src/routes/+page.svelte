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

<svelte:head><title>olang — a batteries-included functional language</title></svelte:head>

<section class="hero">
  <div class="container">
    <div>
      <h1><img src="/brand/logo.svg" alt="olang" /></h1>
      <p class="tag">pipelines · pattern matching · <b>batteries included</b> · tiered execution</p>
      <p class="lede">
        A dynamic functional language with pipeline-oriented data flow,
        algebraic data types, Result-based error handling, thread-backed
        concurrency, and a conservative bytecode tier that accelerates hot
        code without changing program semantics.
      </p>
      <div class="cta-row">
        <a class="btn btn-primary" href="/book">Read the book</a>
        <a class="btn btn-ghost" href="/examples">Browse examples</a>
      </div>
      <div class="copybox">
        <span class="prompt">$</span>
        <span>{install}</span>
        <button on:click={copy}>{copied ? 'copied' : 'copy'}</button>
      </div>
    </div>
    <div class="terminal">
      <div class="terminal-bar">
        <span class="dot"></span><span class="dot"></span><span class="dot"></span>
        <span class="name">shapes.ol</span>
      </div>
      {@html data.heroHtml}
    </div>
  </div>
</section>

<section class="section">
  <div class="container">
    <p class="kicker">overview</p>
    <h2>Design principles</h2>
    <p class="sub">
      Each of these properties is enforced by the test suite: documentation
      examples execute in CI, both execution tiers are required to agree,
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
        <p><code>Result</code> + <code>?</code> + <code>try/catch</code> compose —
        even a failed <code>spawn</code> task is just an <code>Err</code> you can match on.</p>
      </div>
      <div class="card">
        <h3><span class="glyph">~</span> Thread-backed concurrency</h3>
        <p><code>spawn</code> runs on OS threads; <code>http.serve</code> uses a
        bounded worker pool, load-tested for exact consistency under
        concurrent writes.</p>
      </div>
      <div class="card">
        <h3><span class="glyph">&gt;&gt;</span> Tiered execution</h3>
        <p>Hot functions compile to register bytecode (9× on the N-body
        benchmark); unsupported constructs fall back to the interpreter,
        never diverging.</p>
      </div>
      <div class="card">
        <h3><span class="glyph">✓</span> Batteries included</h3>
        <p>18 stdlib modules: JSON, CSV, SQLite, HTTP client + server, regex,
        crypto, time, files — plus a package manager with lockfiles.</p>
      </div>
    </div>
    <div class="stat-row">
      <div class="stat"><b>9×</b><span>N-body on the bytecode tier</span></div>
      <div class="stat"><b>45</b><span>test binaries in CI</span></div>
      <div class="stat"><b>38</b><span>runnable example programs</span></div>
      <div class="stat"><b>18</b><span>stdlib modules</span></div>
    </div>
  </div>
</section>

<section class="section alt">
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

<section class="section">
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
