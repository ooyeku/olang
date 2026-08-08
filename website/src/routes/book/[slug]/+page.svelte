<script>
  import BookShell from '$lib/components/BookShell.svelte';
  export let data;
</script>
<svelte:head><title>{data.chapter.title} — the olang book</title></svelte:head>
<BookShell book={data.book} chapter={data.chapter} />
