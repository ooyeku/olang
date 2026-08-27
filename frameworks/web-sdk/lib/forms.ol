//! forms — declare fields once; render, validate, and read them from
//! the same declaration.
//!
//! A field is data: `field("title", "Title", "text", rules)` where
//! `rules` is a `validate`-format options map (`min`, `max`, `one_of`,
//! `pattern`, `required`) — the starter library every shelf ships.
//! From one field list the app gets:
//!
//! - `render(fields, values, errors)` — the form's markup, with
//!   current values filled in and per-field errors shown
//! - `rules(fields)` — the rule list `validate.check` takes, so the
//!   same declaration validates in the browser (for immediacy) and on
//!   the server (for truth)
//! - `read(payload, fields)` — the submitted map narrowed to declared
//!   fields, numbers parsed for "number" fields

use lib.html { div, label, input, span, textarea, select, option, text }

/// One field: `name` is the payload key, `label_text` what people see,
/// `kind` one of "text" | "number" | "password" | "date" | "textarea"
/// | a `#{ "select": [options] }` map, and `opts` the validate-format
/// rule options for this field (use `#{}` for none).
share fn field(name, label_text, kind, opts) =
    #{ "name": name, "label": label_text, "kind": kind, "opts": opts }

/// The `validate.check` rule list for these fields: number fields
/// check as "num", everything else as "str".
share fn rules(fields) =
    map(fields, (f) => {
        let k = map_get(f, "kind")
        let kind = if typeof(k) == "String" && k == "number" => "num" else => "str"
        [map_get(f, "name"), kind, map_get(f, "opts")]
    })

/// The submitted payload narrowed to the declared fields; "number"
/// fields parse (unparseable input stays a string, so validation
/// reports it rather than a parse trap).
share fn read(payload, fields) = {
    let mut out = #{}
    for f in fields {
        let name = map_get(f, "name")
        if map_has_key(payload, name) => {
            let v = map_get(payload, name)
            let k = map_get(f, "kind")
            let parsed = if typeof(k) == "String" && k == "number" && typeof(v) == "String" =>
                match str.parse_int(v) {
                    Ok(n) => n,
                    Err(e) => match str.parse_float(v) { Ok(x) => x, Err(e2) => v }
                }
            else => v
            out = map_set(out, name, parsed)
        }
    }
    out
}

fn control(f, value, has_error) = {
    let name = map_get(f, "name")
    let kind = map_get(f, "kind")
    let classes = if has_error => "field-input field-error" else => "field-input"
    if typeof(kind) == "Map" && map_has_key(kind, "select") =>
        select(#{ "name": name, "id": name, "class": classes },
            map(map_get(kind, "select"), (o) =>
                option(#{ "value": o, "selected": o == value }, [o])))
    else if kind == "textarea" =>
        textarea(#{ "name": name, "id": name, "class": classes }, [to_string_value(value)])
    else =>
        input(#{ "name": name, "id": name, "type": kind, "class": classes,
                 "value": to_string_value(value) })
}

fn to_string_value(v) =
    if v == () => "" else if typeof(v) == "String" => v else => to_string(v)

/// The form's markup: every declared field with its label, its current
/// value from `values`, and its error (if `errors` — a list of
/// "field: problem" strings from `validate.check` — names it).
share fn form_fields(fields, values, errors) =
    div(#{ "class": "form-fields" }, map(fields, (f) => {
        let name = map_get(f, "name")
        let mine = errors
            |> filter((e) => str.starts_with(e, name + ":"))
            |> map((e) => str.trim(str.substring(e, str.length(name) + 1, str.length(e))))
        let value = if map_has_key(values, name) => map_get(values, name) else => ()
        div(#{ "class": "field" }, [
            label(#{ "for": name }, [map_get(f, "label")]),
            control(f, value, len(mine) > 0),
            if len(mine) > 0 =>
                span(#{ "class": "field-message" }, [mine[0]])
            else => text("")
        ])
    }))

test "select fields rule and read as strings, never trapping" {
    // A select field's kind is a MAP — the comparison against
    // "number" must not trap on Map == String (it did, live).
    let fs = [field("priority", "Priority", #{ "select": ["a", "b"] },
        #{ "one_of": ["a", "b"] })]
    assert_eq(rules(fs), [["priority", "str", #{ "one_of": ["a", "b"] }]])
    assert_eq(read(#{ "priority": "a" }, fs), #{ "priority": "a" })
}

test "rules mirror the field declarations" {
    let fs = [
        field("title", "Title", "text", #{ "min": 1, "max": 80 }),
        field("points", "Points", "number", #{ "min": 0 })
    ]
    assert_eq(rules(fs), [
        ["title", "str", #{ "min": 1, "max": 80 }],
        ["points", "num", #{ "min": 0 }]
    ])
}

test "read narrows and parses numbers" {
    let fs = [field("t", "T", "text", #{}), field("n", "N", "number", #{})]
    let got = read(#{ "t": "x", "n": "42", "extra": "dropped" }, fs)
    assert_eq(got, #{ "t": "x", "n": 42 })
    assert_eq(read(#{ "n": "not-a-number" }, fs), #{ "n": "not-a-number" })
}

test "render fills values and shows the field's error" {
    let fs = [field("title", "Title", "text", #{})]
    let out = html.render(form_fields(fs, #{ "title": "ship it" }, ["title: required"]))
    assert_eq(str.contains(out, "value=\"ship it\""), true)
    assert_eq(str.contains(out, "field-message"), true)
    assert_eq(str.contains(out, "required"), true)
}
