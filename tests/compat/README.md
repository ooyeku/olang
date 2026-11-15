# tests/compat — the 1.x compatibility fixtures

What every olang 1.x must keep doing, as 1.0 does it. The promise is
[docs/stability.md](../../docs/stability.md#the-1x-compatibility-check).
`tools/release.sh` runs `tools/compat-check.sh` before every tag, and
`tools/release-check.sh` runs the programs against every published
release.

| Path | What |
|---|---|
| `programs/<area>.ol` + `<area>.out` | a program per Stable area and its exact stdout |
| `programs/<area>/main.ol` + `<area>.out` | the same, for a program of several files (`modules`) |
| `surface/<x.y>.txt` | every builtin and non-Experimental module member at release x.y; written once, never edited |
| `package/` | a package whose public surface is `@stable`, for `olang check --against` |
| `allow.toml` | intended breaks, reviewed: the only way an expectation may change |

A program's first comment lines may carry a header:
`// compat: exit 1` (the exit status it must end with, default 0) and
`// compat: test` (run with `olang test`; the `.out` lines must then
appear in order among the runner's report, which is not contract).

Run them:

```bash
cargo test --profile fasttest --test compat_test   # with the suite
tools/compat-check.sh --olang path/to/olang         # any binary, plus --against 1.0.0
```

Adding a program: write it, print only what is contract (values and
`raise` messages, never error text; a report's fields rather than the
whole record), run `OLANG_COMPAT_BLESS=1 cargo test --profile fasttest
--test compat_test` to write its `.out` (only a missing `.out` is ever
written), and read the `.out` before committing it. Never edit an
existing `.out` or snapshot to make a test pass: a change that is meant
goes in `allow.toml` first.

## An intended break

A 1.x release may change behavior only as a bug fix, or an Experimental
surface in a minor release (docs/stability.md, Versioning). When such a
change alters a program's output or removes a surface name:

1. Add a `[[break]]` entry to `allow.toml`: `what` (the fixture as the
   test names it, or `surface:<name>`), `version`, `issue`, `kind`
   (`bug-fix`, `experimental` or `major`) and `reason`.
2. Update the `.out` in the same commit if you like; compat-check.sh
   accepts a changed expectation only when an entry names it.
3. Name the issue in the release's CHANGELOG section. `tools/release.sh`
   refuses the release otherwise, refuses a `major` break outside a new
   major version and an `experimental` one in a patch, and prints every
   intended break of the release it cuts.

The allow-list is reviewed like code: an entry is a decision about what
olang promises.
