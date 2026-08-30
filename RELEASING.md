# Releasing olang

The complete runbook: cut the release, let CI build the artifacts, then
publish per channel. Steps 1–4 are the process every release since 0.25
has followed; 5–6 are the distribution machinery layered on top.

## 1. Version bump

- `docs/packages.md` — the REPL transcript shows the version banner
  (`Olang vX.Y.Z`); update it to match.
- `Cargo.toml` — bump `version` (this is the version `olang --version`
  and `otc --version` both report).
- `CHANGELOG.md` — promote `## [Unreleased]` to `## [X.Y.Z] - <date>`.

Nothing else carries the version; `Cargo.lock` is not committed.

## 2. Gates (all green before the release commit)

```bash
cargo test --workspace                 # full suite, including the
                                       # bytecode differential + tier suites
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cd examples && olang run_all.ol        # the example corpus (skips long_running)
cargo build -p olang-playground --target wasm32-unknown-unknown --release
                                       # wasm build stays green (make wasm)
make install                           # then: olang --version / otc --version
                                       # report the new number
```

If uncommitted WIP in `examples/` (historically `loops.ol`,
`benchmark.ol`) interferes with `run_all`, stash it scoped around that
step only.

The Linux pass runs the same suite and the examples harness inside a
Linux container against the working tree (requires a Docker-compatible
daemon; the container builds into its own cached volume, so the host
`target/` is untouched):

```bash
dist/linux-verify.sh                   # full suite + examples on Linux
```

The pass is architecture-honest — on Apple silicon it verifies
linux/aarch64; run it on an x86_64 host for x86_64 coverage.

## 3. Release commit

```bash
git add Cargo.toml CHANGELOG.md docs/packages.md
git commit -m "chore(release): vX.Y.Z"   # body: what the release is,
                                          # closing with the gates that passed
```

Always explicit `git add` paths — never `-A`. Check `git show --stat`
afterward: nothing but the two files should be in the commit.

## 4. Tag and push

```bash
git tag vX.Y.Z
git push origin main vX.Y.Z
```

## 5. What the tag fires automatically

Pushing the `vX.Y.Z` tag triggers `.github/workflows/release.yml`,
which with **no further action**:

1. Builds `olang` + `otc` in release mode for **macos-arm64**,
   **macos-x64**, and **linux-x64**; strips them; packages each with
   LICENSE + README as `olang-X.Y.Z-<platform>.tar.gz`.
2. Builds the playground wasm and packages it as
   `olang-playground-X.Y.Z.wasm`.
3. Generates `SHA256SUMS` over all of it.
4. Creates the GitHub Release for the tag with everything attached.

Cross-check locally any time with `make dist` — same binaries, same
wasm, plus the .vsix, staged into `dist/out/` with its own SHA256SUMS.

## 6. Per-channel publishing

| Channel | Requires account? | What to do |
|---|---|---|
| GitHub Releases | **No** — existing GitHub account | Nothing; step 5 creates it. Optionally edit the release notes. |
| Homebrew tap | **No** — just a repo (`ooyeku/homebrew-olang`) | Update `version` + three `sha256`s in the tap's formula from the release's `SHA256SUMS`. Full steps: [dist/homebrew/README.md](dist/homebrew/README.md). |
| Zed registry | **No** — a GitHub PR to `zed-industries/extensions` | First time: submodule + `extensions.toml` entry. Updates: bump the submodule + version. Full steps: [editors/zed/PUBLISHING.md](editors/zed/PUBLISHING.md). |
| VS Code Marketplace | **Yes** — free Azure DevOps publisher (`ooyeku`) | `vsce login` + `vsce publish`. Full steps: [editors/vscode/PUBLISHING.md](editors/vscode/PUBLISHING.md). |
| VS Code without the Marketplace | **No** | `npx vsce package` (or take the one `make dist` built) and share/attach the .vsix; installs via "Install from VSIX…". |

The editor extensions version independently of olang (`0.1.0` until
they change); only publish them when they actually changed.
