// Input validation. Every validator returns Ok(clean_fields) or
// Err([{ field, message }, ...]) — the handler turns the Err list into a
// 422 with details. Nothing unvalidated reaches lib/store.ol.

fn problem(field, message) = { field: field, message: message }

share fn validate_id(raw) = match str.parse_int(raw) {
    Err(e) => Err("not a numeric id"),
    Ok(n) => if n > 0 => Ok(n) else => Err("ids are positive")
}

share fn valid_date(s) = typeof(s) == "String" && is_ok(dates.parse_date(s))

share fn valid_month(s) =
    typeof(s) == "String" && str.length(s) == 7 && is_ok(dates.parse_date(s + "-01"))

// ── transactions ─────────────────────────────────────────────────────
// `partial` = true for PATCH: only provided keys are validated, and at
// least one known key must be present.

share fn validate_transaction(doc, partial) = {
    let mut fields = #{}
    let mut problems = []

    if map_has_key(doc, "date") || !partial => {
        let v = map_get(doc, "date")
        if valid_date(v) => { fields = map_set(fields, "date", v) }
        else => { problems = concat(problems, [problem("date", "must be YYYY-MM-DD")]) }
    }
    if map_has_key(doc, "amount_cents") || !partial => {
        let v = map_get(doc, "amount_cents")
        if typeof(v) == "Int" && v != 0 => { fields = map_set(fields, "amount_cents", v) }
        else => { problems = concat(problems, [problem("amount_cents", "must be a non-zero integer (cents)")]) }
    }
    if map_has_key(doc, "category_id") || !partial => {
        let v = map_get(doc, "category_id")
        if typeof(v) == "Int" && v > 0 => { fields = map_set(fields, "category_id", v) }
        else => { problems = concat(problems, [problem("category_id", "must be a positive integer")]) }
    }
    if map_has_key(doc, "note") => {
        let v = map_get(doc, "note")
        if typeof(v) == "String" && str.length(v) <= 500 => { fields = map_set(fields, "note", v) }
        else => { problems = concat(problems, [problem("note", "must be a string of at most 500 characters")]) }
    }
    else => if !partial => { fields = map_set(fields, "note", "") }

    if len(problems) > 0 => Err(problems)
    else => if partial && len(map_keys(fields)) == 0 =>
        Err([problem("body", "provide at least one of date, amount_cents, category_id, note")])
    else => Ok(fields)
}

// ── categories ───────────────────────────────────────────────────────

share fn validate_category(doc) = {
    let mut problems = []
    let name = if map_has_key(doc, "name") => map_get(doc, "name") else => ""
    let kind = if map_has_key(doc, "kind") => map_get(doc, "kind") else => ""
    let clean_name = if typeof(name) == "String" => str.trim(name) else => ""
    if clean_name == "" || str.length(clean_name) > 60 =>
        { problems = concat(problems, [problem("name", "must be 1-60 characters")]) }
    if !contains(["expense", "income"], kind) =>
        { problems = concat(problems, [problem("kind", "must be 'expense' or 'income'")]) }
    if len(problems) > 0 => Err(problems)
    else => Ok(#{ "name": clean_name, "kind": kind })
}

share fn validate_rename(doc) = {
    let name = if map_has_key(doc, "name") => map_get(doc, "name") else => ""
    let clean = if typeof(name) == "String" => str.trim(name) else => ""
    if clean == "" || str.length(clean) > 60 =>
        Err([problem("name", "must be 1-60 characters")])
    else => Ok(#{ "name": clean })
}

// ── budgets ──────────────────────────────────────────────────────────

share fn validate_budget(doc) = {
    let mut problems = []
    let cid = if map_has_key(doc, "category_id") => map_get(doc, "category_id") else => 0
    let month = if map_has_key(doc, "month") => map_get(doc, "month") else => ""
    let amount = if map_has_key(doc, "amount_cents") => map_get(doc, "amount_cents") else => -1
    if !(typeof(cid) == "Int" && cid > 0) =>
        { problems = concat(problems, [problem("category_id", "must be a positive integer")]) }
    if !valid_month(month) =>
        { problems = concat(problems, [problem("month", "must be YYYY-MM")]) }
    if !(typeof(amount) == "Int" && amount >= 0) =>
        { problems = concat(problems, [problem("amount_cents", "must be a non-negative integer (cents); 0 clears")]) }
    if len(problems) > 0 => Err(problems)
    else => Ok(#{ "category_id": cid, "month": month, "amount_cents": amount })
}

// ── module self-checks (olang test .) ────────────────────────────────

test "transaction validation catches each field and honors partial" {
    let good = unwrap(validate_transaction(
        #{ "date": "2026-08-15", "amount_cents": -1250, "category_id": 3, "note": "coffee" }, false))
    assert_eq(map_get(good, "amount_cents"), -1250)
    let bad = validate_transaction(#{ "date": "8/15/26", "amount_cents": 0, "category_id": 0 }, false)
    assert_true(is_err(bad), "three bad fields must fail")
    let problem_count = match bad { Err(problems) => len(problems), Ok(v) => 0 }
    assert_eq(problem_count, 3)
    let patch = unwrap(validate_transaction(#{ "note": "renamed" }, true))
    assert_eq(map_get(patch, "note"), "renamed")
    assert_true(is_err(validate_transaction(#{}, true)), "empty patch rejected")
    assert_true(is_err(validate_transaction(#{ "amount_cents": 12.5, "date": "2026-08-01", "category_id": 1 }, false)),
        "float cents rejected")
}

test "category, rename, and budget validation" {
    assert_true(is_ok(validate_category(#{ "name": "  Books ", "kind": "expense" })))
    assert_eq(map_get(unwrap(validate_category(#{ "name": " Books ", "kind": "expense" })), "name"), "Books")
    assert_true(is_err(validate_category(#{ "name": "", "kind": "expense" })))
    assert_true(is_err(validate_category(#{ "name": "x", "kind": "loan" })))
    assert_true(is_err(validate_rename(#{ "name": "   " })))
    assert_true(is_ok(validate_budget(#{ "category_id": 1, "month": "2026-08", "amount_cents": 0 })))
    assert_true(is_err(validate_budget(#{ "category_id": 1, "month": "2026-13", "amount_cents": 100 })))
    assert_true(is_err(validate_budget(#{ "category_id": 1, "month": "2026-8", "amount_cents": 100 })))
}

test "ids and months" {
    assert_eq(unwrap(validate_id("17")), 17)
    assert_true(is_err(validate_id("0")))
    assert_true(is_err(validate_id("abc")))
    assert_true(valid_month("2026-08"))
    assert_true(!valid_month("2026-08-01"))
}
