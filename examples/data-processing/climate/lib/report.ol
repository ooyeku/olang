//! report — markdown and chart assembly for the analysis outputs.
//!
//! The report builds as a list of markdown blocks and writes once at
//! the end; charts render through `plot` to standalone SVG files next
//! to it, referenced by relative path so `out/` is a self-contained
//! artifact.

/// A markdown table from a frame's first `n` rows. Column order is the
//! frame's own; floats render rounded to 2 places.
share fn table(frame, n) = {
    let names = ods.columns(frame)
    let shown = ods.head(frame, n)
    let mut lines = ["| " + str.join(names, " | ") + " |"]
    lines = concat(lines, ["|" + str.join(map(names, (n) => "---"), "|") + "|"])
    for record in ods.to_records(shown) {
        let cells = map(names, (name) => render_cell(map_get(record, name)))
        lines = concat(lines, ["| " + str.join(cells, " | ") + " |"])
    }
    str.join(lines, "\n")
}

fn render_cell(v) =
    if v == () => ""
    else if typeof(v) == "Float" => show(math.round(v * 100.0) / 100.0)
    else => show(v)

/// Write `svg` under out/ and return the markdown image reference.
share fn chart(name, svg) = {
    unwrap(fs.write_file(fs.join("out", name), svg))
    `![${name}](${name})`
}

/// Write a frame as a derived CSV under out/.
share fn derived_csv(name, frame) = {
    unwrap(ods.write_csv(frame, fs.join("out", name)))
    ods.n_rows(frame)
}

/// Assemble and write the final report.
share fn write(blocks) = {
    let text = str.join(blocks, "\n\n") + "\n"
    unwrap(fs.write_file(fs.join("out", "report.md"), text))
    str.length(text)
}

test "table renders headers, rows, and rounded floats" {
    let f = ods.frame_from_records([
        #{ "name": "a", "v": 1.234567 },
        #{ "name": "b", "v": 2.0 }
    ])
    let t = table(f, 10)
    assert_eq(str.contains(t, "| name | v |"), true)
    assert_eq(str.contains(t, "| a | 1.23 |"), true)
    assert_eq(str.contains(t, "| b | 2 |") || str.contains(t, "| b | 2.0 |"), true)
}

test "render_cell shows nulls as empty" {
    assert_eq(render_cell(()), "")
    assert_eq(render_cell("x"), "x")
}
