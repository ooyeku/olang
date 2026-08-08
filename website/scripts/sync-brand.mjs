// Copy brand assets from ../branding into static/brand on every dev/build,
// so the site can never drift from the canonical brand.
import { cpSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const branding = join(here, '..', '..', 'branding');
const out = join(here, '..', 'static', 'brand');

mkdirSync(out, { recursive: true });
for (const f of ['logo.svg', 'logo-light.svg', 'logo-mono.svg', 'wordmark.svg',
  'mark.svg', 'mark-light.svg', 'banner.svg', 'favicon.svg', 'mascot.svg']) {
  cpSync(join(branding, f), join(out, f));
}
cpSync(join(branding, 'mascot'), join(out, 'mascot'), { recursive: true });
cpSync(join(branding, 'icons'), join(out, 'icons'), { recursive: true });
console.log('brand assets synced');
