<script>
  import { onMount } from 'svelte';

  // Three states, not two: "system" is a real choice and the default, so a
  // reader who never touches this follows their OS. Only an explicit pick
  // writes storage, and only an explicit pick stamps data-theme.
  let theme = 'system';

  onMount(() => {
    theme = localStorage.getItem('theme') ?? 'system';
  });

  function apply(next) {
    theme = next;
    if (next === 'system') {
      localStorage.removeItem('theme');
      document.documentElement.removeAttribute('data-theme');
    } else {
      localStorage.setItem('theme', next);
      document.documentElement.setAttribute('data-theme', next);
    }
  }

  /** Flip to whichever scheme is not currently showing. */
  function toggle() {
    const dark =
      theme === 'dark' ||
      (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    apply(dark ? 'light' : 'dark');
  }
</script>

<button
  class="theme-toggle"
  on:click={toggle}
  title="Switch between light and dark"
  aria-label="Switch between light and dark"
>
  <!-- sun: shown on a dark page, because it switches you to light -->
  <svg class="to-dark" viewBox="0 0 24 24" aria-hidden="true">
    <circle cx="12" cy="12" r="4.2" />
    <path
      d="M12 2.6v2.2M12 19.2v2.2M2.6 12h2.2M19.2 12h2.2M5.4 5.4l1.6 1.6M17 17l1.6 1.6M18.6 5.4L17 7M7 17l-1.6 1.6"
      stroke-linecap="round"
    />
  </svg>
  <!-- moon: shown on a light page -->
  <svg class="to-light" viewBox="0 0 24 24" aria-hidden="true">
    <path
      d="M20.5 14.2A8.6 8.6 0 1 1 9.8 3.5a6.9 6.9 0 0 0 10.7 10.7Z"
      stroke-linejoin="round"
    />
  </svg>
</button>
