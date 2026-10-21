# Releasing olang

The complete runbook: cut the release, let CI build the artifacts, then
publish per channel.

## 1. Version bump

- `Cargo.toml` — bump `version` (the version `olang --version` and
  `otc --version` both report).
- `playground/Cargo.toml` — the same `version`, for the browser runtime.
- `Cargo.lock` — refreshed by the next build; commit it with the bump.
- `CHANGELOG.md` — promote `## [Unreleased]` to `## [X.Y.Z] - <date>`.

Nothing else carries the version.

## 2. Gates (all green before the release commit)

```bash
cargo test --workspace --profile fasttest  # full suite (~6 min), including the
                                       # bytecode differential + tier suites
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cd examples && olang run_all.ol        # the example corpus (skips long_running)
cargo xtask wasm                       # the browser runtime builds (make wasm)
make install                           # then: olang --version / otc --version
                                       # report the new number
```

If uncommitted work in `examples/` interferes with `run_all`, stash it
around that step only.

Linux is a preview platform ([stability.md](docs/stability.md#platforms)).
To check it anyway, `dist/linux-verify.sh` runs the same suite and the
examples harness in a Linux container against the working tree (it needs a
Docker-compatible daemon and builds into its own cached volume, so the host
`target/` is untouched). On Apple silicon it verifies linux/aarch64.

## 3. Release commit

```bash
git add Cargo.toml playground/Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "X.Y.Z: <what the release is>"   # body closes with the gates that passed
```

Always explicit `git add` paths, never `-A`. Check `git show --stat`
afterward: nothing but those four files should be in the commit.

## 4. Tag and push

```bash
git tag vX.Y.Z
git push origin main vX.Y.Z
```

## 5. What the tag fires automatically

Pushing the `vX.Y.Z` tag triggers `.github/workflows/release.yml`,
which with **no further action**:

1. Builds the browser runtime (`cargo xtask wasm`), then `olang` +
   `otc` in release mode for **macos-arm64**, **macos-x64**,
   **linux-x64**, and **linux-arm64**, each embedding the runtime;
   strips them; packages each with LICENSE + README as
   `olang-X.Y.Z-<platform>.tar.gz`.
2. Packages the browser runtime as `olang-playground-X.Y.Z.wasm`.
3. Generates `SHA256SUMS` over all of it.
4. Creates the GitHub Release for the tag with everything attached. A
   version with a suffix (`v1.0.0-rc.2`) is marked a prerelease.

Cross-check locally any time with `make dist` — same binaries, same
wasm, plus the .vsix, staged into `dist/out/` with its own SHA256SUMS.

The Homebrew tap updates itself: the workflow's `homebrew` job renders
`dist/homebrew/olang.rb.tmpl` with the release's checksums and pushes it
to `ooyeku/homebrew-olang`, provided the `TAP_PUSH_TOKEN` secret (a
fine-grained PAT with Contents read/write on the tap repo) is
configured. Without the secret the job skips and the manual procedure
in `dist/homebrew/README.md` applies. A prerelease does not move the
tap.

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
