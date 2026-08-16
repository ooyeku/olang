<script>
  import { page } from '$app/stores';
  export let book;
  export let chapter;

  $: idx = book.findIndex((c) => c.slug === chapter.slug);
  $: prev = idx > 0 ? book[idx - 1] : null;
  $: next = idx >= 0 && idx < book.length - 1 ? book[idx + 1] : null;

  // Group chapters by their Part for the navigation, preserving book order.
  $: parts = book.reduce((acc, ch) => {
    const label = ch.part ?? '';
    const group = acc[acc.length - 1];
    if (group && group.part === label) group.chapters.push(ch);
    else acc.push({ part: label, chapters: [ch] });
    return acc;
  }, []);
</script>

<div class="container book-shell">
  <aside class="book-nav">
    <div class="head">The olang book</div>
    <a href="/book" class:active={$page.url.pathname === '/book'}>Overview</a>
    {#each parts as group}
      {#if group.part}<div class="part">{group.part}</div>{/if}
      {#each group.chapters as ch}
        <a href={'/book/' + ch.slug} class:active={$page.url.pathname === '/book/' + ch.slug}>
          {ch.title}
        </a>
      {/each}
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
