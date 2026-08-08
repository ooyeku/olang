import { renderChapter, BOOK } from '$lib/content.js';
export async function load() {
  const chapter = await renderChapter(null);
  return { chapter, book: BOOK };
}
