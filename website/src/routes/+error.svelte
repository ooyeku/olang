<script>
  import { page } from '$app/stores';
</script>

<svelte:head><title>{$page.status} — olang</title></svelte:head>

<section class="notfound container">
  <p class="code">{$page.status}</p>
  <h1>{$page.status === 404 ? 'No page here' : ($page.error?.message ?? 'Something went wrong')}</h1>
  <p>
    {#if $page.status === 404}
      The link may be out of date. The book's table of contents is the
      best place to pick the thread back up.
    {:else}
      Try again, or open the book — everything on this site is also in the
      repository.
    {/if}
  </p>
  <div class="cta-row" style="justify-content:center">
    <a class="btn btn-primary" href="/book">Read the book</a>
    <a class="btn btn-ghost" href="/">Home</a>
  </div>
</section>
