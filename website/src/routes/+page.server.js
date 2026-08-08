import { heroSnippet, highlight, examplesCatalog, BOOK } from '$lib/content.js';

export async function load() {
  const snippet = heroSnippet();
  return {
    heroHtml: await highlight(snippet, 'olang'),
    examples: examplesCatalog().slice(0, 6),
    book: BOOK
  };
}
