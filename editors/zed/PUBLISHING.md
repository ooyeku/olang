# Publishing the Zed extension

Zed's registry is a Git repo: [zed-industries/extensions] holds every
published extension as a submodule, and publishing is one PR from the
existing GitHub account. No new accounts, no tokens.

[zed-industries/extensions]: https://github.com/zed-industries/extensions

## Preconditions (all already true here)

- `extension.toml` has `schema_version = 1` and the id/name/version
  fields — see this directory's `extension.toml`.
- The repo carries an accepted license (MIT, at the olang repo root).
- The grammar reference in `extension.toml` points at a **pushed**
  commit of `ooyeku/olang` (`[grammars.olang]` pins
  `editors/tree-sitter-olang` by SHA — Zed clones the repo at that rev
  to build the parser, so the rev must exist on GitHub, never a local-
  only commit). If the grammar changes, push first, then update `rev`.
- Tested locally as a dev extension (`zed: install dev extension` →
  select this `editors/zed` directory).

## The PR process

1. Fork `zed-industries/extensions` to the personal account (they ask
   for personal forks, not org forks).

2. Add the olang repo as a submodule, over HTTPS (SSH URLs are
   rejected):

   ```bash
   git clone https://github.com/ooyeku/extensions
   cd extensions
   git submodule add https://github.com/ooyeku/olang.git extensions/olang
   git add extensions/olang
   ```

   The submodule's pinned commit must be reachable on a branch of
   `ooyeku/olang` (ours is `main`) — not a detached/unpushed commit.

3. Register it in `extensions.toml`. Because the extension lives in a
   subdirectory of the olang repo, the entry carries a `path`:

   ```toml
   [olang]
   submodule = "extensions/olang"
   path = "editors/zed"
   version = "0.1.0"
   ```

   `version` must match `extension.toml` in `editors/zed`.

4. Sort the metadata files (the CI checks this):

   ```bash
   pnpm sort-extensions
   ```

5. Open the PR against `zed-industries/extensions`. On merge, their CI
   packages the extension and it appears in Zed's in-app extension
   browser. PRs for extensions that don't build or obviously don't work
   get closed, hence the dev-extension test first.

## Releasing updates

Bump `version` in `editors/zed/extension.toml`, push to `main`, then in
the fork: update the `extensions/olang` submodule to the new commit,
bump `version` in `extensions.toml` to match, `pnpm sort-extensions`,
PR again.

## Note on `editors/zed-olang`

`editors/zed` (this directory) is the canonical extension — it carries
the language-server config and the SHA-pinned grammar. `editors/
zed-olang` is the earlier highlight-only variant kept for reference;
don't submit it.
