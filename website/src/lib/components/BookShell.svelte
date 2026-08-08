<script>
  import { page } from '$app/stores';
  export let book;
  export let chapter;

  $: idx = book.findIndex((c) => c.slug === chapter.slug);
  $: prev = idx > 0 ? book[idx - 1] : null;
  $: next = idx >= 0 && idx < book.length - 1 ? book[idx + 1] : null;
</script>

<div class="container book-shell">
  <aside class="book-nav">
    <div class="head">The olang book</div>
    <a href="/book" class:active={$page.url.pathname === '/book'}>Overview</a>
    {#each book as ch}
      <a href={'/book/' + ch.slug} class:active={$page.url.pathname === '/book/' + ch.slug}>
        {ch.title}
      </a>
    {/each}
  </aside>
  <article class="prose">
    {@html chapter.html}
    {#if chapter.slug}
      <div class="chapter-footer">
        <span>{#if prev}<a href={'/book/' + prev.slug}>← {prev.title}</a>{:else}<a href="/book">← Overview</a>{/if}</span>
        <span>{#if next}<a href={'/book/' + next.slug}>{next.title} →</a>{/if}</span>
      </div>
    {/if}
  </article>
</div>
