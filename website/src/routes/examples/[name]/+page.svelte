<script>
  export let data;
</script>
<svelte:head><title>{data.entry.name} — olang examples</title></svelte:head>

<section class="section">
  <div class="container">
    <p class="kicker"><a href="/examples">examples</a> / {data.entry.name}</p>
    <h2>{data.entry.name}/</h2>
    <p class="sub">{data.entry.description}</p>
    <div class="copybox">
      <span class="prompt">$</span>
      <span>cd examples/{data.entry.name} && olang main.ol</span>
    </div>
    <p style="margin-top:0.9rem"><a href={data.github}>View on GitHub →</a></p>
    <div class="example-files">
      {#each data.files as f}
        <div class="file-label">{f.path}</div>
        {@html f.html}
      {/each}
    </div>
  </div>
</section>
