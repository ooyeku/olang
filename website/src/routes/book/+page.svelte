<script>
  import BookShell from '$lib/components/BookShell.svelte';
  export let data;
</script>
<svelte:head><title>The olang Book</title></svelte:head>
<BookShell book={data.book} chapter={data.chapter} />
