// gg — a grammar of graphics (`use gg`), ggplot2's way of saying a plot.
//
// A plot is a VALUE: pure data built in steps, each subject-first so it
// pipes. Nothing here computes: stats, scales, positions, facets,
// legends and layout are native (plot.gg), and a notebook or the REPL
// draws a plot that is a cell's value natively, with hover and zoom.
//
//   gg.plot(houses, gg.aes(x: "sqft", y: "price", color: "neighbourhood"))
//       |> gg.point(alpha: 0.4)
//       |> gg.smooth(method: "lm")
//       |> gg.facet_wrap("neighbourhood", ncol: 3)
//       |> gg.scale_y_log10()
//       |> gg.labs(title: "Price by size", y: "price ($)")
//
// A mapping names columns (or holds a Series, or a stat's variable:
// `gg.after_stat("density")`); a constant goes to the layer
// (`gg.point(color: "steelblue", size: 2)`). gg.svg(p) is the SVG text,
// gg.save(p, path) writes it.

// ── plumbing ───────────────────────────────────────────────────────────

// the native renderer (this module's own `plot` shadows its name)
let native = plot

fn put(m, k, v) = if v == () => m else => map_set(m, k, v)

fn pairs_map(pairs) = fold(pairs, #{}, (m, kv) => put(m, kv[0], kv[1]))

fn frame_of(data) =
    if data == () => ()
    else if typeof(data) == "List" => ods.frame_from_records(data)
    else => data

fn check_plot(p, what) =
    if typeof(p) != "Map" || !map_has_key(p, "$gg") =>
        panic("gg." + what + ": the first argument is a plot (gg.plot(data, gg.aes(...)) |> gg." + what + "(...)), got " + typeof(p))
    else => p

// A layer added: its position given as a name or a gg.position_*(…) map.
fn add(p, geom, stat, position, mapping, data, inherit, legend, pairs) = {
    check_plot(p, geom)
    let mut params = pairs_map(pairs)
    let mut pos = position
    if typeof(position) == "Map" => {
        pos = position.name
        for k in map_keys(position) {
            if k != "name" => { params = map_set(params, k, map_get(position, k)) }
        }
    }
    let layer = #{ "geom": geom, "stat": stat, "position": pos, "mapping": mapping, "data": frame_of(data),
                   "inherit": inherit, "params": params, "legend": legend }
    map_set(p, "layers", p.layers + [layer])
}

