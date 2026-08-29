// Request validation: JSON in, either a clean field map out or a list
// of { field, message } problems for a 422. The rules are the schema's
// real constraints — enums, lengths, ranges — enforced before SQL ever
// sees the values.

let statuses = ["open", "in-progress", "done"]
let priorities = ["low", "medium", "high"]

// Validate an issue document. `partial` = true for PATCH (absent fields
// fine), false for POST (defaults fill in). Returns
// Ok(#{field: value, ...}) or Err([{field, message}, ...]).
share fn validate_issue(doc, partial) = {
    let mut clean = #{}
    let mut problems = []

    if map_has_key(doc, "title") => {
        let t = str.trim(show(map_get(doc, "title")))
        if str.length(t) == 0 => { problems = concat(problems, [{ field: "title", message: "must not be empty" }]) }
        else => if str.length(t) > 200 => { problems = concat(problems, [{ field: "title", message: "at most 200 characters" }]) }
        else => { clean = map_set(clean, "title", t) }
    } else => if !partial => { clean = map_set(clean, "title", "New issue") }

    if map_has_key(doc, "status") => {
        let s = show(map_get(doc, "status"))
        if contains(statuses, s) => { clean = map_set(clean, "status", s) }
        else => { problems = concat(problems, [{ field: "status", message: "one of: " + join(statuses, ", ") }]) }
    } else => if !partial => { clean = map_set(clean, "status", "open") }

    if map_has_key(doc, "priority") => {
        let p = show(map_get(doc, "priority"))
        if contains(priorities, p) => { clean = map_set(clean, "priority", p) }
        else => { problems = concat(problems, [{ field: "priority", message: "one of: " + join(priorities, ", ") }]) }
    } else => if !partial => { clean = map_set(clean, "priority", "medium") }

    if map_has_key(doc, "assignee") => {
        let a = str.trim(show(map_get(doc, "assignee")))
        if str.length(a) > 40 => { problems = concat(problems, [{ field: "assignee", message: "at most 40 characters" }]) }
        else => { clean = map_set(clean, "assignee", a) }
    } else => if !partial => { clean = map_set(clean, "assignee", "") }

    if map_has_key(doc, "points") => {
        let raw = map_get(doc, "points")
        let n = if typeof(raw) == "Int" => raw
            else => if typeof(raw) == "Float" => to_int(raw)
            else => -1
        if n < 0 || n > 100 => { problems = concat(problems, [{ field: "points", message: "an integer from 0 to 100" }]) }
        else => { clean = map_set(clean, "points", n) }
    } else => if !partial => { clean = map_set(clean, "points", 0) }

    if map_has_key(doc, "notes") => {
        let nts = show(map_get(doc, "notes"))
        if str.length(nts) > 2000 => { problems = concat(problems, [{ field: "notes", message: "at most 2000 characters" }]) }
        else => { clean = map_set(clean, "notes", nts) }
    } else => if !partial => { clean = map_set(clean, "notes", "") }

    if len(problems) > 0 => Err(problems)
    else => if partial && len(map_keys(clean)) == 0 =>
        Err([{ field: "(body)", message: "no known fields to update" }])
    else => Ok(clean)
}

// Validate a comment body: { text (required, 1..2000), author (<=40) }.
share fn validate_comment(doc) = {
    let mut problems = []
    let text = if map_has_key(doc, "text") => str.trim(show(map_get(doc, "text"))) else => ""
    if str.length(text) == 0 => { problems = concat(problems, [{ field: "text", message: "must not be empty" }]) }
    if str.length(text) > 2000 => { problems = concat(problems, [{ field: "text", message: "at most 2000 characters" }]) }
    let author = if map_has_key(doc, "author") => str.trim(show(map_get(doc, "author"))) else => ""
    if str.length(author) > 40 => { problems = concat(problems, [{ field: "author", message: "at most 40 characters" }]) }
    if len(problems) > 0 => Err(problems)
    else => Ok({ text: text, author: author })
}

// A positive integer id out of a path parameter, or Err.
share fn validate_id(raw) = match str.parse_int(raw) {
    Ok(n) => if n > 0 => Ok(n) else => Err("id must be positive"),
    Err(e) => Err("id must be an integer")
}
