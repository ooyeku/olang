//! survey — a codebase surveyor, and the flagship of olang's command-line
//! story. It turns a directory into a report: files and lines by
//! language, a bar chart of the biggest ones, and the largest files —
//! the terminal counterpart of the browser data-viz apps.
//!
//! It is one self-contained olang program that leans on the whole
//! command-line toolkit: `cli` for its subcommands and flags, `term`
//! for color, aligned tables, and a live progress bar, `fs` to walk the
//! tree. Because it uses only stdlib and the embedded packages, it
//! bundles into a single executable:
//!
//!   olang build examples/tools/survey/main.ol -o survey
//!   ./survey langs src/ --top 8
//!
//! and it documents itself:  olang doc examples/tools/survey/main.ol

use cli
use term

// ── the command surface, as one declarative spec ───────────────────────

let path_arg = #{ "name": "path", "default": ".", "help": "directory to survey" }

let spec = #{
    "name": "survey", "about": "survey a codebase — files, lines, languages",
    "commands": [
        #{ "name": "summary", "about": "totals and a bar chart of the top languages",
           "args": [path_arg] },
        #{ "name": "langs", "about": "every language, by line count",
           "args": [path_arg],
           "flags": [#{ "name": "top", "short": "n", "type": "int", "default": 12,
                         "help": "show at most N languages" }] },
        #{ "name": "files", "about": "the largest files by line count",
           "args": [path_arg],
           "flags": [#{ "name": "top", "short": "n", "type": "int", "default": 10,
                         "help": "show at most N files" }] }
    ]
}

// ── extension → language, with a color for each ─────────────────────────

fn lang_of(ext) = {
    let names = #{
        "ol": "olang", "rs": "Rust", "js": "JavaScript", "mjs": "JavaScript",
        "ts": "TypeScript", "py": "Python", "go": "Go", "c": "C", "h": "C",
        "cpp": "C++", "java": "Java", "rb": "Ruby", "sh": "Shell",
        "md": "Markdown", "html": "HTML", "css": "CSS", "json": "JSON",
        "toml": "TOML", "yaml": "YAML", "yml": "YAML", "svg": "SVG",
        "pest": "Grammar", "lua": "Lua"
    }
    if map_has_key(names, ext) => map_get(names, ext) else => ""
}

fn hue(lang) = {
    let colors = #{
        "olang": "green", "Rust": "yellow", "JavaScript": "yellow",
        "TypeScript": "blue", "Python": "blue", "Markdown": "cyan",
        "HTML": "magenta", "CSS": "magenta", "JSON": "gray", "TOML": "gray",
        "Shell": "green", "SVG": "magenta", "Grammar": "cyan"
    }
    if map_has_key(colors, lang) => map_get(colors, lang) else => "white"
}

