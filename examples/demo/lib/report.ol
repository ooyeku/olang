// report — the daily digest: metrics folded into template-rendered
// text, a revenue chart as plot SVG, both written under out/. The
// digest text is what the signing chain hashes — reports are not just
// pretty, they are the audited record.
//
// This module sits atop lib.metrics, which itself re-shares prelude
// helpers — a three-deep import chain (report -> metrics -> prelude)
// that mirrors how layered olang systems compose.

use metrics { total_by, top_entry, share_lines, money }
use manifest { render, digest_filename }
use schedule { date_of, day_of }

// ── The digest text ─────────────────────────────────────────────────
share fn build_digest(tick: Int, charge_rows, served: Int, queue_len: Int) = {
    let by_kind = total_by(charge_rows,
        (r) => map_get(r, "kind"), (r) => map_get(r, "amount"))
    let revenue = map_keys(by_kind) |> fold(0.0, (a, k) => a + map_get(by_kind, k))
    let top = top_entry(by_kind)
    let body = share_lines(by_kind) |> map((l) => "  " + l) |> join("\n")

    let text = render(join([
        "HARBORLINE DAILY DIGEST — day {{day}} ({{date}})",
        "vessels served: {{served}}   still in roads: {{queue}}",
        "revenue: {{revenue}}   top earner: {{top}}",
        "by kind:",
        "{{body}}"
    ], "\n"), #{
        "day": show(day_of(tick)),
        "date": date_of(tick),
        "served": show(served),
        "queue": show(queue_len),
        "revenue": money(revenue),
        "top": if top[0] == "" => "n/a" else => top[0],
        "body": if body == "" => "  (no traffic)" else => body
    })
    { text: text, revenue: revenue, by_kind: by_kind }
}

// ── The chart: revenue by kind, as SVG on disk ──────────────────────
share fn revenue_chart(by_kind) = {
    let kinds = sort(map_keys(by_kind))
    if len(kinds) == 0 => ""
    else => plot.bar(kinds, ods.series(kinds |> map((k) => map_get(by_kind, k))), #{
        "title": "revenue by cargo kind",
        "width": 520, "height": 300
    })
}

// ── Writing the day out ─────────────────────────────────────────────
share fn write_digest(out_dir: String, tick: Int, digest) = {
    unwrap(fs.create_dir_all(out_dir))
    let date = date_of(tick)
    let txt_path = out_dir + "/" + digest_filename(date)
    unwrap(fs.write_file(txt_path, digest.text))
    let svg = revenue_chart(digest.by_kind)
    if svg != "" => {
        unwrap(fs.write_file(out_dir + "/revenue-" + date + ".svg", svg))
    }
    txt_path
}

// ── Self-checks ─────────────────────────────────────────────────────
test "a digest renders and its chart is real svg" {
    let rows = [
        #{ "callsign": "AAAA-111", "kind": "container", "units": 40.0, "amount": 100.0 },
        #{ "callsign": "BBBB-222", "kind": "bulk", "units": 60.0, "amount": 66.0 }
    ]
    let d = build_digest(30, rows, 2, 1)
    testing.assert_eq(d.revenue, 166.0)
    testing.assert_true(str.contains(d.text, "day 1 (2026-08-02)"))
    testing.assert_true(str.contains(d.text, "vessels served: 2"))
    testing.assert_true(str.contains(d.text, "container: $100.00"))
    let svg = revenue_chart(d.by_kind)
    testing.assert_true(str.starts_with(svg, "<svg"))
    testing.assert_true(str.contains(svg, "revenue by cargo kind"))
}

test "an empty day still reports" {
    let d = build_digest(0, [], 0, 0)
    testing.assert_eq(d.revenue, 0.0)
    testing.assert_true(str.contains(d.text, "(no traffic)"))
    testing.assert_eq(revenue_chart(d.by_kind), "")
}

test "digests land on disk" {
    let dir = unwrap(os.temp_dir()) + "/harborline_report_test"
    let rows = [#{ "callsign": "AAAA-111", "kind": "reefer", "units": 8.0, "amount": 90.0 }]
    let path = write_digest(dir, 54, build_digest(54, rows, 1, 0))
    testing.assert_true(unwrap(fs.exists(path)))
    let back = unwrap(fs.read_file(path))
    testing.assert_true(str.contains(back, "reefer: $90.00"))
    unwrap(fs.remove_dir_all(dir))
}
