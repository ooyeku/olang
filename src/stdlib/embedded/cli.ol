// cli — declarative command-line argument parsing, bundled as an
// embedded olang package (`use cli`).
//
// A spec is a plain map describing a program's flags, positional
// arguments, and (optionally) subcommands. `parse` walks an argv list
// and returns Ok(values) — a map holding every flag and argument by
// name, plus a boolean `help` — or Err(message) with a human-readable
// diagnostic. `-h`/`--help` short-circuit parse with `help` set true,
// so the caller prints the usage from `help(spec)` and exits 0.
//
//   use cli
//   let spec = #{
//       "name": "greet", "about": "Greet someone",
//       "flags": [
//           #{ "name": "loud", "short": "l", "type": "bool", "help": "SHOUT it" },
//           #{ "name": "count", "short": "n", "type": "int", "default": 1,
//              "help": "repeat N times" }
//       ],
//       "args": [ #{ "name": "who", "required": true, "help": "who to greet" } ]
//   }
//   match cli.parse(spec, cli.args()) {
//       Err(e) => { println(e); os.exit(2) }
//       Ok(a) => {
//           if map_get(a, "help") => { println(cli.help(spec)); os.exit(0) }
//           // ... map_get(a, "who"), map_get(a, "count"), map_get(a, "loud")
//       }
//   }

// The program's own arguments, with the program path (argv[0]) dropped.
share fn args() = skip(unwrap_or(os.args(), []), 1)

// ── spec accessors ─────────────────────────────────────────────────────

fn opt(m, k, d) = if map_has_key(m, k) => map_get(m, k) else => d
fn flags_of(spec) = opt(spec, "flags", [])
fn args_of(spec) = opt(spec, "args", [])
fn cmds_of(spec) = opt(spec, "commands", [])
fn ftype(f) = opt(f, "type", "string")
fn strip(tok, pfx) = str.replace_first(tok, pfx, "")

fn by_long(flags, name) = filter(flags, (f) => map_get(f, "name") == name)
fn by_short(flags, ch) = filter(flags,
    (f) => map_has_key(f, "short") && map_get(f, "short") == ch)

// ── value coercion (Result, so type errors surface cleanly) ────────────

fn coerce(f, label, raw) = {
    let t = ftype(f)
    if t == "int" => {
        if is_ok(str.parse_int(raw)) => Ok(unwrap(str.parse_int(raw)))
        else => Err(label + " expects an integer, got \"" + raw + "\"")
    } else => {
        if t == "float" => {
            if is_ok(str.parse_float(raw)) => Ok(unwrap(str.parse_float(raw)))
            else => Err(label + " expects a number, got \"" + raw + "\"")
        } else => Ok(raw)
    }
}

// ── the core walk: flags + positionals for one (sub)command spec ────────

