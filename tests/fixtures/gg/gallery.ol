// The gg gallery: a plot of every geom, stat, scale, position, facet,
// coord and theme the grammar has, on deterministic data. Golden SVGs
// (tests/gg_gallery_test.rs) hold each in light and dark.
use gg

// a deterministic noise in [0, 1)
fn noise(i, k) = {
    let v = math.sin(to_float(i) * 12.9898 + to_float(k) * 78.233) * 43758.5453
    v - math.floor(v)
}

// an approximately normal draw (sum of uniforms)
fn normal(i, k) = (noise(i, k) + noise(i, k + 1) + noise(i, k + 2) + noise(i, k + 3) - 2.0) * 1.7

let hoods = ["Ashby", "Bellmont", "Cedar Hill", "Dunmore", "Elmwood", "Fairview"]
let n = 600
let houses = {
    let ids = range(0, n)
    let hood = map(ids, (i) => hoods[i % 6])
    let sqft = map(ids, (i) => 600.0 + 2600.0 * noise(i, 1) * noise(i, 2) + 400.0 * noise(i, 3))
    let beds = map(ids, (i) => 1 + to_int(noise(i, 4) * 4.0))
    let price = map(ids, (i) => {
        let base = 80000.0 + (i % 6) * 30000.0
        (base + sqft[i] * (140.0 + (i % 6) * 18.0)) * math.exp(normal(i, 5) * 0.12)
    })
    let year = map(ids, (i) => 1950 + to_int(noise(i, 9) * 70.0))
    ods.frame([["hood", ods.series(hood)], ["sqft", ods.series(sqft)], ["beds", ods.series(beds)],
               ["price", ods.series(price)], ["year", ods.series(year)]])
}

let months = {
    let ids = range(0, 72)
    let date = map(ids, (i) => {
        let y = 2019 + i / 12
        let m = i % 12 + 1
        to_string(y) + "-" + (if m < 10 => "0" else => "") + to_string(m) + "-01"
    })
    let sales = map(ids, (i) => 120.0 + to_float(i) * 1.6 + 25.0 * math.sin(to_float(i) * 0.5236) + normal(i, 11) * 6.0)
    let lo = map(ids, (i) => sales[i] - 12.0 - to_float(i) * 0.15)
    let hi = map(ids, (i) => sales[i] + 12.0 + to_float(i) * 0.15)
    ods.frame([["date", ods.series(date)], ["sales", ods.series(sales)], ["lo", ods.series(lo)], ["hi", ods.series(hi)]])
}

let channels = {
    let rows = fold(range(0, 24), [], (acc, i) => acc + map(["web", "store", "phone"], (c) => #{
        "month": i + 1, "channel": c,
        "revenue": (if c == "web" => 40.0 + to_float(i) * 2.0 else if c == "store" => 60.0 - to_float(i) * 0.8 else => 15.0 + 5.0 * noise(i, 3))
    }))
    ods.frame_from_records(rows)
}

let grid = {
    let rows = fold(range(0, 12), [], (acc, r) => acc + map(range(0, 7), (c) => #{
        "hour": r * 2, "day": ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][c],
        "load": math.round((math.sin(to_float(r) / 2.0) + math.cos(to_float(c) / 1.5) + 2.0) * 25.0)
    }))
    ods.frame_from_records(rows)
}

let effects = ods.frame_from_records([
    #{ "term": "sqft", "est": 0.62, "lo": 0.48, "hi": 0.76 },
    #{ "term": "beds", "est": 0.11, "lo": -0.04, "hi": 0.26 },
    #{ "term": "age", "est": -0.31, "lo": -0.45, "hi": -0.17 },
    #{ "term": "garage", "est": 0.18, "lo": 0.02, "hi": 0.34 },
    #{ "term": "pool", "est": 0.05, "lo": -0.20, "hi": 0.30 }
])

// two measures on very different scales, long (for free facets' bins)
let measures = {
    let ids = range(0, n)
    let sqft = ods.to_list(houses["sqft"])
    let years = ods.to_list(houses["year"])
    ods.frame([["measure", ods.series(map(ids, (i) => "floor area (sq ft)") + map(ids, (i) => "year built"))],
               ["value", ods.series(sqft + map(years, (y) => to_float(y)))]])
}

