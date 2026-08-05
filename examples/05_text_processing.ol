// ═══════════════════════════════════════════════════════════════════
// 05 — Text Processing
// Real text work with the `str` and `re` modules: parsing, validation,
// extraction, tokenizing, and a word-frequency report.
// ═══════════════════════════════════════════════════════════════════

println("═══ text processing ═══")

// ── Word frequency over a paragraph ─────────────────────────────────
let paragraph = "the quick brown fox jumps over the lazy dog the fox runs"
let words = str.words(str.to_lower(paragraph))
let freq = words |> fold(#{}, (acc, w) => {
    let n = if map_has_key(acc, w) => map_get(acc, w) else => 0
    map_set(acc, w, n + 1)
})
println("── word frequency ──")
for word in sort(map_keys(freq)) {
    let count = map_get(freq, word)
    println(`  ${str.pad_end(word, 8, " ")} ${str.repeat("#", count)} ${count}`)
}

// ── Parsing a structured log line with regex captures ──────────────
println("── log parsing ──")
let logs = [
    "2026-08-05 14:30:01 ERROR  database connection failed",
    "2026-08-05 14:30:02 INFO   retrying in 5s",
    "2026-08-05 14:30:07 WARN   slow query detected",
    "2026-08-05 14:30:08 INFO   request completed"
]
let pattern = "^(\\S+) (\\S+) (\\w+)\\s+(.+)$"
let by_level = logs |> fold(#{}, (acc, line) => {
    match re.captures(pattern, line) {
        Ok(groups) => {
            if len(groups) >= 4 => {
                let level = groups[3]
                let n = if map_has_key(acc, level) => map_get(acc, level) else => 0
                map_set(acc, level, n + 1)
            } else => acc
        },
        Err(e) => acc
    }
})
for level in sort(map_keys(by_level)) {
    println(`  ${str.pad_end(level, 6, " ")} ${map_get(by_level, level)} message(s)`)
}

// ── Field extraction: pull all emails from free text ───────────────
println("── extraction ──")
let text = "contact ann@acme.io or bob@acme.io, cc: ops@acme.io for support"
let emails = unwrap(re.find_all("[\\w.]+@[\\w.]+", text))
println(`found ${len(emails)} emails: ${emails}`)

// ── Validation: which inputs are well-formed? ──────────────────────
println("── validation ──")
fn is_valid_phone(s) = unwrap(re.is_match("^\\d{3}-\\d{3}-\\d{4}$", s))
let candidates = ["555-123-4567", "12-34", "999-888-7777", "phone"]
for c in candidates {
    let mark = if is_valid_phone(c) => "valid  " else => "invalid"
    println(`  ${mark}  ${c}`)
}

// ── A tiny template engine: {{name}} substitution ──────────────────
println("── template engine ──")
fn render(template, vars) = {
    map_keys(vars) |> fold(template, (acc, key) =>
        str.replace(acc, "{{" + key + "}}", map_get(vars, key)))
}
let out = render(
    "Dear {{name}}, your order {{id}} ships to {{city}}.",
    #{"name": "Ann", "id": "A-4021", "city": "Portland"}
)
println(`  ${out}`)

// ── CSV-ish parsing with str functions ─────────────────────────────
println("── record parsing ──")
let csv = "Ann,30,Portland;Bob,25,Denver;Cy,41,Austin"
let records = str.split(csv, ";")
    |> map((row) => {
        let cols = str.split(row, ",")
        { name: cols[0], age: unwrap(str.parse_int(cols[1])), city: cols[2] }
    })
let avg_age = (records |> fold(0, (a, r) => a + r.age)) / len(records)
for r in records {
    println(`  ${str.pad_end(r.name, 5, " ")} age ${r.age}, ${r.city}`)
}
println(`  average age: ${avg_age}`)

// ── Slug generation: title -> url-safe slug ────────────────────────
println("── slugify ──")
fn slugify(title) = {
    let lower = str.to_lower(str.trim(title))
    let hyphenated = unwrap(re.replace_all("[^a-z0-9]+", lower, "-"))
    unwrap(re.replace_all("^-|-$", hyphenated, ""))
}
for title in ["Hello, World!", "  Trailing Spaces  ", "Rust & olang: 2026"] {
    println(`  "${title}" -> ${slugify(title)}`)
}

println("═══ text processing complete ═══")
