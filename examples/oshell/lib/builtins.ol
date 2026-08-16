// oshell builtins: every command implemented on the olang stdlib —
// fs for the filesystem, os for environment and directories, re for
// grep, str/col for text. Nothing here shells out.
//
// Convention: run(state, argv, input) -> { state, out, code }.
// `input` is the stage's stdin text (from a pipe or < redirect); `out`
// is what flows to the next stage, a > redirect, or the terminal.
// State is threaded functionally: { aliases, history, last }.

share fn is_builtin(name) = str.contains(
    " cd pwd ls cat echo env export unset alias unalias history which type " +
    "mkdir rm cp mv touch head tail grep wc sleep help exit true false ",
    " " + name + " "
)

share fn run(state, argv, input) = {
    let name = argv[0]
    let args = tail(argv)
    if name == "cd" => bi_cd(state, args)
    else => if name == "pwd" => done(state, cwd_or("?") + "\n", 0)
    else => if name == "ls" => bi_ls(state, args)
    else => if name == "cat" => bi_cat(state, args, input)
    else => if name == "echo" => bi_echo(state, args)
    else => if name == "env" => bi_env(state)
    else => if name == "export" => bi_export(state, args)
    else => if name == "unset" => bi_unset(state, args)
    else => if name == "alias" => bi_alias(state, args)
    else => if name == "unalias" => bi_unalias(state, args)
    else => if name == "history" => bi_history(state)
    else => if name == "which" => bi_which(state, args, false)
    else => if name == "type" => bi_which(state, args, true)
    else => if name == "mkdir" => bi_mkdir(state, args)
    else => if name == "rm" => bi_rm(state, args)
    else => if name == "cp" => bi_two(state, args, "cp", (a, b) => fs.copy_file(a, b))
    else => if name == "mv" => bi_two(state, args, "mv", (a, b) => fs.move_file(a, b))
    else => if name == "touch" => bi_touch(state, args)
    else => if name == "head" => bi_headtail(state, args, input, true)
    else => if name == "tail" => bi_headtail(state, args, input, false)
    else => if name == "grep" => bi_grep(state, args, input)
    else => if name == "wc" => bi_wc(state, args, input)
    else => if name == "sleep" => bi_sleep(state, args)
    else => if name == "help" => bi_help(state)
    else => if name == "exit" => bi_exit(state, args)
    else => if name == "true" => done(state, "", 0)
    else => if name == "false" => done(state, "", 1)
    else => fail(state, "oshell: not a builtin: " + name)
}

fn done(state, out, code) = { state: state, out: out, code: code }
fn fail(state, msg) = { state: state, out: msg + "\n", code: 1 }

fn cwd_or(fallback) = match os.cwd() { Ok(d) => d, Err(e) => fallback }

// ── directories ──────────────────────────────────────────────────────

fn bi_cd(state, args) = {
    let target = if len(args) == 0 => home_or("/")
        else => if args[0] == "-" => match os.get_env("OLDPWD") { Ok(d) => d, Err(e) => cwd_or(".") }
        else => args[0]
    let before = cwd_or(".")
    match os.chdir(target) {
        Ok(v) => {
            os.set_env("OLDPWD", before)
            done(state, "", 0)
        },
        Err(e) => fail(state, "cd: " + target + ": " + e)
    }
}

fn home_or(fallback) = match os.home_dir() { Ok(h) => h, Err(e) => fallback }

fn bi_ls(state, args) = {
    let long = contains(args, "-l")
    let all = contains(args, "-a")
    let paths = args |> filter((a) => !str.starts_with(a, "-"))
    let target = if len(paths) == 0 => "." else => paths[0]
    if target != "." && unwrap_or(fs.exists(target), false) && !unwrap_or(fs.is_dir(target), false) => {
        if long => {
            match fs.file_info(target) {
                Ok(info) => done(state, "- " + str.pad_start(to_string(map_get(info, "size")), 10, " ") + "  " + target + "\n", 0),
                Err(e) => fail(state, "ls: " + target + ": " + to_string(e))
            }
        } else => done(state, target + "\n", 0)
    } else => match fs.list_dir(target) {
        Ok(names) => {
            let visible = if all => sort(names)
                else => sort(names |> filter((n) => !str.starts_with(n, ".")))
            if long => {
                let mut lines = visible |> map((n) => {
                    let p = if target == "." => n else => target + "/" + n
                    match fs.file_info(p) {
                        Ok(info) => {
                            let kind = if map_get(info, "is_dir") => "d" else => "-"
                            kind + " " + str.pad_start(to_string(map_get(info, "size")), 10, " ") + "  " + n
                        },
                        Err(e) => "?" + " " + str.pad_start("?", 10, " ") + "  " + n
                    }
                })
                done(state, joined_lines(lines), 0)
            } else => done(state, joined_lines(visible), 0)
        },
        Err(e) => fail(state, "ls: " + target + ": " + to_string(e))
    }
}

