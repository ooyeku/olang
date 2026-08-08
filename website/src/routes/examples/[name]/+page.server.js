import { error } from '@sveltejs/kit';
import { examplesCatalog, exampleSource, GITHUB } from '$lib/content.js';

export function entries() {
  return examplesCatalog().map((e) => ({ name: e.name }));
}

export async function load({ params }) {
  const entry = examplesCatalog().find((e) => e.name === params.name);
  if (!entry) error(404, 'no such example');
  return { entry, files: await exampleSource(params.name), github: `${GITHUB}/tree/main/examples/${params.name}` };
}