// A scale added (one per aesthetic: a later one replaces it).
fn add_scale(p, aes, kind, pairs) = {
    check_plot(p, "scale")
    let kept = filter(p.scales, (s) => s.aes != aes)
    map_set(p, "scales", kept + [#{ "aes": aes, "kind": kind, "params": pairs_map(pairs) }])
}

fn pick(color, colour) = color ?? colour

// ── the plot and its mapping ───────────────────────────────────────────

/// A plot of `data` (a Frame, or records) with a default mapping.
share fn plot(data = (), mapping = ()) =
    #{ "$gg": 1, "data": frame_of(data), "mapping": mapping, "layers": [], "scales": [],
       "facet": (), "coord": (), "labels": #{}, "theme": #{} }

/// Aesthetic mappings: each a column's name, a Series, or after_stat(…).
share fn aes(x = (), y = (), color = (), colour = (), fill = (), size = (), shape = (), alpha = (),
             linetype = (), group = (), label = (), xmin = (), xmax = (), ymin = (), ymax = (),
             xend = (), yend = (), weight = (), sample = (), xintercept = (), yintercept = ()) =
    pairs_map([["x", x], ["y", y], ["color", pick(color, colour)], ["fill", fill], ["size", size],
               ["shape", shape], ["alpha", alpha], ["linetype", linetype], ["group", group],
               ["label", label], ["xmin", xmin], ["xmax", xmax], ["ymin", ymin], ["ymax", ymax],
               ["xend", xend], ["yend", yend], ["weight", weight], ["sample", sample],
               ["xintercept", xintercept], ["yintercept", yintercept]])

/// A variable the layer's stat computes, as a mapping (`y: gg.after_stat("density")`).
share fn after_stat(name) = "after_stat(" + name + ")"

/// Any geom with any stat and position: the general form of every layer.
share fn layer(p, geom, stat = (), position = (), mapping = (), data = (), params = #{}, inherit = true, legend = true) =
    add(p, geom, stat, position, mapping, data, inherit, legend, map(map_keys(params), (k) => [k, map_get(params, k)]))

// ── geoms ──────────────────────────────────────────────────────────────

/// Points (a scatter plot).
share fn point(p, mapping = (), data = (), color = (), colour = (), fill = (), size = (), shape = (), alpha = (),
               stroke = (), position = (), stat = (), inherit = true, legend = true) =
    add(p, "point", stat, position, mapping, data, inherit, legend,
        [["color", pick(color, colour)], ["fill", fill], ["size", size], ["shape", shape], ["alpha", alpha], ["stroke", stroke]])

/// Points jittered (`width`, `height`: the spread; `seed`).
share fn jitter(p, mapping = (), data = (), width = (), height = (), seed = (), color = (), colour = (), size = (),
                shape = (), alpha = (), inherit = true, legend = true) =
    add(p, "jitter", (), (), mapping, data, inherit, legend,
        [["width", width], ["height", height], ["seed", seed], ["color", pick(color, colour)], ["size", size], ["shape", shape], ["alpha", alpha]])

fn line_params(color, colour, linewidth, linetype, alpha) =
    [["color", pick(color, colour)], ["linewidth", linewidth], ["linetype", linetype], ["alpha", alpha]]

/// Lines through the points in x order, one per group.
share fn line(p, mapping = (), data = (), color = (), colour = (), linewidth = (), linetype = (), alpha = (),
              stat = (), inherit = true, legend = true) =
    add(p, "line", stat, (), mapping, data, inherit, legend, line_params(color, colour, linewidth, linetype, alpha))

/// Lines through the points in the data's order.
share fn path(p, mapping = (), data = (), color = (), colour = (), linewidth = (), linetype = (), alpha = (),
              inherit = true, legend = true) =
    add(p, "path", (), (), mapping, data, inherit, legend, line_params(color, colour, linewidth, linetype, alpha))

/// A staircase through the points (`direction`: "hv" or "vh").
share fn step(p, mapping = (), data = (), direction = (), color = (), colour = (), linewidth = (), linetype = (),
              inherit = true, legend = true) =
    add(p, "step", (), (), mapping, data, inherit, legend,
        line_params(color, colour, linewidth, linetype, ()) + [["direction", direction]])

/// An area from zero to y, stacked by group.
share fn area(p, mapping = (), data = (), fill = (), color = (), colour = (), alpha = (), position = (),
              inherit = true, legend = true) =
    add(p, "area", (), position, mapping, data, inherit, legend, [["fill", fill], ["color", pick(color, colour)], ["alpha", alpha]])

/// A band from ymin to ymax.
share fn ribbon(p, mapping = (), data = (), fill = (), color = (), colour = (), alpha = (), inherit = true, legend = true) =
    add(p, "ribbon", (), (), mapping, data, inherit, legend, [["fill", fill], ["color", pick(color, colour)], ["alpha", alpha]])

/// Bars counting each x (stacked by group; `position: "dodge"` or "fill").
share fn bar(p, mapping = (), data = (), fill = (), color = (), colour = (), alpha = (), width = (), position = (),
             stat = (), inherit = true, legend = true) =
    add(p, "bar", stat, position, mapping, data, inherit, legend,
        [["fill", fill], ["color", pick(color, colour)], ["alpha", alpha], ["width", width]])

/// Bars of height y.
share fn col(p, mapping = (), data = (), fill = (), color = (), colour = (), alpha = (), width = (), position = (),
             inherit = true, legend = true) =
    add(p, "col", (), position, mapping, data, inherit, legend,
        [["fill", fill], ["color", pick(color, colour)], ["alpha", alpha], ["width", width]])

/// A histogram: x binned (`bins`, `binwidth`, `boundary`, `center`, `breaks`).
share fn histogram(p, mapping = (), data = (), bins = (), binwidth = (), boundary = (), center = (), breaks = (),
                   fill = (), color = (), colour = (), alpha = (), position = (), inherit = true, legend = true) =
    add(p, "histogram", (), position, mapping, data, inherit, legend,
        [["bins", bins], ["binwidth", binwidth], ["boundary", boundary], ["center", center], ["breaks", breaks],
         ["fill", fill], ["color", pick(color, colour)], ["alpha", alpha]])

/// A frequency polygon: a histogram's counts as a line.
share fn freqpoly(p, mapping = (), data = (), bins = (), binwidth = (), color = (), colour = (), linewidth = (),
                  inherit = true, legend = true) =
    add(p, "freqpoly", (), (), mapping, data, inherit, legend,
        [["bins", bins], ["binwidth", binwidth], ["color", pick(color, colour)], ["linewidth", linewidth]])

/// A kernel density estimate (`bw`: "nrd0", "nrd", "scott" or a number;
/// `adjust`; `kernel`; `n`; `trim`).
share fn density(p, mapping = (), data = (), bw = (), adjust = (), kernel = (), n = (), trim = (), fill = (),
                 color = (), colour = (), alpha = (), position = (), inherit = true, legend = true) =
    add(p, "density", (), position, mapping, data, inherit, legend,
        [["bw", bw], ["adjust", adjust], ["kernel", kernel], ["n", n], ["trim", trim], ["fill", fill],
         ["color", pick(color, colour)], ["alpha", alpha]])

/// Boxplots: median, hinges, 1.5 IQR whiskers, outliers.
share fn boxplot(p, mapping = (), data = (), width = (), coef = (), outliers = (), fill = (), color = (),
                 colour = (), alpha = (), position = (), inherit = true, legend = true) =
    add(p, "boxplot", (), position, mapping, data, inherit, legend,
        [["width", width], ["coef", coef], ["outliers", outliers], ["fill", fill], ["color", pick(color, colour)], ["alpha", alpha]])

/// Violins: each group's density mirrored (`scale`: "area", "count",
/// "width"; `draw_quantiles`: e.g. [0.25, 0.5, 0.75]).
share fn violin(p, mapping = (), data = (), scale = (), trim = (), bw = (), adjust = (), draw_quantiles = (),
                width = (), fill = (), color = (), colour = (), alpha = (), inherit = true, legend = true) =
    add(p, "violin", (), (), mapping, data, inherit, legend,
        [["scale", scale], ["trim", trim], ["bw", bw], ["adjust", adjust], ["draw_quantiles", draw_quantiles],
         ["width", width], ["fill", fill], ["color", pick(color, colour)], ["alpha", alpha]])

fn range_params(color, colour, linewidth, width, size) =
    [["color", pick(color, colour)], ["linewidth", linewidth], ["width", width], ["size", size]]

/// Error bars from ymin to ymax.
share fn errorbar(p, mapping = (), data = (), width = (), color = (), colour = (), linewidth = (), inherit = true, legend = true) =
    add(p, "errorbar", (), (), mapping, data, inherit, legend, range_params(color, colour, linewidth, width, ()))

/// A point at y on a line from ymin to ymax.
share fn pointrange(p, mapping = (), data = (), color = (), colour = (), linewidth = (), size = (), inherit = true, legend = true) =
    add(p, "pointrange", (), (), mapping, data, inherit, legend, range_params(color, colour, linewidth, (), size))

/// A line from ymin to ymax.
share fn linerange(p, mapping = (), data = (), color = (), colour = (), linewidth = (), inherit = true, legend = true) =
    add(p, "linerange", (), (), mapping, data, inherit, legend, range_params(color, colour, linewidth, (), ()))

/// A box from ymin to ymax with a line at y.
share fn crossbar(p, mapping = (), data = (), width = (), color = (), colour = (), linewidth = (), inherit = true, legend = true) =
    add(p, "crossbar", (), (), mapping, data, inherit, legend, range_params(color, colour, linewidth, width, ()))

/// A summary of y at each x (`fun`: "mean_se", "mean_cl_normal",
/// "mean_sdl", "median_hilow", "mean", "median"; `level`), drawn as
/// `geom` ("pointrange", "errorbar", "crossbar", "point", "line", "col").
share fn summary(p, mapping = (), data = (), fun = (), level = (), mult = (), geom = "pointrange", color = (),
                 colour = (), size = (), width = (), inherit = true, legend = true) =
    add(p, if geom == "pointrange" => "summary" else => geom, "summary", (), mapping, data, inherit, legend,
        [["fun", fun], ["level", level], ["mult", mult], ["color", pick(color, colour)], ["size", size], ["width", width]])

/// A smoother with its standard-error band (`method`: "lm", "loess",
/// "gam", "auto"; `se`; `level`; `span`; `degree`; `k`; `n`; `fullrange`).
share fn smooth(p, mapping = (), data = (), method = (), se = (), level = (), span = (), degree = (), k = (),
                n = (), fullrange = (), color = (), colour = (), fill = (), linewidth = (), linetype = (),
                alpha = (), inherit = true, legend = true) =
    add(p, "smooth", (), (), mapping, data, inherit, legend,
        [["method", method], ["se", se], ["level", level], ["span", span], ["degree", degree], ["k", k], ["n", n],
         ["fullrange", fullrange], ["color", pick(color, colour)], ["fill", fill], ["linewidth", linewidth],
         ["linetype", linetype], ["alpha", alpha]])

fn text_params(label, color, colour, size, hjust, vjust, nudge_x, nudge_y, check_overlap, fontface, fill) =
    [["label", label], ["color", pick(color, colour)], ["size", size], ["hjust", hjust], ["vjust", vjust],
     ["nudge_x", nudge_x], ["nudge_y", nudge_y], ["check_overlap", check_overlap], ["fontface", fontface], ["fill", fill]]

fn nudged(nudge_x, nudge_y) = if nudge_x == () && nudge_y == () => () else => "nudge"

/// Text at each point (the label aesthetic; `size` in pixels; `hjust`,
/// `vjust`, `nudge_x`, `nudge_y`, `check_overlap`, `fontface`).
share fn text(p, mapping = (), data = (), label = (), color = (), colour = (), size = (), hjust = (), vjust = (),
              nudge_x = (), nudge_y = (), check_overlap = (), fontface = (), inherit = true, legend = true) =
    add(p, "text", (), nudged(nudge_x, nudge_y), mapping, data, inherit, legend,
        text_params(label, color, colour, size, hjust, vjust, nudge_x, nudge_y, check_overlap, fontface, ()))

/// Text on a box.
share fn label(p, mapping = (), data = (), label = (), color = (), colour = (), fill = (), size = (), hjust = (),
               vjust = (), nudge_x = (), nudge_y = (), inherit = true, legend = true) =
    add(p, "label", (), nudged(nudge_x, nudge_y), mapping, data, inherit, legend,
        text_params(label, color, colour, size, hjust, vjust, nudge_x, nudge_y, (), (), fill))

/// Tiles at each (x, y), filled (a heatmap).
share fn tile(p, mapping = (), data = (), width = (), height = (), fill = (), color = (), colour = (), linewidth = (),
              alpha = (), inherit = true, legend = true) =
    add(p, "tile", (), (), mapping, data, inherit, legend,
        [["width", width], ["height", height], ["fill", fill], ["color", pick(color, colour)], ["linewidth", linewidth], ["alpha", alpha]])

/// Tiles that abut exactly (a raster image of a grid).
share fn raster(p, mapping = (), data = (), fill = (), alpha = (), inherit = true, legend = true) =
    add(p, "raster", (), (), mapping, data, inherit, legend, [["fill", fill], ["alpha", alpha]])

/// Rectangles from xmin to xmax, ymin to ymax.
share fn rect(p, mapping = (), data = (), fill = (), color = (), colour = (), alpha = (), inherit = true, legend = true) =
    add(p, "rect", (), (), mapping, data, inherit, legend, [["fill", fill], ["color", pick(color, colour)], ["alpha", alpha]])

/// Horizontal reference lines at `yintercept` (a number or a list).
share fn hline(p, yintercept = (), mapping = (), data = (), color = (), colour = (), linetype = (), linewidth = (), alpha = ()) =
    add(p, "hline", (), (), mapping, data, yintercept == (), false,
        [["yintercept", yintercept]] + line_params(color, colour, linewidth, linetype, alpha))

/// Vertical reference lines at `xintercept`.
share fn vline(p, xintercept = (), mapping = (), data = (), color = (), colour = (), linetype = (), linewidth = (), alpha = ()) =
    add(p, "vline", (), (), mapping, data, xintercept == (), false,
        [["xintercept", xintercept]] + line_params(color, colour, linewidth, linetype, alpha))

/// A line y = intercept + slope · x (in the scales' transformed space).
share fn abline(p, slope = 1.0, intercept = 0.0, color = (), colour = (), linetype = (), linewidth = (), alpha = ()) =
    add(p, "abline", (), (), (), (), false, false,
        [["slope", slope], ["intercept", intercept]] + line_params(color, colour, linewidth, linetype, alpha))

/// Ticks along the axes at each value (`sides`: "bl", "b", "l", "tr"…).
share fn rug(p, mapping = (), data = (), sides = (), length = (), color = (), colour = (), alpha = (), inherit = true, legend = true) =
    add(p, "rug", (), (), mapping, data, inherit, legend,
        [["sides", sides], ["length", length], ["color", pick(color, colour)], ["alpha", alpha]])

/// Segments from (x, y) to (xend, yend).
share fn segment(p, mapping = (), data = (), color = (), colour = (), linewidth = (), linetype = (), alpha = (),
                 inherit = true, legend = true) =
    add(p, "segment", (), (), mapping, data, inherit, legend, line_params(color, colour, linewidth, linetype, alpha))

/// Hexagonal 2-D bins, filled by count (`bins`, `binwidth`).
share fn hex(p, mapping = (), data = (), bins = (), binwidth = (), inherit = true, legend = true) =
    add(p, "hex", (), (), mapping, data, inherit, legend, [["bins", bins], ["binwidth", binwidth]])

/// Rectangular 2-D bins, filled by count (`bins`, `binwidth`).
share fn bin2d(p, mapping = (), data = (), bins = (), binwidth = (), inherit = true, legend = true) =
    add(p, "bin2d", (), (), mapping, data, inherit, legend, [["bins", bins], ["binwidth", binwidth]])

/// The empirical cdf of x, as a staircase.
share fn ecdf(p, mapping = (), data = (), color = (), colour = (), linewidth = (), inherit = true, legend = true) =
    add(p, "ecdf", (), (), mapping, data, inherit, legend, line_params(color, colour, linewidth, (), ()))

/// A normal Q-Q plot of the sample aesthetic (`mean`, `sd` of the reference).
share fn qq(p, mapping = (), data = (), mean = (), sd = (), color = (), colour = (), size = (), alpha = (),
            inherit = true, legend = true) =
    add(p, "qq", (), (), mapping, data, inherit, legend,
        [["mean", mean], ["sd", sd], ["color", pick(color, colour)], ["size", size], ["alpha", alpha]])

/// The line through a Q-Q plot's quartiles.
share fn qq_line(p, mapping = (), data = (), mean = (), sd = (), color = (), colour = (), linetype = (),
                 inherit = true, legend = true) =
    add(p, "qq_line", (), (), mapping, data, inherit, legend,
        [["mean", mean], ["sd", sd], ["color", pick(color, colour)], ["linetype", linetype]])

/// One mark of any geom from constants (a label, a band, an arrow's
/// line): `gg.annotate(p, "text", x: 3, y: 9, label: "peak")`.
share fn annotate(p, geom, x = (), y = (), xend = (), yend = (), xmin = (), xmax = (), ymin = (), ymax = (),
                  label = (), color = (), colour = (), fill = (), alpha = (), size = (), linetype = (), hjust = (),
                  vjust = ()) = {
    let cols = filter([["x", x], ["y", y], ["xend", xend], ["yend", yend], ["xmin", xmin], ["xmax", xmax],
                       ["ymin", ymin], ["ymax", ymax], ["label", label]], (kv) => kv[1] != ())
    let n = fold(cols, 1, (a, kv) => if typeof(kv[1]) == "List" => max(a, len(kv[1])) else => a)
    let column = (v) => {
        let vs = if typeof(v) == "List" => v else => map(range(0, n), (i) => v)
        let fl = filter(vs, (e) => typeof(e) == "Int" || typeof(e) == "Float")
        if len(fl) == len(vs) => ods.series(map(vs, (e) => to_float(e))) else => ods.series(map(vs, (e) => to_string(e)))
    }
    let frame = ods.frame(map(cols, (kv) => [kv[0], column(kv[1])]))
    let mapping = fold(cols, #{}, (m, kv) => map_set(m, kv[0], kv[0]))
    add(p, geom, (), (), mapping, frame, false, false,
        [["color", pick(color, colour)], ["fill", fill], ["alpha", alpha], ["size", size], ["linetype", linetype],
         ["hjust", hjust], ["vjust", vjust]])
}

// ── positions ──────────────────────────────────────────────────────────

/// Stacked (`reverse`: the first group at the bottom).
share fn position_stack(reverse = ()) = put(#{ "name": "stack" }, "reverse", reverse)
/// Stacked and scaled to 1 at each x.
share fn position_fill(reverse = ()) = put(#{ "name": "fill" }, "reverse", reverse)
/// Side by side (`width`: the space the groups share).
share fn position_dodge(width = ()) = put(#{ "name": "dodge" }, "width", width)
/// Side by side with `padding` between.
share fn position_dodge2(width = (), padding = ()) = put(put(#{ "name": "dodge2" }, "width", width), "padding", padding)
/// Moved at random (`width`, `height`; `seed` keeps it reproducible).
share fn position_jitter(width = (), height = (), seed = ()) =
    put(put(put(#{ "name": "jitter" }, "width", width), "height", height), "seed", seed)
/// Moved by a constant (`x`, `y`).
share fn position_nudge(x = (), y = ()) = put(put(#{ "name": "nudge" }, "nudge_x", x), "nudge_y", y)

// ── scales ─────────────────────────────────────────────────────────────

fn axis_pairs(name, breaks, labels, limits, expand, prefix, suffix) =
    [["name", name], ["breaks", breaks], ["labels", labels], ["limits", limits], ["expand", expand],
     ["prefix", prefix], ["suffix", suffix]]

/// The x axis, continuous (`labels`: "percent", "comma", "dollar",
/// "short", "scientific", or a list; `breaks`; `limits`: [lo, hi];
/// `trans`: "log10", "sqrt", "reverse"; `prefix`, `suffix`).
share fn scale_x_continuous(p, name = (), breaks = (), labels = (), limits = (), expand = (), trans = (), prefix = (), suffix = ()) =
    add_scale(p, "x", "continuous", axis_pairs(name, breaks, labels, limits, expand, prefix, suffix) + [["trans", trans]])
/// The y axis, continuous.
share fn scale_y_continuous(p, name = (), breaks = (), labels = (), limits = (), expand = (), trans = (), prefix = (), suffix = ()) =
    add_scale(p, "y", "continuous", axis_pairs(name, breaks, labels, limits, expand, prefix, suffix) + [["trans", trans]])
/// The x axis on a log10 scale (breaks at powers of ten).
share fn scale_x_log10(p, name = (), breaks = (), labels = (), limits = (), expand = ()) =
    add_scale(p, "x", "log10", axis_pairs(name, breaks, labels, limits, expand, (), ()))
/// The y axis on a log10 scale.
share fn scale_y_log10(p, name = (), breaks = (), labels = (), limits = (), expand = ()) =
    add_scale(p, "y", "log10", axis_pairs(name, breaks, labels, limits, expand, (), ()))
/// The x axis on a square-root scale.
share fn scale_x_sqrt(p, name = (), breaks = (), labels = (), limits = ()) =
    add_scale(p, "x", "sqrt", axis_pairs(name, breaks, labels, limits, (), (), ()))
/// The y axis on a square-root scale.
share fn scale_y_sqrt(p, name = (), breaks = (), labels = (), limits = ()) =
    add_scale(p, "y", "sqrt", axis_pairs(name, breaks, labels, limits, (), (), ()))
/// The x axis reversed.
share fn scale_x_reverse(p, name = (), breaks = (), labels = (), limits = ()) =
    add_scale(p, "x", "reverse", axis_pairs(name, breaks, labels, limits, (), (), ()))
/// The y axis reversed.
share fn scale_y_reverse(p, name = (), breaks = (), labels = (), limits = ()) =
    add_scale(p, "y", "reverse", axis_pairs(name, breaks, labels, limits, (), (), ()))
/// The x axis as categories (`limits`: their order; `labels`: a list or
/// a map of level to label).
share fn scale_x_discrete(p, name = (), limits = (), labels = (), breaks = ()) =
    add_scale(p, "x", "discrete", [["name", name], ["limits", limits], ["labels", labels], ["breaks", breaks]])
/// The y axis as categories.
share fn scale_y_discrete(p, name = (), limits = (), labels = (), breaks = ()) =
    add_scale(p, "y", "discrete", [["name", name], ["limits", limits], ["labels", labels], ["breaks", breaks]])
/// The x axis as dates (ISO strings parsed; `date_breaks`: "1 month";
/// `date_labels`: "%b %Y").
share fn scale_x_date(p, name = (), date_breaks = (), date_labels = (), limits = (), breaks = ()) =
    add_scale(p, "x", "date", [["name", name], ["date_breaks", date_breaks], ["date_labels", date_labels], ["limits", limits], ["breaks", breaks]])
/// The y axis as dates.
share fn scale_y_date(p, name = (), date_breaks = (), date_labels = (), limits = ()) =
    add_scale(p, "y", "date", [["name", name], ["date_breaks", date_breaks], ["date_labels", date_labels], ["limits", limits]])
/// The x axis as date-times.
share fn scale_x_datetime(p, name = (), date_breaks = (), date_labels = (), limits = ()) =
    add_scale(p, "x", "datetime", [["name", name], ["date_breaks", date_breaks], ["date_labels", date_labels], ["limits", limits]])
/// The y axis as date-times.
share fn scale_y_datetime(p, name = (), date_breaks = (), date_labels = (), limits = ()) =
    add_scale(p, "y", "datetime", [["name", name], ["date_breaks", date_breaks], ["date_labels", date_labels], ["limits", limits]])

fn color_pairs(name, values, labels, limits, breaks, guide) =
    [["name", name], ["values", values], ["labels", labels], ["limits", limits], ["breaks", breaks], ["guide", guide]]

/// Colours by hand: `values` a list (in the levels' order) or a map of
/// level to colour.
share fn scale_color_manual(p, values, name = (), labels = (), limits = (), breaks = (), guide = ()) =
    add_scale(p, "color", "manual", color_pairs(name, values, labels, limits, breaks, guide))
/// Fills by hand.
share fn scale_fill_manual(p, values, name = (), labels = (), limits = (), breaks = (), guide = ()) =
    add_scale(p, "fill", "manual", color_pairs(name, values, labels, limits, breaks, guide))
/// Categorical colours from a named palette ("quiet", the default, or a
/// viridis option).
share fn scale_color_discrete(p, palette = (), name = (), labels = (), limits = (), guide = ()) =
    add_scale(p, "color", "palette", color_pairs(name, (), labels, limits, (), guide) + [["palette", palette]])
/// Categorical fills from a named palette.
share fn scale_fill_discrete(p, palette = (), name = (), labels = (), limits = (), guide = ()) =
    add_scale(p, "fill", "palette", color_pairs(name, (), labels, limits, (), guide) + [["palette", palette]])

fn viridis_pairs(option, begin, end, direction, name, labels, limits, breaks, guide) =
    color_pairs(name, (), labels, limits, breaks, guide) + [["option", option], ["begin", begin], ["end", end], ["direction", direction]]

/// Categorical colours from the viridis family (`option`: "viridis",
/// "magma", "inferno", "plasma", "cividis", "mako", "rocket").
share fn scale_color_viridis_d(p, option = (), begin = (), end = (), direction = (), name = (), labels = (), limits = (), guide = ()) =
    add_scale(p, "color", "viridis_d", viridis_pairs(option, begin, end, direction, name, labels, limits, (), guide))
/// Categorical fills from the viridis family.
share fn scale_fill_viridis_d(p, option = (), begin = (), end = (), direction = (), name = (), labels = (), limits = (), guide = ()) =
    add_scale(p, "fill", "viridis_d", viridis_pairs(option, begin, end, direction, name, labels, limits, (), guide))
/// Continuous colours along a viridis ramp.
share fn scale_color_viridis_c(p, option = (), begin = (), end = (), direction = (), name = (), labels = (), limits = (), breaks = (), trans = (), guide = ()) =
    add_scale(p, "color", "viridis_c", viridis_pairs(option, begin, end, direction, name, labels, limits, breaks, guide) + [["trans", trans]])
/// Continuous fills along a viridis ramp.
share fn scale_fill_viridis_c(p, option = (), begin = (), end = (), direction = (), name = (), labels = (), limits = (), breaks = (), trans = (), guide = ()) =
    add_scale(p, "fill", "viridis_c", viridis_pairs(option, begin, end, direction, name, labels, limits, breaks, guide) + [["trans", trans]])
/// Continuous colours from `low` to `high`.
share fn scale_color_gradient(p, low = (), high = (), name = (), labels = (), limits = (), breaks = (), trans = (), guide = ()) =
    add_scale(p, "color", "gradient", color_pairs(name, (), labels, limits, breaks, guide) + [["low", low], ["high", high], ["trans", trans]])
/// Continuous fills from `low` to `high`.
share fn scale_fill_gradient(p, low = (), high = (), name = (), labels = (), limits = (), breaks = (), trans = (), guide = ()) =
    add_scale(p, "fill", "gradient", color_pairs(name, (), labels, limits, breaks, guide) + [["low", low], ["high", high], ["trans", trans]])
/// Diverging colours centred on `midpoint` (0): `low`, `mid`, `high`.
share fn scale_color_gradient2(p, low = (), mid = (), high = (), midpoint = (), name = (), labels = (), limits = (), breaks = (), guide = ()) =
    add_scale(p, "color", "gradient2", color_pairs(name, (), labels, limits, breaks, guide) + [["low", low], ["mid", mid], ["high", high], ["midpoint", midpoint]])
/// Diverging fills centred on `midpoint`.
share fn scale_fill_gradient2(p, low = (), mid = (), high = (), midpoint = (), name = (), labels = (), limits = (), breaks = (), guide = ()) =
    add_scale(p, "fill", "gradient2", color_pairs(name, (), labels, limits, breaks, guide) + [["low", low], ["mid", mid], ["high", high], ["midpoint", midpoint]])
/// Continuous colours through a list of `colors`.
share fn scale_color_gradientn(p, colors, name = (), labels = (), limits = (), breaks = (), guide = ()) =
    add_scale(p, "color", "gradientn", color_pairs(name, (), labels, limits, breaks, guide) + [["colors", colors]])
/// Continuous fills through a list of `colors`.
share fn scale_fill_gradientn(p, colors, name = (), labels = (), limits = (), breaks = (), guide = ()) =
    add_scale(p, "fill", "gradientn", color_pairs(name, (), labels, limits, breaks, guide) + [["colors", colors]])
/// Greys for categories (`start`, `end` in [0, 1]).
share fn scale_color_grey(p, start = (), end = (), name = (), guide = ()) =
    add_scale(p, "color", "grey", color_pairs(name, (), (), (), (), guide) + [["start", start], ["end", end]])
/// Grey fills for categories.
share fn scale_fill_grey(p, start = (), end = (), name = (), guide = ()) =
    add_scale(p, "fill", "grey", color_pairs(name, (), (), (), (), guide) + [["start", start], ["end", end]])
/// The column's values are the colours themselves (names or `#hex`).
share fn scale_color_identity(p) = add_scale(p, "color", "identity", [])
/// The column's values are the fills themselves.
share fn scale_fill_identity(p) = add_scale(p, "fill", "identity", [])
/// Point sizes by area (`range`: [smallest, largest] in size units).
share fn scale_size(p, range = (), name = (), breaks = (), labels = (), limits = (), trans = (), guide = ()) =
    add_scale(p, "size", "area", [["range", range], ["name", name], ["breaks", breaks], ["labels", labels], ["limits", limits], ["trans", trans], ["guide", guide]])
/// Opacities (`range`: [least, most]).
share fn scale_alpha(p, range = (), name = (), breaks = (), labels = (), limits = (), guide = ()) =
    add_scale(p, "alpha", "continuous", [["range", range], ["name", name], ["breaks", breaks], ["labels", labels], ["limits", limits], ["guide", guide]])
/// Shapes for categories (`values`: e.g. ["circle", "triangle", "square"]).
share fn scale_shape(p, values = (), name = (), labels = (), limits = (), guide = ()) =
    add_scale(p, "shape", "shape", [["values", values], ["name", name], ["labels", labels], ["limits", limits], ["guide", guide]])
/// Dash patterns for categories (`values`: "solid", "dashed", "dotted",
/// "dotdash", "longdash", "twodash").
share fn scale_linetype(p, values = (), name = (), labels = (), limits = (), guide = ()) =
    add_scale(p, "linetype", "linetype", [["values", values], ["name", name], ["labels", labels], ["limits", limits], ["guide", guide]])

// ── facets ─────────────────────────────────────────────────────────────

fn names_of(v) = if v == () => [] else if typeof(v) == "List" => v else => [v]

/// Small multiples: one panel per value of `facets` (a column or a list),
/// wrapped into `ncol` columns (`scales`: "fixed", "free", "free_x", "free_y").
share fn facet_wrap(p, facets, ncol = (), nrow = (), scales = "fixed") =
    map_set(check_plot(p, "facet_wrap"), "facet", #{ "kind": "wrap", "rows": names_of(facets), "cols": [], "ncol": ncol, "nrow": nrow, "scales": scales })

/// A grid of panels: `rows` by `cols` (either may be ()).
share fn facet_grid(p, rows = (), cols = (), scales = "fixed") =
    map_set(check_plot(p, "facet_grid"), "facet", #{ "kind": "grid", "rows": names_of(rows), "cols": names_of(cols), "scales": scales })

// ── coordinates ────────────────────────────────────────────────────────

/// The axes swapped: x drawn vertically (horizontal bars, boxes).
share fn coord_flip(p, xlim = (), ylim = ()) = map_set(check_plot(p, "coord_flip"), "coord", #{ "kind": "flip", "xlim": xlim, "ylim": ylim })
/// One unit of y the length of `ratio` units of x.
share fn coord_fixed(p, ratio = 1.0, xlim = (), ylim = ()) = map_set(check_plot(p, "coord_fixed"), "coord", #{ "kind": "fixed", "ratio": ratio, "xlim": xlim, "ylim": ylim })
/// Zoomed to `xlim`, `ylim` without dropping data (unlike a scale's limits).
share fn coord_cartesian(p, xlim = (), ylim = ()) = map_set(check_plot(p, "coord_cartesian"), "coord", #{ "kind": "cartesian", "xlim": xlim, "ylim": ylim })

// ── labels and theme ───────────────────────────────────────────────────

/// Titles: the plot's, a subtitle, a caption, the axes', the legends'.
share fn labs(p, title = (), subtitle = (), caption = (), x = (), y = (), color = (), colour = (), fill = (),
              size = (), shape = (), alpha = (), linetype = ()) = {
    let new = pairs_map([["title", title], ["subtitle", subtitle], ["caption", caption], ["x", x], ["y", y],
                         ["color", pick(color, colour)], ["fill", fill], ["size", size], ["shape", shape],
                         ["alpha", alpha], ["linetype", linetype]])
    map_set(check_plot(p, "labs"), "labels", map_merge(p.labels, new))
}

/// The plot's title.
share fn ggtitle(p, title, subtitle = ()) = labs(p, title: title, subtitle: subtitle)
/// The x axis's title.
share fn xlab(p, title) = labs(p, x: title)
/// The y axis's title.
share fn ylab(p, title) = labs(p, y: title)

/// The look: `name` ("minimal" — the default, Studio's quiet instrument —
/// "classic", "bw", "gray", "linedraw", "void", or "light", "dark",
/// "contrast" for minimal in that appearance), `appearance`, `base_size`,
/// `legend_position` ("right", "bottom", "top", "left", "none"),
/// `width` and `height` in pixels, `grid` ("both", "x", "y", "none"), `font`.
share fn theme(p, name = (), appearance = (), base_size = (), legend_position = (), width = (), height = (),
               grid = (), font = ()) = {
    let new = pairs_map([["name", name], ["appearance", appearance], ["base_size", base_size],
                         ["legend_position", legend_position], ["width", width], ["height", height],
                         ["grid", grid], ["font", font]])
    map_set(check_plot(p, "theme"), "theme", map_merge(p.theme, new))
}

// ── output ─────────────────────────────────────────────────────────────

/// The plot as SVG text.
share fn svg(p) = native.gg(check_plot(p, "svg"))

/// The plot's scene as JSON text: what a host draws natively.
share fn scene(p) = native.gg_scene(check_plot(p, "scene"))

/// The plot written to `path` as SVG.
share fn save(p, path) = fs.write_file(path, svg(p))