fn joined_lines(xs) = if len(xs) == 0 => "" else => str.join(xs, "\n") + "\n"

fn contains(xs, x) = len(xs |> filter((v) => v == x)) > 0

// ── text ─────────────────────────────────────────────────────────────

fn bi_cat(state, args, input) = {
    if len(args) == 0 => done(state, input, 0)
    else => {
        let mut out = ""
        let mut code = 0
        for f in args {
            match fs.read_file(f) {
                Ok(text) => { out = out + text },
                Err(e) => {
                    out = out + "cat: " + f + ": " + e + "\n"
                    code = 1
                }
            }
        }
        done(state, out, code)
    }
}

fn bi_echo(state, args) = {
    if len(args) > 0 && args[0] == "-n" => done(state, str.join(tail(args), " "), 0)
    else => done(state, str.join(args, " ") + "\n", 0)
}

fn bi_headtail(state, args, input, is_head) = {
    let mut count = 10
    let mut rest = args
    if len(args) >= 2 && args[0] == "-n" => {
        count = unwrap_or(str.parse_int(args[1]), 10)
        rest = tail(tail(args))
    }
    let text = if len(rest) > 0 => match fs.read_file(rest[0]) { Ok(t) => t, Err(e) => "" }
        else => input
    let mut lines = str.lines(text)
    let n = len(lines)
    let keep = if is_head => lines |> take_n(count)
        else => lines |> drop_n(if n > count => n - count else => 0)
    done(state, joined_lines(keep), 0)
}

fn take_n(xs, n) = {
    let mut out = []
    let mut i = 0
    while i < len(xs) && i < n {
        out = out + [xs[i]]
        i = i + 1
    }
    out
}

fn drop_n(xs, n) = {
    let mut out = []
    let mut i = n
    while i < len(xs) {
        out = out + [xs[i]]
        i = i + 1
    }
    out
}

fn bi_grep(state, args, input) = {
    if len(args) == 0 => fail(state, "grep: usage: grep pattern [file]")
    else => {
        let pattern = args[0]
        let text = if len(args) > 1 => match fs.read_file(args[1]) { Ok(t) => t, Err(e) => "" }
            else => input
        let hits = str.lines(text) |> filter((line) => unwrap_or(re.is_match(pattern, line), false))
        done(state, joined_lines(hits), if len(hits) > 0 => 0 else => 1)
    }
}

fn bi_wc(state, args, input) = {
    let text = if len(args) > 0 => match fs.read_file(args[0]) { Ok(t) => t, Err(e) => "" }
        else => input
    let l = len(str.lines(text))
    let w = len(str.words(text))
    let c = str.length(text)
    done(state, str.pad_start(to_string(l), 8, " ") + str.pad_start(to_string(w), 8, " ") + str.pad_start(to_string(c), 8, " ") + "\n", 0)
}

// ── environment and shell state ──────────────────────────────────────

fn bi_env(state) = {
    match os.list_env() {
        Ok(pairs) => {
            let mut lines = sort(map_keys(pairs)) |> map((k) => k + "=" + map_get(pairs, k))
            done(state, joined_lines(lines), 0)
        },
        Err(e) => fail(state, "env: " + e)
    }
}

fn bi_export(state, args) = {
    if len(args) == 0 => fail(state, "export: usage: export NAME=value")
    else => {
        let mut code = 0
        for spec in args {
            let eq = str.index_of(spec, "=")
            if eq > 0 => {
                os.set_env(str.substring(spec, 0, eq), str.substring(spec, eq + 1, str.length(spec)))
            } else => { code = 1 }
        }
        if code == 0 => done(state, "", 0)
        else => fail(state, "export: usage: export NAME=value")
    }
}

fn bi_unset(state, args) = {
    for name in args { os.remove_env(name) }
    done(state, "", 0)
}

fn bi_alias(state, args) = {
    if len(args) == 0 => {
        let mut lines = sort(map_keys(state.aliases))
            |> map((k) => "alias " + k + "='" + map_get(state.aliases, k) + "'")
        done(state, joined_lines(lines), 0)
    } else => {
        let spec = str.join(args, " ")
        let eq = str.index_of(spec, "=")
        if eq > 0 => {
            let name = str.substring(spec, 0, eq)
            let value = str.substring(spec, eq + 1, str.length(spec))
            done({ aliases: map_set(state.aliases, name, value), history: state.history, last: state.last }, "", 0)
        } else => fail(state, "alias: usage: alias name=value")
    }
}

fn bi_unalias(state, args) = {
    if len(args) == 0 => fail(state, "unalias: usage: unalias name")
    else => {
        let mut kept = #{}
        for k in map_keys(state.aliases) {
            if !contains(args, k) => {
                kept = map_set(kept, k, map_get(state.aliases, k))
            }
        }
        done({ aliases: kept, history: state.history, last: state.last }, "", 0)
    }
}

