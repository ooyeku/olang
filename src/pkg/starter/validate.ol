//! validate — declarative checks for map-shaped input.
//!
//! Data arriving from a form, a JSON body, a CSV row, or a config file
//! is a Map of unknowns. `validate.check` holds it against a rule list
//! and returns `Ok(value)` or `Err(problems)` — every problem, not just
//! the first, each naming its field, so the caller can show a complete
//! error report in one round trip.
//!
//! A rule is a list: `[field, kind]` or `[field, kind, opts]`, where
//! `kind` is one of "str", "int", "float", "num", "bool", "list",
//! "map", "date", "any", and `opts` is a map of any of:
//!
//!   required: false     absent field is fine (present still checks)
//!   min / max: Int      numeric bound, or length bound for str/list;
//!                       for "date", an ISO day the value must not
//!                       precede / exceed
//!   one_of: List        value must be one of these
//!   pattern: String     regex the whole string must match
//!   where: fn           any predicate: (value) => Result — an Err
//!                       message becomes the field's problem
//!
//! A "date" is an ISO day string (`2026-08-31`) that `dates.parse`
//! accepts. Parsed JSON (`JsonObject`) is map-shaped and checks like a
//! Map, so a request body needs no copy first.

/// Does `value` have the olang type `kind` names?
fn kind_ok(kind, value) = {
    let t = typeof(value)
    if kind == "any" => true
    else if kind == "str" => t == "String"
    else if kind == "int" => t == "Int"
    else if kind == "float" => t == "Float"
    else if kind == "num" => t == "Int" || t == "Float"
    else if kind == "bool" => t == "Bool"
    else if kind == "list" => t == "List"
    else if kind == "map" => t == "Map" || t == "JsonObject"
    else if kind == "date" => t == "String" && is_ok(dates.parse(value))
    else => false
}

/// The size `min`/`max` govern: the value itself for numbers, the
/// length for strings and lists; Unit when bounds don't apply.
fn measure(value) = {
    let t = typeof(value)
    if t == "Int" || t == "Float" => value
    else if t == "String" || t == "List" => len(value)
    else => ()
}

/// One field's problems under one rule, as a list of messages.
fn field_problems(m, rule) = {
    let field = rule[0]
    let kind = rule[1]
    let opts = if len(rule) > 2 => rule[2] else => #{}
    let required = if map_has_key(opts, "required") => map_get(opts, "required") else => true

    if !map_has_key(m, field) => {
        if required => [`${field}: required`] else => []
    }
    else => {
        let value = map_get(m, field)
        if !kind_ok(kind, value) => [`${field}: expected ${kind}, got ${typeof(value)}`]
        else => {
            let mut problems = []
            let size = if kind == "date" => () else => measure(value)
            if map_has_key(opts, "min") && size != () && size < map_get(opts, "min") => {
                problems = problems + [`${field}: below minimum ${map_get(opts, "min")}`]
            }
            if map_has_key(opts, "max") && size != () && size > map_get(opts, "max") => {
                problems = problems + [`${field}: above maximum ${map_get(opts, "max")}`]
            }
            if map_has_key(opts, "one_of") && !contains(map_get(opts, "one_of"), value) => {
                problems = problems + [`${field}: must be one of ${to_string(map_get(opts, "one_of"))}`]
            }
            if map_has_key(opts, "pattern") && typeof(value) == "String" => {
                let hit = match re.is_match(map_get(opts, "pattern"), value) {
                    Ok(b) => b,
                    Err(e) => false
                }
                if !hit => {
                    problems = problems + [`${field}: does not match ${map_get(opts, "pattern")}`]
                }
            }
            // Dates compare as ISO day strings, which order lexically.
            if kind == "date" => {
                if map_has_key(opts, "min") && value < map_get(opts, "min") => {
                    problems = problems + [`${field}: before ${map_get(opts, "min")}`]
                }
                if map_has_key(opts, "max") && value > map_get(opts, "max") => {
                    problems = problems + [`${field}: after ${map_get(opts, "max")}`]
                }
            }
            if map_has_key(opts, "where") => {
                match map_get(opts, "where")(value) {
                    Ok(v) => (),
                    Err(message) => { problems = problems + [`${field}: ${message}`] }
                }
            }
            problems
        }
    }
}