// two models' estimates of the same terms
let compare = ods.frame_from_records(fold(["sqft", "beds", "age", "garage", "pool"], [], (acc, t) => {
    let i = len(acc)
    acc + map(["ols", "ridge"], (m) => {
        let e = (if m == "ols" => 1.0 else => 0.8) * (0.6 - to_float(i) * 0.15) + noise(i, 40) * 0.1
        let h = 0.08 + noise(i, 41) * 0.08 + (if m == "ridge" => -0.03 else => 0.0)
        #{ "term": t, "model": m, "est": e, "lo": e - h, "hi": e + h }
    })
}))

// a few points: a gam's basis and penalty from so little
let sparse = ods.frame_from_records(map(range(0, 11), (i) => {
    let x = noise(i, 50) * 10.0
    #{ "x": x, "y": x * 0.3 + noise(i, 51) * 3.0 }
}))

share fn gallery() = [
    ["scatter-smooth", gg.plot(houses, gg.aes(x: "sqft", y: "price", color: "hood"))
        |> gg.point(alpha: 0.6)
        |> gg.smooth(method: "lm", mapping: gg.aes(group: "hood"), se: false)
        |> gg.scale_y_continuous(labels: "dollar")
        |> gg.labs(title: "Price by size", subtitle: "600 sales in six neighbourhoods", x: "floor area (sq ft)", y: "price", color: "neighbourhood")],
    ["facet-loess-log", gg.plot(houses, gg.aes(x: "sqft", y: "price"))
        |> gg.point(alpha: 0.35, size: 1)
        |> gg.smooth(method: "loess")
        |> gg.facet_wrap("hood", ncol: 3)
        |> gg.scale_y_log10(labels: "short")
        |> gg.labs(title: "Price by size, each neighbourhood", y: "price (log)")],
    ["histogram-stacked", gg.plot(houses, gg.aes(x: "price", fill: "hood")) |> gg.histogram(bins: 30)
        |> gg.scale_x_continuous(labels: "short") |> gg.labs(title: "Prices", fill: "neighbourhood")],
    ["density-groups", gg.plot(houses, gg.aes(x: "sqft", fill: "hood", color: "hood")) |> gg.density(alpha: 0.15)
        |> gg.labs(title: "Floor area, by neighbourhood")],
    ["boxplot-jitter", gg.plot(houses, gg.aes(x: "hood", y: "price")) |> gg.boxplot(outliers: false)
        |> gg.jitter(width: 0.2, alpha: 0.25, size: 0.8) |> gg.scale_y_continuous(labels: "short")
        |> gg.labs(title: "Price by neighbourhood", x: "")],
    ["violin-flip", gg.plot(houses, gg.aes(x: "hood", y: "sqft", fill: "hood")) |> gg.violin(draw_quantiles: [0.25, 0.5, 0.75])
        |> gg.coord_flip() |> gg.theme(legend_position: "none") |> gg.labs(title: "Floor area distribution")],
    ["bar-dodge", gg.plot(houses, gg.aes(x: "beds", fill: "hood")) |> gg.bar(position: "dodge")
        |> gg.scale_fill_viridis_d(option: "mako", begin: 0.25, end: 0.9) |> gg.labs(title: "Bedrooms", x: "bedrooms")],
    ["col-fill-percent", gg.plot(channels, gg.aes(x: "month", y: "revenue", fill: "channel")) |> gg.col(position: "fill")
        |> gg.scale_y_continuous(labels: "percent") |> gg.labs(title: "Revenue share by channel", y: "share")],
    ["line-dates", gg.plot(months, gg.aes(x: "date", y: "sales")) |> gg.line() |> gg.point(size: 1)
        |> gg.labs(title: "Monthly sales", caption: "Six years of monthly totals")],
    ["area-stacked", gg.plot(channels, gg.aes(x: "month", y: "revenue", fill: "channel")) |> gg.area()
        |> gg.labs(title: "Revenue by channel")],
    ["ribbon-band", gg.plot(months, gg.aes(x: "date")) |> gg.ribbon(gg.aes(ymin: "lo", ymax: "hi"))
        |> gg.line(gg.aes(y: "sales")) |> gg.scale_x_date(date_breaks: "1 year", date_labels: "%Y")
        |> gg.labs(title: "Sales with a band", y: "sales")],
    ["summary-ci", gg.plot(houses, gg.aes(x: "beds", y: "price", color: "hood"))
        |> gg.summary(fun: "mean_cl_normal") |> gg.line(stat: "summary", mapping: gg.aes(group: "hood"))
        |> gg.scale_y_continuous(labels: "short") |> gg.labs(title: "Mean price by bedrooms (95% CI)")],
    ["heatmap-text", gg.plot(grid, gg.aes(x: "day", y: "hour", fill: "load")) |> gg.tile(color: "white")
        |> gg.text(gg.aes(label: "load"), size: 9) |> gg.scale_fill_gradient(low: "#f6f1e7", high: "#c0412b")
        |> gg.scale_x_discrete(limits: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"])
        |> gg.labs(title: "Load by hour and day")],
    ["hex", gg.plot(houses, gg.aes(x: "sqft", y: "price")) |> gg.hex(bins: 18)
        |> gg.scale_y_continuous(labels: "short") |> gg.labs(title: "Hexagonal bins")],
    ["bin2d", gg.plot(houses, gg.aes(x: "sqft", y: "price")) |> gg.bin2d(bins: 20)
        |> gg.scale_fill_viridis_c(option: "plasma") |> gg.labs(title: "Rectangular bins")],
    ["ecdf", gg.plot(houses, gg.aes(x: "price", color: "hood")) |> gg.ecdf() |> gg.scale_x_log10(labels: "short")
        |> gg.labs(title: "Empirical cdf of price", y: "share at or below")],
    ["qq", gg.plot(houses, gg.aes(sample: "sqft")) |> gg.qq(alpha: 0.5) |> gg.qq_line()
        |> gg.labs(title: "Normal Q-Q of floor area")],
    ["annotations", gg.plot(houses, gg.aes(x: "sqft", y: "price")) |> gg.point(color: "grey60", alpha: 0.4)
        |> gg.hline(yintercept: 400000, linetype: "dashed") |> gg.vline(xintercept: [1000, 2500], color: "firebrick")
        |> gg.annotate("rect", xmin: 2500, xmax: 3600, ymin: 600000, ymax: 900000, fill: "gold", alpha: 0.15)
        |> gg.annotate("text", x: 3050, y: 860000, label: "the premium corner")
        |> gg.scale_y_continuous(labels: "short") |> gg.labs(title: "Reference lines and notes")],
    ["facet-grid-free", gg.plot(houses, gg.aes(x: "year", y: "price")) |> gg.point(size: 0.7, alpha: 0.5)
        |> gg.smooth(method: "gam") |> gg.facet_grid(rows: "beds", scales: "free_y")
        |> gg.scale_y_continuous(labels: "short") |> gg.theme(height: 620) |> gg.labs(title: "Price by year built, per bedroom count")],
    ["step-rug", gg.plot(months, gg.aes(x: "date", y: "sales")) |> gg.step() |> gg.rug(sides: "l", alpha: 0.5)
        |> gg.labs(title: "A staircase and a rug")],
    ["coef-flip", gg.plot(effects, gg.aes(x: "term", y: "est", ymin: "lo", ymax: "hi")) |> gg.hline(yintercept: 0, color: "grey50")
        |> gg.pointrange() |> gg.coord_flip() |> gg.labs(title: "Coefficients with 95% intervals", y: "estimate", x: "")],
    ["diverging", gg.plot(grid, gg.aes(x: "hour", y: "load", color: "load")) |> gg.point(size: 3)
        |> gg.scale_color_gradient2(midpoint: 50) |> gg.labs(title: "Diverging colour, centred at 50")],
    ["merged-legend", gg.plot(houses, gg.aes(x: "sqft", y: "price", color: "hood", shape: "hood")) |> gg.point(size: 2)
        |> gg.scale_y_continuous(labels: "short") |> gg.labs(title: "Colour and shape, one legend", color: "neighbourhood", shape: "neighbourhood")],
    ["size-legend", gg.plot(houses, gg.aes(x: "sqft", y: "price", size: "beds", color: "year")) |> gg.point(alpha: 0.5)
        |> gg.scale_y_continuous(labels: "short") |> gg.labs(title: "Size and a colour bar")],
    ["classic-bottom", gg.plot(channels, gg.aes(x: "month", y: "revenue", color: "channel", linetype: "channel")) |> gg.line()
        |> gg.scale_y_continuous(labels: "dollar") |> gg.theme("classic", legend_position: "bottom")
        |> gg.labs(title: "Classic theme, legend below", caption: "revenue in thousands")],
    ["labels", gg.plot(effects, gg.aes(x: "lo", y: "hi", label: "term")) |> gg.point() |> gg.label(nudge_y: 0.04)
        |> gg.coord_fixed() |> gg.labs(title: "Labels, fixed aspect")],
    ["raster", gg.plot(grid, gg.aes(x: "hour", y: "day", fill: "load")) |> gg.raster()
        |> gg.scale_fill_gradient(low: "#f3f0e6", high: "#7a2e1d") |> gg.labs(title: "Raster")],
    ["free-bins", gg.plot(measures, gg.aes(x: "value")) |> gg.histogram(bins: 24)
        |> gg.facet_wrap("measure", scales: "free") |> gg.labs(title: "Each panel binned over its own range", x: "")],
    ["dodge-intervals", gg.plot(compare, gg.aes(x: "term", y: "est", ymin: "lo", ymax: "hi", color: "model"))
        |> gg.hline(yintercept: 0, color: "grey50")
        |> gg.errorbar(width: 0.25, position: gg.position_dodge(width: 0.6))
        |> gg.pointrange(position: gg.position_dodge(width: 0.6))
        |> gg.coord_flip() |> gg.labs(title: "Two models' intervals, dodged", x: "", y: "estimate")],
    ["summary-styled", gg.plot(houses, gg.aes(x: "hood", y: "price"))
        |> gg.summary(fun: "mean", geom: "col", fill: "#c0412b", alpha: 0.2, width: 0.9)
        |> gg.boxplot(outliers: false, linewidth: 0.3, width: 0.4)
        |> gg.summary(fun: "mean_cl_normal", color: "#c0412b", linewidth: 0.9, alpha: 0.9)
        |> gg.scale_y_continuous(labels: "short") |> gg.labs(title: "Summaries styled: alpha, fill, linewidth", x: "")],
    ["linewidth-scale", gg.plot(channels, gg.aes(x: "month", y: "revenue", color: "channel", linewidth: "channel")) |> gg.line()
        |> gg.scale_linewidth(range: [0.3, 1.4]) |> gg.labs(title: "Line widths mapped")],
    ["constant-aes", gg.plot(effects, gg.aes(x: "term", y: "est", ymin: "lo", ymax: "hi"))
        |> gg.line(gg.aes(group: 1), alpha: 0.5) |> gg.pointrange(gg.aes(color: gg.constant("estimate")))
        |> gg.labs(title: "A line across categories (group: 1), a constant colour keyed", x: "", color: "")],
    ["gam-sparse", gg.plot(sparse, gg.aes(x: "x", y: "y")) |> gg.point() |> gg.smooth(method: "gam")
        |> gg.labs(title: "A gam through eleven points")],
    ["abline-limits", gg.plot(houses, gg.aes(x: "sqft", y: "price")) |> gg.point(alpha: 0.3, size: 0.8)
        |> gg.abline(slope: 400, intercept: -200000, color: "firebrick", linetype: "dashed")
        |> gg.coord_cartesian(xlim: [1000, 2000], ylim: [100000, 600000])
        |> gg.scale_y_continuous(labels: "short") |> gg.labs(title: "A reference line inside the zoomed panel")]
]
