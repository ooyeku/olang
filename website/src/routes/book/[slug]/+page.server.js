import { error } from '@sveltejs/kit';
import { renderChapter, BOOK } from '$lib/content.js';

export function entries() {
  return BOOK.map((c) => ({ slug: c.slug }));
}

export async function load({ params }) {
  const chapter = await renderChapter(params.slug);
  if (!chapter) error(404, 'no such chapter');
  return { chapter, book: BOOK };
}
