// oshell — a Unix-like shell written in olang.
//
// The proof-of-life example for long-running systems work: an
// interactive loop on os.read_line, pipelines threading stdout→stdin
// through os.exec, redirection and globbing on fs, environment on os,
// grep on re — every operation in olang and its stdlib.
//
//   cd examples/tools/oshell && olang main.ol
//
// Also scriptable: printf 'ls -l\nexit\n' | olang main.ol

use lib.lexer { tokenize }
use lib.parse { parse_chains, expand_token }
use lib.builtins { is_builtin, run, path_lookup }

// ── executing one pipeline stage ─────────────────────────────────────

fn stage_argv(state, stage_tokens) = {
    // Alias expansion (one level) on the leading word, then token
    // expansion: $VAR, ~, globs — each token can splice several words.
    let mut toks = stage_tokens
    if len(toks) > 0 && toks[0].q == "w" && map_has_key(state.aliases, toks[0].t) => {
        let value = map_get(state.aliases, toks[0].t)
        match tokenize(value) {
            Ok(alias_toks) => { toks = alias_toks + tail(toks) },
            Err(e) => {}
        }
    }
    toks |> col.flat_map((t) => expand_token(t, state.last))
}

fn run_stage(state, argv, input) = {
    if len(argv) == 0 => { state: state, out: input, code: 0 }
    else => if is_builtin(argv[0]) => run(state, argv, input)
    else => {
        match os.exec(argv[0], tail(argv), #{ "stdin": input }) {
            Ok(r) => {
                if r.stderr != "" => { print(r.stderr) }
                { state: state, out: r.stdout, code: r.code }
            },
            Err(e) => {
                let msg = match path_lookup(argv[0]) {
                    Ok(found) => `oshell: ${argv[0]}: ${e}`,
                    Err(nf) => "oshell: command not found: " + argv[0]
                }
                println(msg)
                { state: state, out: "", code: 127 }
            }
        }
    }
}

// ── executing a pipeline, then a chain of pipelines ──────────────────

fn run_pipeline(state, link) = {
    let input = if link.in_file != "" => {
        match fs.read_file(link.in_file) {
            Ok(text) => text,
            Err(e) => {
                println(`oshell: ${link.in_file}: ${e}`)
                ""
            }
        }
    } else => ""

    let mut s = state
    let mut out = input
    let mut code = 0
    for stage in link.pipe {
        let argv = stage_argv(s, stage)
        let r = run_stage(s, argv, out)
        s = r.state
        out = r.out
        code = r.code
    }

    if link.out_file != "" => {
        let w = if link.append => fs.append_file(link.out_file, out)
            else => fs.write_file(link.out_file, out)
        match w {
            Ok(v) => {},
            Err(e) => {
                println(`oshell: ${link.out_file}: ${e}`)
                code = 1
            }
        }
    } else => {
        if out != "" => { print(out) }
    }
    { state: s, code: code }
}

fn run_line(state, line) = {
    match tokenize(line) {
        Err(e) => {
            println(e)
            { aliases: state.aliases, history: state.history, last: 2 }
        },
        Ok(tokens) => match parse_chains(tokens, state.last) {
            Err(e) => {
                println(e)
                { aliases: state.aliases, history: state.history, last: 2 }
            },
            Ok(chains) => {
                let mut s = state
                let mut skip_until_or = false
                let mut skip_until_and = false
                for link in chains {
                    // Sequencing: run this link unless a previous && failed
                    // (skip to after ;) or a previous || succeeded.
                    if !skip_until_or && !skip_until_and => {
                        let r = run_pipeline(s, link)
                        s = { aliases: r.state.aliases, history: r.state.history, last: r.code }
                        if link.op == "&&" && r.code != 0 => { skip_until_or = true }
                        if link.op == "||" && r.code == 0 => { skip_until_and = true }
                    } else => {
                        // Skipping: a ; boundary clears both skip modes; a
                        // || after a failed && gives that branch a chance.
                        if skip_until_or && link.op == "||" => { skip_until_or = false }
                        if link.op == ";" => {
                            skip_until_or = false
                            skip_until_and = false
                        }
                    }
                }
                s
            }
        }
    }
}

// ── history ──────────────────────────────────────────────────────────

fn hist_path() = match os.home_dir() {
    Ok(h) => h + "/.oshell_history",
    Err(e) => ".oshell_history"
}

fn load_history() = match fs.read_file(hist_path()) {
    Ok(text) => str.lines(text) |> filter((l) => l != ""),
    Err(e) => []
}

// ── the prompt and the loop ──────────────────────────────────────────

fn prompt_text() = {
    let user = os.username()
    let host = match os.hostname() { Ok(h) => h, Err(e) => "?" }
    let cwd = match os.cwd() { Ok(d) => d, Err(e) => "?" }
    let home = match os.home_dir() { Ok(h) => h, Err(e) => "" }
    let shown = if home != "" && str.starts_with(cwd, home) =>
        "~" + str.substring(cwd, str.length(home), str.length(cwd))
        else => cwd
    "\u001b[1;32m" + user + "@" + host + "\u001b[0m:\u001b[1;34m" + shown + "\u001b[0m$ "
}

let interactive = len(os.args()) <= 1 || os.args()[1] != "-q"
if interactive => {
    println("oshell 1.0 — a Unix-like shell written in olang. 'help' lists builtins; ctrl-d exits.")
}

let mut state = { aliases: #{}, history: load_history(), last: 0 }
let mut running = true
while running {
    print(prompt_text())
    match os.read_line() {
        Err(e) => {
            println("")
            running = false
        },
        Ok(line) => {
            let trimmed = str.trim(line)
            if trimmed != "" => {
                state = { aliases: state.aliases, history: state.history + [trimmed], last: state.last }
                let _ = fs.append_file(hist_path(), trimmed + "\n")   // history is best-effort
                state = run_line(state, trimmed)
            }
        }
    }
}
