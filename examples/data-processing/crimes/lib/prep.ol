//! prep — feature engineering over the raw record.
//!
//! The extract's timestamps are "MM/DD/YYYY HH:MM:SS AM" strings; the
//! derivations below run one lambda over all 8.6 million of them, the
//! kind of bulk per-cell work the engine promotes and compiles.

/// Months ("01".."12") and hours (0..23) from the full Date column.
share fn derive_time(dates) = {
    let months = map(dates, (d) => str.substring(d, 0, 2))
    let hours = map(dates, (d) => {
        let h = unwrap(str.parse_int(str.substring(d, 11, 13)))
        let pm = str.substring(d, 20, 22) == "PM"
        if h == 12 => { if pm => 12 else => 0 }
        else if pm => h + 12
        else => h
    })
    #{ "months": months, "hours": hours }
}

/// The share of `labels` equal to `value` — a readable rate helper.
share fn label_share(labels, value) = {
    let mut hits = 0
    for l in labels { if l == value => { hits = hits + 1 } }
    to_float(hits) / to_float(len(labels))
}

/// One-hot feature columns: for each category in `cats`, a 0.0/1.0
/// column marking the rows of `values` that equal it.
share fn one_hot(values, cats) =
    map(cats, (c) => map(values, (v) => if v == c => 1.0 else => 0.0))

test "derive_time reads the extract's timestamp format" {
    let tm = derive_time([
        "07/29/2022 03:39:00 AM",
        "01/05/2015 12:10:00 AM",
        "11/20/2019 12:00:00 PM",
        "03/02/2008 11:59:00 PM"
    ])
    assert_eq(map_get(tm, "months"), ["07", "01", "11", "03"])
    assert_eq(map_get(tm, "hours"), [3, 0, 12, 23])
}

test "one_hot marks exactly the matching rows" {
    let cols = one_hot(["a", "b", "a", "c"], ["a", "b"])
    assert_eq(cols[0], [1.0, 0.0, 1.0, 0.0])
    assert_eq(cols[1], [0.0, 1.0, 0.0, 0.0])
}

test "label_share counts a rate" {
    assert_eq(label_share([1, 2, 1, 1], 1), 0.75)
}
