# The Homebrew tap

`brew install olang` via a personal tap. No new accounts, no central
approval — a tap is just a GitHub repo with a magic name, and the
existing `ooyeku` account is all it takes.

## One-time setup

1. Create a **public** GitHub repo named exactly `homebrew-olang`
   (i.e. `ooyeku/homebrew-olang`). The `homebrew-` prefix is what lets
   `brew tap ooyeku/olang` find it.
2. Copy `olang.rb` from this directory into `Formula/olang.rb` in that
   repo. (The repo needs nothing else — no README required, though one
   is polite.)
3. Fill in the real `version` and the three `sha256` values (below),
   commit, push.

Users then install with:

```bash
brew tap ooyeku/olang
brew install olang        # installs both `olang` and `otc`
```

or in one line: `brew install ooyeku/olang/olang`.

## Per-release update (automated)

The release workflow's `homebrew` job renders `olang.rb.tmpl` with the
new version and the four tarball checksums and pushes the result to the
tap's `Formula/olang.rb`. It needs one secret in the olang repo:
`TAP_PUSH_TOKEN`, a token with push access to `ooyeku/homebrew-olang`
(a fine-grained PAT scoped to that one repo, Contents read/write). With
the secret absent the job skips quietly and the manual procedure below
still applies.

## Per-release update (manual fallback)

The release workflow (`.github/workflows/release.yml`) attaches a
`SHA256SUMS` file to every GitHub Release alongside the tarballs. After
a release is published:

1. In the tap's `Formula/olang.rb`, set `version` to the new release
   (tag without the leading `v` — e.g. tag `v0.46.0` → `"0.46.0"`).
   The `url` lines derive from `version` and need no editing.
2. Replace the three `sha256` placeholders/values with the matching
   lines from the release's `SHA256SUMS`:
   - `olang-<version>-macos-arm64.tar.gz` → the `on_arm` block
   - `olang-<version>-macos-x64.tar.gz` → the `on_intel` (macOS) block
   - `olang-<version>-linux-x64.tar.gz` → the `on_linux` block

   ```bash
   curl -LO https://github.com/ooyeku/olang/releases/download/v0.46.0/SHA256SUMS
   cat SHA256SUMS
   ```

3. Sanity-check locally, then commit and push the tap:

   ```bash
   brew reinstall --build-from-source ooyeku/olang/olang   # uses the local formula
   brew test olang                                          # runs the formula's test block
   ```

Users pick the new version up with a plain `brew upgrade olang`.

## Keeping the copies in sync

`olang.rb.tmpl` in this directory is the template of record — if the
asset naming or install layout in the release workflow ever changes,
change this file in the same commit, then propagate to the tap at the
next release.