fn bi_history(state) = {
    let mut lines = []
    let mut i = 0
    while i < len(state.history) {
        lines = lines + [str.pad_start(to_string(i + 1), 5, " ") + "  " + state.history[i]]
        i = i + 1
    }
    done(state, joined_lines(lines), 0)
}

// ── locating commands ────────────────────────────────────────────────

fn bi_which(state, args, verbose) = {
    if len(args) == 0 => fail(state, "which: usage: which command")
    else => {
        let name = args[0]
        if verbose && is_builtin(name) => done(state, name + " is an oshell builtin\n", 0)
        else => if verbose && map_has_key(state.aliases, name) => {
            done(state, name + " is aliased to '" + map_get(state.aliases, name) + "'\n", 0)
        } else => {
            match path_lookup(name) {
                Ok(full) => done(state, if verbose => name + " is " + full + "\n" else => full + "\n", 0),
                Err(e) => fail(state, name + " not found")
            }
        }
    }
}

share fn path_lookup(name) = {
    if str.contains(name, "/") => {
        if unwrap_or(fs.exists(name), false) => Ok(name) else => Err("not found")
    } else => {
        let path = match os.get_env("PATH") { Ok(p) => p, Err(e) => "" }
        let mut hit = ""
        for dir in str.split(path, ":") {
            if hit == "" && dir != "" => {
                let candidate = dir + "/" + name
                if unwrap_or(fs.exists(candidate), false) => { hit = candidate }
            }
        }
        if hit != "" => Ok(hit) else => Err("not found")
    }
}

// ── files ────────────────────────────────────────────────────────────

fn bi_mkdir(state, args) = {
    let real = args |> filter((a) => a != "-p")
    if len(real) == 0 => fail(state, "mkdir: usage: mkdir [-p] dir")
    else => {
        let mut code = 0
        let mut out = ""
        for d in real {
            match fs.create_dir_all(d) {
                Ok(v) => {},
                Err(e) => {
                    out = out + "mkdir: " + d + ": " + e + "\n"
                    code = 1
                }
            }
        }
        done(state, out, code)
    }
}

fn bi_rm(state, args) = {
    let recursive = contains(args, "-r") || contains(args, "-rf")
    let real = args |> filter((a) => !str.starts_with(a, "-"))
    if len(real) == 0 => fail(state, "rm: usage: rm [-r] path")
    else => {
        let mut code = 0
        let mut out = ""
        for p in real {
            let r = if unwrap_or(fs.is_dir(p), false) => {
                if recursive => fs.remove_dir_all(p)
                else => Err("is a directory (use rm -r)")
            } else => fs.remove_file(p)
            match r {
                Ok(v) => {},
                Err(e) => {
                    out = out + "rm: " + p + ": " + to_string(e) + "\n"
                    code = 1
                }
            }
        }
        done(state, out, code)
    }
}

fn bi_two(state, args, name, op) = {
    if len(args) != 2 => fail(state, name + ": usage: " + name + " source dest")
    else => match op(args[0], args[1]) {
        Ok(v) => done(state, "", 0),
        Err(e) => fail(state, name + ": " + to_string(e))
    }
}

fn bi_touch(state, args) = {
    if len(args) == 0 => fail(state, "touch: usage: touch file")
    else => {
        for f in args {
            if !unwrap_or(fs.exists(f), false) => { fs.write_file(f, "") }
        }
        done(state, "", 0)
    }
}

// ── misc ─────────────────────────────────────────────────────────────

fn bi_sleep(state, args) = {
    if len(args) == 0 => fail(state, "sleep: usage: sleep seconds")
    else => {
        let secs = unwrap_or(str.parse_float(args[0]), -1.0)
        if secs < 0.0 => fail(state, "sleep: invalid time: " + args[0])
        else => {
            time.sleep(to_int(secs * 1000.0))
            done(state, "", 0)
        }
    }
}

fn bi_exit(state, args) = {
    let mut code = if len(args) > 0 => unwrap_or(str.parse_int(args[0]), 0) else => 0
    os.exit(code)
    done(state, "", code)
}

fn bi_help(state) = {
    let text = "oshell — a Unix-like shell written in olang\n" +
        "\n" +
        "builtins:\n" +
        "  cd [dir|-]        pwd               ls [-l] [-a] [path]\n" +
        "  cat [file...]     echo [-n] args    head/tail [-n N] [file]\n" +
        "  grep pat [file]   wc [file]         which/type cmd\n" +
        "  mkdir [-p] dir    rm [-r] path      cp/mv src dest    touch file\n" +
        "  env               export K=V        unset K\n" +
        "  alias [k=v]       unalias k         history\n" +
        "  sleep secs        true/false        exit [code]\n" +
        "\n" +
        "features: pipes (a | b), redirection (< > >>), sequencing (; && ||),\n" +
        "$VAR and ${VAR} and $?, ~ expansion, globbing (* ?), quoting (' \" \\),\n" +
        "comments (#). Anything else runs from PATH via os.exec.\n"
    done(state, text, 0)
}
