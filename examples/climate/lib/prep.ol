//! prep — from raw OWID tables to analysis-ready frames.
//!
//! Both datasets carry one row per country-year, with aggregate rows
//! (World, European Union, international transport) interleaved among
//! real countries. Aggregates are the right rows for global trend
//! questions and the wrong rows for cross-country ones, so the split
//! is the first honest step of any analysis on this data.

/// Aggregate rows that carry an iso_code and would otherwise pass for
/// countries. Continents and income groups have no iso_code and fall
/// to the `iso_code != ""` filter.
fn aggregate_rows() = ods.frame_from_records([
    #{ "country": "World" },
    #{ "country": "European Union (27)" },
    #{ "country": "International transport" },
    #{ "country": "International aviation" },
    #{ "country": "International shipping" },
    #{ "country": "Kuwaiti Oil Fires" }
])

/// The country-only view: rows with a real iso_code, minus the named
/// aggregates — an anti-join keeps the exclusion list data, not code.
share fn countries(frame) = {
    let with_iso = ods.filter(frame, frame["iso_code"] != "")
    ods.join_anti(with_iso, aggregate_rows(), "country")
}

/// The World aggregate: the rows global trend questions want.
share fn world(frame) = ods.filter(frame, frame["country"] == "World")

/// The latest year at which `column` has wide coverage: some series
/// (GDP especially) trail the calendar by years, so "latest" must be
/// discovered from the data rather than assumed.
share fn latest_covered_year(frame, column, min_rows) = {
    let present = ods.drop_null(ods.select(frame, ["year", column]))
    let by_year = ods.group_by(present, ["year"], [["rows", "count"]])
    let wide = ods.filter(by_year, by_year["rows"] >= min_rows)
    ods.max(wide["year"])
}

/// One year of one frame, countries only, nulls dropped in the given
/// columns — the cross-sectional slice the relationship analyses use.
share fn cross_section(frame, year, columns) = {
    let rows = ods.filter(frame, frame["year"] == year)
    ods.drop_null(ods.select(rows, columns))
}

test "countries keeps real iso rows and drops aggregates" {
    let f = ods.frame_from_records([
        #{ "country": "World", "iso_code": "OWID_WRL", "year": 2020 },
        #{ "country": "Asia", "iso_code": "", "year": 2020 },
        #{ "country": "France", "iso_code": "FRA", "year": 2020 },
        #{ "country": "Kenya", "iso_code": "KEN", "year": 2020 }
    ])
    let c = countries(f)
    assert_eq(ods.n_rows(c), 2)
    assert_eq(ods.to_list(ods.sort_by(c, "country", false)["country"]), ["France", "Kenya"])
    assert_eq(ods.n_rows(world(f)), 1)
}

test "latest_covered_year respects the coverage floor" {
    let f = ods.frame_from_records([
        #{ "year": 2020, "gdp": 1.0 }, #{ "year": 2020, "gdp": 2.0 },
        #{ "year": 2021, "gdp": 3.0 }, #{ "year": 2021, "gdp": () },
        #{ "year": 2022, "gdp": () }, #{ "year": 2022, "gdp": () }
    ])
    assert_eq(latest_covered_year(f, "gdp", 2), 2020)
    assert_eq(latest_covered_year(f, "gdp", 1), 2021)
}

test "cross_section slices one year and drops incomplete rows" {
    let f = ods.frame_from_records([
        #{ "country": "A", "year": 2020, "x": 1.0 },
        #{ "country": "B", "year": 2020, "x": () },
        #{ "country": "A", "year": 2021, "x": 3.0 }
    ])
    let s = cross_section(f, 2020, ["country", "x"])
    assert_eq(ods.n_rows(s), 1)
    assert_eq(ods.to_list(s["country"]), ["A"])
}
