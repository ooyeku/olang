import { examplesCatalog } from '$lib/content.js';
export function load() {
  return { examples: examplesCatalog() };
}
