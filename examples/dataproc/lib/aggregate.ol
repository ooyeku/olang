// Total revenue per region: region -> summed revenue.
share fn revenue_by_region(records) = records
    |> fold(#{}, (acc, r) => {
        let prior = if map_has_key(acc, r.region) => map_get(acc, r.region) else => 0.0
        map_set(acc, r.region, prior + r.revenue)
    })

// The record with the highest single revenue.
share fn top_sale(records) = col.max_by(records, (r) => r.revenue)
