//! ui — the opinionated components, bound to web.css.
//!
//! Each component is a plain `web.html` node wearing the design
//! system's classes, so an app that writes zero CSS still looks
//! deliberate — and an app that wants its own look overrides the
//! tokens in `:root`, not the components.

use lib.html {
    el, div, span, button, table, thead, tbody, tr, th, td, a, header, text
}

/// A vertical stack with the standard gap.
share fn stack(children) = div(#{ "class": "stack" }, children)

/// A horizontal row, wrapping, standard gap.
share fn row(children) = div(#{ "class": "row" }, children)

/// Left and right ends of a line — titles with actions.
share fn spread(left, right) = div(#{ "class": "spread" }, [left, right])

/// A responsive card grid.
share fn grid(children) = div(#{ "class": "grid" }, children)

/// A surface with a border and padding.
share fn card(children) = div(#{ "class": "card" }, children)

/// Dimmed helper text.
share fn muted(s) = span(#{ "class": "muted" }, [s])

/// A small status pill; `accent` gives it the theme color.
share fn badge(s, accent) =
    span(#{ "class": if accent => "badge badge-accent" else => "badge" }, [s])

/// The standard button. `data-action` is how web.view's delegation
/// finds it: `btn("Add", "todo.add")` fires the "todo.add" handler.
share fn btn(label_text, action) =
    button(#{ "class": "btn", "data-action": action }, [label_text])

/// The primary action of a view.
share fn btn_primary(label_text, action) =
    button(#{ "class": "btn btn-primary", "data-action": action }, [label_text])

/// A destructive action.
share fn btn_danger(label_text, action) =
    button(#{ "class": "btn btn-danger", "data-action": action }, [label_text])

/// A data table from headers and rows of cells (cells are nodes or
/// strings): `data_table(["Title", "Points"], rows)`.
share fn data_table(headers, rows) =
    table(#{ "class": "table" }, [
        thead(#{}, [tr(#{}, map(headers, (h) => th(#{}, [h])))]),
        tbody(#{}, map(rows, (cells) => tr(#{}, map(cells, (c) => td(#{}, [c])))))
    ])

/// The page header: a brand and a row of right-side content.
share fn topbar(brand, right) =
    header(#{ "class": "topbar" }, [
        span(#{ "class": "brand" }, [brand]),
        row(right)
    ])

test "components wear the design system's classes" {
    assert_eq(html.render(card(["x"])), "<div class=\"card\">x</div>")
    assert_eq(html.render(btn("Add", "todo.add")),
        "<button class=\"btn\" data-action=\"todo.add\">Add</button>")
    assert_eq(str.contains(html.render(badge("open", true)), "badge-accent"), true)
}

test "data_table builds the full table" {
    let out = html.render(data_table(["A"], [["1"], ["2"]]))
    assert_eq(str.contains(out, "<th>A</th>"), true)
    assert_eq(str.contains(out, "<td>2</td>"), true)
}
