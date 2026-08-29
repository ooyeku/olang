# survey — the command-line flagship

`survey` is to olang's terminal story what the [tracker
suite](../app/) is to its web story: one real program that exercises
the whole command-line toolkit at once.

- **`cli`** — the entire command surface (`summary` / `langs` /
  `files`, each with a path argument and a `--top` flag) is one
  declarative spec, with `--help` and clean exit codes for free.
- **`term`** — colored summaries, a per-language bar chart, aligned
  tables (colored cells and all), and a live progress bar while
  scanning. All of it falls back to plain text automatically when the
  output is piped.
- **`fs`** — it walks the directory tree, skipping build and VCS dirs.

It is a single self-contained `main.ol`: no local imports, only stdlib
and the embedded packages. That is what lets it become a standalone
binary.

## Run it

```bash
olang examples/tools/survey/main.ol                 # survey the current directory
olang examples/tools/survey/main.ol summary src/    # a specific directory
olang examples/tools/survey/main.ol langs . --top 8 # every language, by lines
olang examples/tools/survey/main.ol files . -n 15   # the largest files
```

## The full build → ship → document arc

```bash
# ship it as one binary that needs no olang installed
olang build examples/tools/survey/main.ol -o survey
./survey summary src/

# generate its reference from the /// and //! doc comments
olang doc examples/tools/survey/main.ol -o survey.html
```

That is the Toolsmith campaign end to end: write the tool with `cli`
and `term`, ship it with `olang build`, document it with `olang doc`.
