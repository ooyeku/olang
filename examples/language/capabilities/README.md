# capabilities — blocking a malicious dependency

This example shows per-dependency capability attenuation: a dependency can
be granted less access than the application, and the runtime enforces it at
the boundary where effects happen.

## Layout

- `analytics/` — a third-party package. `summarize` is a pure helper; `report`
  is a backdoor that reads a local file.
- `unguarded/` — an app that depends on `analytics` with no capability
  manifest. Every dependency inherits full access, as in most package
  managers today.
- `guarded/` — the same app, whose `olang.toml` grants `analytics`
  `fs = false`.
- `main.ol` — runs both variants in a subprocess and reports the contrast.

The two apps have identical code. Only the manifest differs.

## Run

```bash
olang main.ol
```

This demo runs its sub-apps in fresh olang subprocesses, so run it with the
interpreter. It is not meant to be built into a standalone binary with
`olang build` — a built binary can only run its own embedded program, so the
narrator detects that case and exits with a note instead.

The unguarded app lets `analytics` read `secret.txt`. The guarded app blocks
the identical read at the capability gate, while the app's own file read
still succeeds — the gate keys on which package makes a call, not on the
call itself.

## The manifest

`guarded/olang.toml` restricts the dependency:

```toml
[capabilities]
fs = true

[capabilities.dependencies.analytics]
fs = false
net = false
```

Attenuation can only shrink a grant, never widen it, so a dependency cannot
exceed what the application allows regardless of what it or its transitive
dependencies attempt. See [Capabilities](../../docs/packages.md#capabilities).
