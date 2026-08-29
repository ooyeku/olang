use parse { route_of }

// Count log lines by severity level.
share fn level_counts(records) = col.count_by(records, (r) => r.level)

// The messages of every ERROR record.
share fn errors(records) = records
    |> filter((r) => r.level == "ERROR")
    |> map((r) => r.message)

// Top routes by request count: a list of (route, count) sorted desc.
share fn top_routes(records) = {
    let routed = records
        |> map((r) => route_of(r.message))
        |> filter((route) => route != "")
    let counts = col.frequencies(routed)
    map_keys(counts)
        |> map((route) => (route, map_get(counts, route)))
        |> col.sort_by((pair) => { let (r, c) = pair; 0 - c })
}
