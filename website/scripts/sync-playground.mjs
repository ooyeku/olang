// Build the olang playground wasm from the repo source and stage it into
// static/. Same zero-drift rule as the rest of the site: the artifact is
// built from the checkout, never committed or hand-copied.
import { execSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '..', '..');
const wasm = path.join(
  repo,
  'target',
  'wasm32-unknown-unknown',
  'release',
  'olang_playground.wasm'
);
const dest = path.join(here, '..', 'static', 'playground');

execSync('cargo build -p olang-playground --target wasm32-unknown-unknown --release', {
  cwd: repo,
  stdio: 'inherit',
});
mkdirSync(dest, { recursive: true });
copyFileSync(wasm, path.join(dest, 'olang.wasm'));
console.log('playground: staged olang.wasm');
