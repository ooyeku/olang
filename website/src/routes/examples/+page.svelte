<script>
  export let data;
</script>
<svelte:head><title>Examples — olang</title></svelte:head>

<section class="section">
  <div class="container">
    <p class="kicker">examples</p>
    <h2>Example programs</h2>
    <p class="sub">
      Complete, runnable programs — data pipelines, servers, parsers,
      simulations, and tooling — each shipped and tested with the language.
    </p>
    <div class="grid-3">
      {#each data.examples as ex}
        <a class="card" href={'/examples/' + ex.name}>
          <h3>{ex.name}/</h3>
          <p>{ex.description}</p>
        </a>
      {/each}
    </div>
  </div>
</section>
