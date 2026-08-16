<script>
  import '../app.css';
  import { page } from '$app/stores';
  import Header from '$lib/components/Header.svelte';
  import Footer from '$lib/components/Footer.svelte';

  const SITE = 'https://olang.dev';
  const DESC =
    'olang — the Open Language. A program\'s structure is a public data ' +
    'format, a compiled binary carries its own source and declares what it ' +
    'may touch, and any run records and replays bit-for-bit. Batteries ' +
    'included: pipelines, pattern matching, a built-in data stack, no-GIL ' +
    'parallelism, and a three-tier runtime.';

  $: canonical = SITE + $page.url.pathname;
</script>

<svelte:head>
  <link rel="canonical" href={canonical} />
  <meta property="og:type" content="website" />
  <meta property="og:site_name" content="olang" />
  <!-- Crawlers that find no og:title fall back to <title>, which each
       route already sets; this is the default for the ones that don't. -->
  <meta property="og:title" content="olang — the Open Language" />
  <meta name="twitter:title" content="olang — the Open Language" />
  <meta property="og:url" content={canonical} />
  <meta property="og:description" content={DESC} />
  <meta property="og:image" content="{SITE}/brand/og.png" />
  <meta property="og:image:width" content="1200" />
  <meta property="og:image:height" content="630" />
  <meta property="og:image:alt" content="The olang mark: a ring with the pipeline running through it" />
  <meta name="twitter:card" content="summary_large_image" />
  <meta name="twitter:description" content={DESC} />
  <meta name="twitter:image" content="{SITE}/brand/og.png" />
</svelte:head>

<Header />
<main id="main"><slot /></main>
<Footer />
