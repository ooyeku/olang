// manifest — the paper trail: structured-line parsing with regex
// captures, a {{name}} template engine, charter-party contact
// extraction, and lossless JSON/CSV round-trips of the traffic log.
//
// Text is where robust systems leak; this module keeps every format
// behind a parse/render pair, each pinned by a round-trip test.

use vessels { slugify }

// ── The template engine: {{name}} substitution ──────────────────────
share fn render(template, vars) =
    map_keys(vars) |> fold(template, (acc, key) =>
        str.replace(acc, "{{" + key + "}}", map_get(vars, key)))

// ── Traffic-log lines: render and parse are inverses ────────────────
// A line: "T0042 BERTHED ABCD-123 @2 | MV Iron Duke"
share fn log_line(tick: Int, event: String, callsign: String, berth: Int, name: String) =
    `T${str.pad_start(show(tick), 4, "0")} ${str.to_upper(event)} ${callsign} @${berth} | ${name}`

share fn parse_log_line(line) = {
    let pattern = "^T(\\d+) (\\w+) ([A-Z]{4}-\\d{3}) @(-?\\d+) \\| (.+)$"
    match re.captures(pattern, line) {
        Ok(groups) => if len(groups) >= 6 => Ok(#{
            "tick": unwrap(str.parse_int(groups[1])),
            "event": groups[2],
            "callsign": groups[3],
            "berth": unwrap(str.parse_int(groups[4])),
            "name": groups[5]
        }) else => Err("wrong shape: " + line),
        Err(e) => Err(e)
    }
}

// Count log lines per event kind — the word-frequency fold, on traffic.
share fn events_by_kind(lines) =
    lines |> fold(#{}, (acc, line) => {
        match parse_log_line(line) {
            Ok(rec) => {
                let k = map_get(rec, "event")
                let n = if map_has_key(acc, k) => map_get(acc, k) else => 0
                map_set(acc, k, n + 1)
            },
            Err(e) => acc
        }
    })

// ── Charter contacts: pull the addresses out of free text ───────────
share fn extract_contacts(text) = unwrap(re.find_all("[\\w.]+@[\\w.]+", text))

// ── JSON: a day summary that round-trips ────────────────────────────
share fn day_to_json(day: Int, revenue: Float, vessels_served: Int) = {
    let doc = unwrap(json.stringify(#{
        "day": day, "revenue": revenue, "vessels": vessels_served
    }))
    doc
}

share fn day_from_json(doc) = {
    let parsed = unwrap(json.parse(doc))
    { day: map_get(parsed, "day"), revenue: map_get(parsed, "revenue"),
      vessels: map_get(parsed, "vessels") }
}

// ── CSV: the charge log as an exportable table ──────────────────────
share fn charges_to_csv(rows) = {
    let table = concat([["callsign", "kind", "units", "amount"]],
        rows |> map((r) => [map_get(r, "callsign"), map_get(r, "kind"),
                            show(map_get(r, "units")), show(map_get(r, "amount"))]))
    csv.stringify(table)
}

// A digest filename from the date, slug-safe.
share fn digest_filename(date: String) = "digest-" + slugify(date) + ".txt"

// ── Self-checks ─────────────────────────────────────────────────────
test "log lines round-trip through the parser" {
    let line = log_line(42, "berthed", "ABCD-123", 2, "MV Iron Duke")
    testing.assert_eq(line, "T0042 BERTHED ABCD-123 @2 | MV Iron Duke")
    let rec = unwrap(parse_log_line(line))
    testing.assert_eq(map_get(rec, "tick"), 42)
    testing.assert_eq(map_get(rec, "event"), "BERTHED")
    testing.assert_eq(map_get(rec, "callsign"), "ABCD-123")
    testing.assert_eq(map_get(rec, "berth"), 2)
    testing.assert_eq(map_get(rec, "name"), "MV Iron Duke")
    testing.assert_true(is_err(parse_log_line("not a log line")))
}

test "event tallies" {
    let lines = [
        log_line(1, "arrived", "AAAA-111", -1, "MV A"),
        log_line(2, "berthed", "AAAA-111", 0, "MV A"),
        log_line(3, "arrived", "BBBB-222", -1, "MV B"),
        "garbage line"
    ]
    let tally = events_by_kind(lines)
    testing.assert_eq(map_get(tally, "ARRIVED"), 2)
    testing.assert_eq(map_get(tally, "BERTHED"), 1)
    testing.assert_false(map_has_key(tally, "garbage"))
}

test "templates, contacts, json, csv" {
    let out = render("Vessel {{name}} owes {{amount}}.",
        #{ "name": "MV Tern", "amount": "$88.00" })
    testing.assert_eq(out, "Vessel MV Tern owes $88.00.")

    let contacts = extract_contacts("bill ops@harbor.io, cc master@tern.example")
    testing.assert_eq(len(contacts), 2)

    let round = day_from_json(day_to_json(3, 1234.5, 7))
    testing.assert_eq(round.day, 3)
    testing.assert_eq(round.revenue, 1234.5)
    testing.assert_eq(round.vessels, 7)

    let sheet = charges_to_csv([#{ "callsign": "AAAA-111", "kind": "bulk",
                                   "units": 40.0, "amount": 44.0 }])
    testing.assert_true(str.contains(sheet, "callsign,kind,units,amount"))
    testing.assert_true(str.contains(sheet, "AAAA-111,bulk,40.0,44.0"))

    testing.assert_eq(digest_filename("2026-08-03"), "digest-2026-08-03.txt")
}
