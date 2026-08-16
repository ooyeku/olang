<script>
  import { page } from '$app/stores';
  import { tick } from 'svelte';
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

  let article;
  let headings = [];
  let activeId = '';
  let navEl;
  let menuOpen = false;

  // The chapter outline is read from the rendered HTML rather than from the
  // markdown, so it stays true to whatever the pipeline actually emitted.
  async function indexHeadings(_html) {
    await tick();
    if (!article) return;
    headings = [...article.querySelectorAll('h2[id], h3[id]')].map((el) => ({
      id: el.id,
      text: el.textContent.replace(/\s*#\s*$/, '').trim(),
      sub: el.tagName === 'H3',
    }));
    activeId = headings[0]?.id ?? '';
    observe();
  }
  $: indexHeadings(chapter.html);

  // Scroll position decides the active heading rather than an
  // IntersectionObserver: an observer only fires on a *crossing*, so a
  // jump (a hash link, restored scroll, a flick of the wheel) can skip
  // every heading it passed and leave the rail pointing at the top.
  let tocEl;
  let throttled = false;
  function onScroll() {
    // A plain time throttle rather than requestAnimationFrame: rAF is
    // suspended while the tab is hidden, which would leave the rail
    // frozen on whatever was active when the reader switched away.
    if (throttled) return;
    throttled = true;
    setTimeout(() => {
      throttled = false;
      spy();
    }, 90);
    spy();
  }

  function spy() {
    if (!headings.length) return;
    const line = 140; // just below the sticky header
    let current = headings[0].id;
    for (const h of headings) {
      const el = document.getElementById(h.id);
      if (el && el.getBoundingClientRect().top <= line) current = h.id;
    }
    // At the very bottom the last section is the one being read, even if
    // its heading never reaches the line.
    if (window.innerHeight + window.scrollY >= document.body.offsetHeight - 4) {
      current = headings[headings.length - 1].id;
    }
    if (current !== activeId) {
      activeId = current;
      keepTocVisible();
    }
  }

  function keepTocVisible() {
    const link = tocEl?.querySelector('a.active');
    if (!link) return;
    const box = tocEl.getBoundingClientRect();
    const lb = link.getBoundingClientRect();
    if (lb.top < box.top + 8 || lb.bottom > box.bottom - 8) {
      link.scrollIntoView({ block: 'nearest', behavior: 'auto' });
    }
  }

  function observe() {
    spy();
  }

  // Keep the active chapter visible in a sidebar that now scrolls itself.
  $: if (navEl && chapter.slug !== undefined) scrollNavToActive();
  async function scrollNavToActive() {
    await tick();
    const active = navEl?.querySelector('a.active');
    if (!active) return;
    const navBox = navEl.getBoundingClientRect();
    const box = active.getBoundingClientRect();
    if (box.top < navBox.top || box.bottom > navBox.bottom) {
      active.scrollIntoView({ block: 'center', behavior: 'auto' });
    }
  }
</script>

<svelte:window on:scroll={onScroll} on:resize={onScroll} />

<div class="container book-shell">
  <aside class="book-nav" bind:this={navEl}>
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

  <article class="prose" bind:this={article}>
    <details class="book-menu" bind:open={menuOpen}>
      <summary>{chapter.title ?? 'The olang book'}</summary>
      <div class="menu-body">
        <a href="/book" class:active={$page.url.pathname === '/book'}>Overview</a>
        {#each parts as group}
          {#if group.part}<div class="part">{group.part}</div>{/if}
          {#each group.chapters as ch}
            <a
              href={'/book/' + ch.slug}
              class:active={$page.url.pathname === '/book/' + ch.slug}
              on:click={() => (menuOpen = false)}
            >
              {ch.title}
            </a>
          {/each}
        {/each}
      </div>
    </details>

    {@html chapter.html}

    {#if chapter.slug}
      <div class="chapter-footer">
        <span>{#if prev}<a href={'/book/' + prev.slug}>← {prev.title}</a>{:else}<a href="/book">← Overview</a>{/if}</span>
        <span>{#if next}<a href={'/book/' + next.slug}>{next.title} →</a>{/if}</span>
      </div>
    {/if}
  </article>

  <nav class="book-toc" aria-label="On this page" bind:this={tocEl}>
    {#if headings.length > 1}
      <div class="head">On this page</div>
      {#each headings as h}
        <a href={'#' + h.id} class:sub={h.sub} class:active={activeId === h.id}>{h.text}</a>
      {/each}
    {/if}
  </nav>
</div>

<style>
  /* The disclosure reuses the sidebar's link styling inside its body. */
  .book-menu :global(a) {
    display: block;
    color: var(--text-2);
    padding: 0.34rem 0.75rem;
    border-left: 1px solid var(--line);
    font-size: 0.9rem;
  }
  .book-menu :global(a:hover) { color: var(--text); text-decoration: none; background: var(--surface); }
  .book-menu :global(a.active) { color: var(--accent); border-left-color: var(--accent); background: var(--accent-soft); }
  .book-menu :global(.part) {
    font-family: var(--mono);
    color: var(--text-4);
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    margin: 1rem 0 0.35rem;
    padding-left: 0.75rem;
  }
</style>