/// Hold a map against a rule list. `Ok(m)` when every rule passes;
/// `Err(problems)` — a list of "field: problem" strings, one per
/// failure, in rule order — when any fails. A non-map value fails
/// immediately with a single problem naming its type.
share fn check(m, rules) = {
    if typeof(m) != "Map" && typeof(m) != "JsonObject" => Err([`expected a map, got ${typeof(m)}`])
    else => {
        let problems = fold(rules, [], (acc, rule) => acc + field_problems(m, rule))
        if len(problems) == 0 => Ok(m) else => Err(problems)
    }
}

/// True when `m` passes every rule — `check` for code that only needs
/// the verdict.
share fn ok(m, rules) = is_ok(check(m, rules))

test "dates, predicates, and parsed JSON" {
    let rules = [["due", "date", #{ "required": false, "min": "2026-01-01" }]]
    assert_eq(ok(#{ "due": "2026-08-31" }, rules), true)
    assert_eq(ok(#{}, rules), true)
    assert_eq(ok(#{ "due": "not a day" }, rules), false)
    assert_eq(ok(#{ "due": "2025-12-31" }, rules), false)
    let even = [["n", "int", #{ "where": (v) => if v % 2 == 0 => Ok(v) else => Err("must be even") }]]
    assert_eq(ok(#{ "n": 4 }, even), true)
    match check(#{ "n": 3 }, even) {
        Err(problems) => assert_eq(problems, ["n: must be even"]),
        Ok(v) => assert_eq("passed", "should have failed")
    }
    let body = unwrap(json.parse("{\"name\": \"ada\", \"age\": 36}"))
    assert_eq(ok(body, [["name", "str"], ["age", "int"]]), true)
}

test "kinds and requirement" {
    let rules = [["name", "str"], ["age", "int"]]
    assert_eq(check(#{ "name": "Ada", "age": 36 }, rules), Ok(#{ "name": "Ada", "age": 36 }))
    assert_eq(check(#{ "name": "Ada" }, rules), Err(["age: required"]))
    assert_eq(check(#{ "name": 7, "age": "x" }, rules),
        Err(["name: expected str, got Int", "age: expected int, got String"]))
    assert_eq(check("nope", rules), Err(["expected a map, got String"]))
}

test "optional fields check only when present" {
    let rules = [["note", "str", #{ "required": false }]]
    assert_eq(ok(#{}, rules), true)
    assert_eq(ok(#{ "note": "hi" }, rules), true)
    assert_eq(ok(#{ "note": 3 }, rules), false)
}

test "bounds are values for numbers, lengths for strings and lists" {
    assert_eq(ok(#{ "age": 36 }, [["age", "int", #{ "min": 0, "max": 130 }]]), true)
    assert_eq(ok(#{ "age": 200 }, [["age", "int", #{ "max": 130 }]]), false)
    assert_eq(ok(#{ "pin": "1234" }, [["pin", "str", #{ "min": 4, "max": 4 }]]), true)
    assert_eq(ok(#{ "tags": [1, 2, 3] }, [["tags", "list", #{ "max": 2 }]]), false)
}

test "one_of and pattern" {
    assert_eq(ok(#{ "kind": "expense" }, [["kind", "str", #{ "one_of": ["expense", "income"] }]]), true)
    assert_eq(ok(#{ "kind": "loan" }, [["kind", "str", #{ "one_of": ["expense", "income"] }]]), false)
    assert_eq(ok(#{ "id": "tx_042" }, [["id", "str", #{ "pattern": "^tx_[0-9]+$" }]]), true)
    assert_eq(ok(#{ "id": "42" }, [["id", "str", #{ "pattern": "^tx_[0-9]+$" }]]), false)
}
