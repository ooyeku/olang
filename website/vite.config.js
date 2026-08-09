import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: { fs: { allow: ['..'] } },
  // Honor a harness-assigned port (tooling sets PORT when it manages the
  // server); default stays vite's usual 4173/5173 when unset.
  preview: process.env.PORT ? { port: Number(process.env.PORT) } : {}
});
