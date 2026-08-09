# olang website

The static site for olang, built with SvelteKit and Bun.

```bash
cd website
bun install
bun run dev        # local dev server
bun run build      # static site into build/
bun run preview    # serve the built site
```

## How content works

The site has no copied content. At build time it reads from the repository:

| Site section | Source |
|---|---|
| Book pages (`/book`, `/book/[slug]`) | `../docs/*.md` and `../docs/design/*.md`, rendered with chapter navigation and rewritten cross-links |
| Example gallery (`/examples`) | entry names and descriptions parsed from `../examples/README.md` |
| Example sources (`/examples/[name]`) | the actual `.ol` files under `../examples/<name>/` |
| Hero code sample | the first `olang` code block in `../README.md` |
| Brand assets (`static/brand/`) | synced from `../branding/` by `scripts/sync-brand.mjs` on every dev/build |

Because everything renders from the canonical files, updating a doc or an
example updates the site on the next build — and a broken cross-link in the
docs fails the build (prerender link checking), which has already caught
real anchor errors.

Syntax highlighting uses a custom TextMate grammar for olang
(`src/lib/olang-lang.js`) with a theme built from the brand palette. The
grammar's keyword list mirrors `grammar.pest`; update it when keywords
change.

## Deploying

`bun run build` emits a fully static site in `build/` — deployable to any
static host (GitHub Pages, Netlify, Cloudflare Pages). All routes are
prerendered; there is no server runtime.