fn parse_core(spec, argv) = {
    let flags = flags_of(spec)
    let pargs = args_of(spec)

    // Seed defaults: bool flags start false; typed flags take their
    // `default`, or an `env` fallback, or stay absent (caller uses
    // unwrap_or / map_has_key).
    let mut out = #{ "help": false }
    for f in flags {
        let name = map_get(f, "name")
        if ftype(f) == "bool" => { out = map_set(out, name, false) }
        else => {
            if map_has_key(f, "default") => { out = map_set(out, name, map_get(f, "default")) }
            else => {
                if map_has_key(f, "env") && unwrap_or(os.has_env(map_get(f, "env")), false) => {
                    out = map_set(out, name, unwrap_or(os.get_env(map_get(f, "env")), ""))
                }
            }
        }
    }

    let mut pos = []
    let mut i = 0
    let n = len(argv)
    while i < n {
        let tok = argv[i]
        if tok == "-h" || tok == "--help" => {
            return Ok(map_set(out, "help", true))
        }
        if str.starts_with(tok, "--") => {
            let body = strip(tok, "--")
            let eq = str.index_of(body, "=")
            let has_eq = eq >= 0
            let key = if has_eq => str.substring(body, 0, eq) else => body
            let matches = by_long(flags, key)
            if len(matches) == 0 => { return Err("unknown flag: --" + key) }
            let f = matches[0]
            if ftype(f) == "bool" => {
                out = map_set(out, key, true)
                i = i + 1
            } else => {
                if has_eq => {
                    match coerce(f, "--" + key, str.substring(body, eq + 1, str.length(body))) {
                        Err(e) => { return Err(e) }
                        Ok(v) => { out = map_set(out, key, v) }
                    }
                    i = i + 1
                } else => {
                    if i + 1 >= n => { return Err("flag --" + key + " needs a value") }
                    match coerce(f, "--" + key, argv[i + 1]) {
                        Err(e) => { return Err(e) }
                        Ok(v) => { out = map_set(out, key, v) }
                    }
                    i = i + 2
                }
            }
        } else => {
            if str.starts_with(tok, "-") && str.length(tok) > 1 => {
                let body = strip(tok, "-")
                let eq = str.index_of(body, "=")
                let has_eq = eq >= 0
                let ch = if has_eq => str.substring(body, 0, eq) else => body
                let matches = by_short(flags, ch)
                if len(matches) == 0 => { return Err("unknown flag: -" + ch) }
                let f = matches[0]
                let name = map_get(f, "name")
                if ftype(f) == "bool" => {
                    out = map_set(out, name, true)
                    i = i + 1
                } else => {
                    if has_eq => {
                        match coerce(f, "-" + ch, str.substring(body, eq + 1, str.length(body))) {
                            Err(e) => { return Err(e) }
                            Ok(v) => { out = map_set(out, name, v) }
                        }
                        i = i + 1
                    } else => {
                        if i + 1 >= n => { return Err("flag -" + ch + " needs a value") }
                        match coerce(f, "-" + ch, argv[i + 1]) {
                            Err(e) => { return Err(e) }
                            Ok(v) => { out = map_set(out, name, v) }
                        }
                        i = i + 2
                    }
                }
            } else => {
                pos = pos + [tok]
                i = i + 1
            }
        }
    }

    // Assign collected positionals to declared slots, in order.
    let mut pi = 0
    for pa in pargs {
        let name = map_get(pa, "name")
        if pi < len(pos) => {
            out = map_set(out, name, pos[pi])
            pi = pi + 1
        } else => {
            if opt(pa, "required", false) => {
                return Err("missing required argument: <" + name + ">")
            }
            if map_has_key(pa, "default") => { out = map_set(out, name, map_get(pa, "default")) }
        }
    }
    if pi < len(pos) => { return Err("unexpected argument: " + pos[pi]) }

    // Required typed flags must have landed a value.
    for f in flags {
        if opt(f, "required", false) && map_has_key(out, map_get(f, "name")) == false => {
            return Err("missing required flag: --" + map_get(f, "name"))
        }
    }
    Ok(out)
}

// ── public parse: dispatches subcommands, else parses directly ─────────

share fn parse(spec, argv) = {
    let cmds = cmds_of(spec)
    if len(cmds) == 0 => parse_core(spec, argv)
    else => {
        if len(argv) == 0 => Ok(#{ "help": true, "command": "" })
        else => {
            let head = argv[0]
            if head == "-h" || head == "--help" => Ok(#{ "help": true, "command": "" })
            else => {
                let matches = filter(cmds, (c) => map_get(c, "name") == head)
                if len(matches) == 0 => Err("unknown command: " + head)
                else => match parse_core(matches[0], skip(argv, 1)) {
                    Err(e) => Err(e),
                    Ok(sub) => Ok(map_set(sub, "command", head))
                }
            }
        }
    }
}

// ── help / usage text ──────────────────────────────────────────────────

fn pad(s, w) = str.pad_end(s, w, " ")

share fn help(spec) = {
    let name = opt(spec, "name", "program")
    let about = opt(spec, "about", "")
    let cmds = cmds_of(spec)
    let flags = flags_of(spec)
    let pargs = args_of(spec)

    let mut out = name
    if about != "" => { out = out + " — " + about }
    out = out + "\n\n"

    if len(cmds) > 0 => {
        out = out + "Usage: " + name + " <command> [options]\n\nCommands:\n"
        for c in cmds {
            out = out + "  " + pad(map_get(c, "name"), 12) + opt(c, "about", "") + "\n"
        }
    } else => {
        let mut usage = "Usage: " + name
        if len(flags) > 0 => { usage = usage + " [options]" }
        for pa in pargs {
            let nm = map_get(pa, "name")
            usage = usage + (if opt(pa, "required", false) => " <" + nm + ">" else => " [" + nm + "]")
        }
        out = out + usage + "\n"
        if len(pargs) > 0 => {
            out = out + "\nArguments:\n"
            for pa in pargs {
                out = out + "  " + pad(map_get(pa, "name"), 14) + opt(pa, "help", "") + "\n"
            }
        }
    }

    out = out + "\nOptions:\n"
    for f in flags {
        let short = if map_has_key(f, "short") => "-" + map_get(f, "short") + ", " else => "    "
        let val = if ftype(f) == "bool" => "" else => " <" + map_get(f, "name") + ">"
        let head = short + "--" + map_get(f, "name") + val
        let mut line = "  " + pad(head, 24) + opt(f, "help", "")
        if map_has_key(f, "default") => {
            line = line + " (default: " + show(map_get(f, "default")) + ")"
        }
        out = out + line + "\n"
    }
    out + "  " + pad("-h, --help", 24) + "show this help\n"
}