fn paint(lang, s) = term.style(s, #{ "fg": hue(lang) })

// ── scan: walk the tree once, collecting per-language and per-file counts ─

fn count_lines(path) = len(str.lines(unwrap_or(fs.read_file(path), "")))

// Build artifacts, VCS metadata, and vendored trees are never source.
fn ignored(f) =
    str.contains(f, "/target/") || str.starts_with(f, "target/") ||
    str.contains(f, "/.git/") || str.starts_with(f, ".git/") ||
    str.contains(f, "/node_modules/") || str.contains(f, "/.claude/") ||
    str.contains(f, "/dist/") || str.contains(f, "/.svelte-kit/") ||
    str.contains(f, "/build/")

fn scan(root) = {
    let files = filter(unwrap_or(fs.walk(root), []),
        (f) => lang_of(fs.ext(f)) != "" && ignored(f) == false)
    let total = len(files)
    // A live progress bar only makes sense on a terminal; piped or
    // redirected output stays clean.
    let show_progress = os.is_tty() && total > 40

    let mut by_lang = #{}    // language -> [files, lines]
    let mut per_file = []    // [path, lines]
    let mut done = 0
    for f in files {
        let lang = lang_of(fs.ext(f))
        let n = count_lines(f)
        let cur = if map_has_key(by_lang, lang) => map_get(by_lang, lang) else => [0, 0]
        by_lang = map_set(by_lang, lang, [cur[0] + 1, cur[1] + n])
        per_file = per_file + [[f, n]]

        done = done + 1
        if show_progress && done % 8 == 0 => {
            print("\r  scanning " + term.bar(to_float(done) / to_float(total), 24))
            os.flush()
        }
    }
    if show_progress => { print("\r" + str.repeat(" ", 40) + "\r") }
    #{ "by_lang": by_lang, "per_file": per_file, "files": total }
}

// ── shared helpers ──────────────────────────────────────────────────────

fn total_lines(report) = {
    let mut sum = 0
    for (lang, fl) in entries(map_get(report, "by_lang")) {
        sum = sum + fl[1]
    }
    sum
}

// Languages as [lang, files, lines], sorted by lines descending.
fn ranked(report) = {
    let rows = map(entries(map_get(report, "by_lang")),
        (e) => [e[0], e[1][0], e[1][1]])
    // sort by lines desc: sort by negative lines via a keyed comparison
    sort_by_lines(rows)
}

fn sort_by_lines(rows) = {
    // insertion sort — the language set is tiny.
    let mut out = []
    for r in rows {
        let mut i = 0
        let mut placed = false
        let mut result = []
        for o in out {
            if placed == false && r[2] > o[2] => {
                result = result + [r]
                placed = true
            }
            result = result + [o]
        }
        if placed == false => { result = result + [r] }
        out = result
    }
    out
}

fn commas(n) = {
    let s = show(n)
    let mut out = ""
    let mut i = 0
    let digits = str.length(s)
    for ch in s {
        if i > 0 && (digits - i) % 3 == 0 => { out = out + "," }
        out = out + ch
        i = i + 1
    }
    out
}

// ── the three views ─────────────────────────────────────────────────────

fn view_summary(report) = {
    let files = map_get(report, "files")
    let lines = total_lines(report)
    println(term.bold("survey") + term.dim("  " + show(files) + " files · "
        + commas(lines) + " lines"))
    println("")
    let langs = ranked(report)
    let top = if len(langs) > 6 => take(langs, 6) else => langs
    let widest = if len(top) > 0 => top[0][2] else => 1
    for row in top {
        let frac = to_float(row[2]) / to_float(widest)
        println("  " + paint(row[0], str.pad_end(row[0], 12, " ")) + " "
            + paint(row[0], term.bar(frac, 22))
            + term.dim("  " + commas(row[2])))
    }
}

fn view_langs(report, limit) = {
    let lines = total_lines(report)
    let langs = ranked(report)
    let shown = if len(langs) > limit => take(langs, limit) else => langs
    let rows = map(shown, (row) => [
        paint(row[0], row[0]),
        commas(row[1]),
        commas(row[2]),
        show(to_int(to_float(row[2]) * 100.0 / to_float(lines))) + "%"
    ])
    println(term.table(["language", "files", "lines", "share"], rows))
}

fn view_files(report, limit) = {
    let ranked_files = sort_files(map_get(report, "per_file"))
    let shown = if len(ranked_files) > limit => take(ranked_files, limit) else => ranked_files
    let rows = map(shown, (r) => [paint(lang_of(fs.ext(r[0])), r[0]), commas(r[1])])
    println(term.table(["file", "lines"], rows))
}

fn sort_files(files) = {
    let mut out = []
    for r in files {
        let mut placed = false
        let mut result = []
        for o in out {
            if placed == false && r[1] > o[1] => { result = result + [r]; placed = true }
            result = result + [o]
        }
        if placed == false => { result = result + [r] }
        out = result
    }
    out
}

// ── dispatch ────────────────────────────────────────────────────────────

fn run(a) = {
    let path = if map_has_key(a, "path") => map_get(a, "path") else => "."
    if fs.is_dir(path) == false => {
        println(term.red("survey: ") + "not a directory: " + path)
        os.exit(1)
    }
    let report = scan(path)
    if map_get(report, "files") == 0 => { println(term.dim("no source files under " + path)) }
    else => {
        let cmd = if map_has_key(a, "command") && map_get(a, "command") != ""
            => map_get(a, "command") else => "summary"
        let top = if map_has_key(a, "top") => map_get(a, "top") else => 10
        match cmd {
            "langs" => view_langs(report, top),
            "files" => view_files(report, top),
            _ => view_summary(report)
        }
    }
}

let argv = cli.args()
match cli.parse(spec, argv) {
    Err(e) => {
        println(term.red("survey: ") + e)
        os.exit(2)
    },
    Ok(a) => {
        // `-h`/`--help` explicitly asked → usage. No args at all → survey
        // the current directory (the friendly default).
        if map_get(a, "help") && len(argv) > 0 => println(cli.help(spec))
        else => run(a)
    }
}
