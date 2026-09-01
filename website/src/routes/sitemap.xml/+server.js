// The sitemap is generated from the same catalog the pages render from,
// so it can never list a route that does not exist (the hand-maintained
// file it replaces drifted twice: stale example paths, missing chapters).
import { BOOK, examplesCatalog } from '$lib/content.js';

export const prerender = true;

export function GET() {
  const SITE = 'https://olang.dev';
  const urls = [
    '/',
    '/book',
    '/examples',
    '/playground',
    ...BOOK.map((c) => `/book/${c.slug}`),
    ...examplesCatalog().map((e) => `/examples/${e.name}`)
  ];
  const xml =
    '<?xml version="1.0" encoding="UTF-8"?>\n' +
    '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n' +
    urls.map((u) => `  <url><loc>${SITE}${u}</loc></url>`).join('\n') +
    '\n</urlset>\n';
  return new Response(xml, { headers: { 'Content-Type': 'application/xml' } });
}
