//! The idioms ratchet: runs tools/idioms/rules.ol over a project and
//! holds each file to the count of findings it had when the baseline
//! was taken. A file may only get better. A file the baseline does not
//! list is held to zero, so new files start clean.
//!
//!     olang tools/idioms/ratchet.ol check  <project>
//!     olang tools/idioms/ratchet.ol update <project>
//!
//! `check` exits 1 when any file has more findings of a rule than its
//! baseline, naming each one. `update` writes the baseline from what the
//! project has now: run it after a file improves, so the lower count
//! holds. The baseline is the project's own `tests/idioms_baseline.tsv`,
//! one `file<TAB>rule<TAB>count` a line, sorted, so a diff of it reads
//! as what changed.

let HERE = fs.dirname(unwrap_or(fs.abs_path(os.args()[0]), os.args()[0]))
let RULES = fs.join(HERE, "rules.ol")
let RULE_NAMES = ["rule_map_set_chain", "rule_record_as_map", "rule_kind_chain", "rule_swallowed_err", "rule_long_fn"]

// Code no one maintains by hand: fixtures are wrong on purpose, and the
// rest is built or copied in.
let SKIP = ["/fixtures/", "/.build/", "/target/", "/dist/", "/shelf/", "/node_modules/", "/vendor/"]

fn olang_bin() = unwrap_or(os.get_env("OLANG"), "olang")

fn kept(path) = !any(SKIP, (s) => str.contains("/" + path, s))

// One finding line, `face/app.ol:312: [rule_long_fn] …`, as `(file, rule)`;
// () for any other line of the check's output.
fn finding(line) = {
    let c = unwrap_or(re.captures("^(?:\\./)?([^:\\s]+\\.ol):\\d+: \\[(rule_[a-z_]+)\\]", line), ())
    if c == () || !contains(RULE_NAMES, c[2]) || !kept(c[1]) => () else => (c[1], c[2])
}

/// The project's findings now, counted: `#{ "file\trule": n }`.
fn current(project) = {
    let r = os.exec([olang_bin(), "check", "--rules", RULES, "."], #{ "cwd": project, "env": #{ "NO_COLOR": "1" } })?
    let lines = str.lines(r.stdout + "\n" + r.stderr)
    // a rule that raised, or a rules file that did not load, counts
    // nothing: say so rather than report a clean project
    let broken = filter(lines, (l) => str.contains(l, "rule raised an error") || str.contains(l, "rules file"))
    if len(broken) > 0 => return Err(join(take(broken, 3), "\n"))
    let found = lines
        |> map(finding)
        |> filter((f) => f != ())
    Ok(fold(found, #{}, (counts, (file, rule)) => map_update(counts, file + "\t" + rule, (n) => (n ?? 0) + 1)))
}

fn baseline_path(project) = fs.join(project, "tests", "idioms_baseline.tsv")

fn read_baseline(project) = match fs.read_file(baseline_path(project)) {
    Err(_) => #{},
    Ok(text) => str.lines(text)
        |> filter((l) => l != "")
        |> fold(#{}, (counts, l) => {
            let parts = str.split(l, "\t")
            map_set(counts, parts[0] + "\t" + parts[1], unwrap(str.parse_int(parts[2])))
        }),
}

fn write_baseline(project, counts) = {
    let text = map_keys(counts)
        |> sort
        |> map((k) => k + "\t" + to_string(counts[k]))
        |> join("\n")
    fs.create_dir_all(fs.join(project, "tests"))?
    fs.write_file(baseline_path(project), text + "\n")
}

fn total(counts) = fold(map_values(counts), 0, (s, n) => s + n)

/// Each file and rule that has more findings than the baseline allows.
fn worse(now, base) = map_keys(now)
    |> sort
    |> filter((k) => now[k] > (base[k] ?? 0))
    |> map((k) => { key: k, now: now[k], was: base[k] ?? 0 })

fn better(now, base) = map_keys(base) |> filter((k) => (now[k] ?? 0) < base[k]) |> len

fn report_worse(rows) = for r in rows {
    let parts = str.split(r.key, "\t")
    println(`${parts[0]}: ${parts[1]} ${r.was} → ${r.now}`)
}

fn check(project) = {
    let now = current(project)?
    let base = read_baseline(project)
    let rows = worse(now, base)
    let gained = better(now, base)
    if len(rows) > 0 => {
        println(`idioms: ${len(rows)} file and rule pairs got worse than the baseline (${baseline_path(project)}):`)
        report_worse(rows)
        println("Write the idiom the finding names, or mark a deliberate exception with `// allow: rule_<name>`.")
        return Ok(1)
    }
    println(`idioms: no file got worse (${total(now)} findings, baseline ${total(base)})`)
    if gained > 0 => println(`${gained} file and rule pairs improved: run \`update\` so the lower counts hold`)
    Ok(0)
}

fn update(project) = {
    let now = current(project)?
    write_baseline(project, now)?
    println(`idioms: baseline ${baseline_path(project)} written (${total(now)} findings in ${len(map_keys(now))} file and rule pairs)`)
    Ok(0)
}

fn usage() = {
    println("usage: olang tools/idioms/ratchet.ol check|update <project>")
    2
}

fn main(args) = {
    if len(args) < 3 => return usage()
    let project = unwrap_or(fs.abs_path(args[2]), args[2])
    let run = match args[1] {
        "check" => check(project),
        "update" => update(project),
        _ => return usage(),
    }
    match run {
        Ok(code) => code,
        Err(e) => {
            println("idioms: " + show(e))
            2
        },
    }
}

os.exit(main(os.args()))
