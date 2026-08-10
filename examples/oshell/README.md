# oshell — a Unix-like shell written in olang

The long-running systems-work proof: an interactive shell where every
operation is olang and its stdlib. The line loop reads with
`os.read_line`, pipelines thread stdout→stdin through `os.exec`,
redirection and globbing ride on `fs`, the environment on `os`, `grep`
on `re` — no operation shells out to another shell.

```bash
cd examples/oshell
olang main.ol
```

```
oshell 1.0 — a Unix-like shell written in olang. 'help' lists builtins; ctrl-d exits.
you@host:~/Code/olang/examples/oshell$ cat data.txt | grep two >> hits.log && echo saved
```

## What works

- **Pipelines** — `a | b | c`, stdout threading into stdin, builtins and
  external commands mixing freely in one pipeline
- **Redirection** — `< in.txt`, `> out.txt`, `>> append.txt`
- **Sequencing** — `;`, `&&`, `||` with real short-circuit semantics and
  `$?` tracking (127 for command-not-found, external exit codes as-is)
- **Expansion** — `$VAR`, `${VAR}`, `$?`, leading `~`, and globbing
  (`*`, `?`) with bash's no-match-stays-literal rule
- **Quoting** — single quotes are literal, double quotes group but
  expand, `\` escapes, segments concatenate (`--name="a b"` is one word)
- **Builtins on the stdlib** — `cd pwd ls cat echo head tail grep wc
  mkdir rm cp mv touch env export unset alias unalias history which
  type sleep true false help exit`
- **Everything else** — resolved from `PATH` and run via `os.exec`
- **History** — persisted to `~/.oshell_history` across sessions

## Why it exists

To demonstrate that olang holds up when a program is a *process*, not a
script: a session runs indefinitely, threads state through thousands of
commands, spawns and reaps child processes, and touches the filesystem
constantly. The soak test pushes 1,000 mixed commands (pipes, externals,
redirects, sequencing) through one session in ~1.4 s with nothing
leaking or degrading.

`os.read_line()` — one line from stdin, `Err("eof")` at end — was added
to the stdlib for exactly this class of program; it is the only
primitive a prompt loop needs.

## Layout

| File | Role |
|---|---|
| `main.ol` | state, prompt, the read loop, pipeline/chain execution |
| `lib/lexer.ol` | line → tokens: quoting, operators, comments, escapes |
| `lib/parse.ol` | tokens → pipelines and chains; `$`/`~`/glob expansion |
| `lib/builtins.ol` | every builtin, implemented on `fs`/`os`/`re`/`str` |

## Honest limits

No job control (`&`, `fg`, ctrl-z), no subshells or command
substitution, no `VAR=x cmd` prefixes, and pipelines buffer between
stages rather than streaming — each stage runs to completion before the
next starts. Redirections apply per pipeline, not per stage. These are
scope choices, not language limits.
