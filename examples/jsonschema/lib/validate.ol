// A JSON Schema validator (a practical subset), self-hosted in olang. Both the
// schema and the document are ordinary parsed-JSON values, so validation is a
// recursive walk of the two in step, collecting an error per violated
// keyword. Each error carries a JSON-path (`$.address.zip`) so failures point
// at exactly where they occurred.
//
// Supported keywords: type, enum, required, properties, items, minimum,
// maximum, minLength, maxLength, minItems, maxItems.

// The schema's notion of a value's type. JSON has one number kind, but olang
// tells integers from floats, which lets us honour "integer" vs "number".
fn json_type(v) = {
    let t = typeof(v)
    if t == "Int" => "integer"
    else if t == "Float" => "number"
    else if t == "String" => "string"
    else if t == "Bool" => "boolean"
    else if t == "List" => "array"
    else if t == "JsonObject" => "object"
    else if t == "Object" => "object"
    else if t == "Unit" => "null"
    else => "unknown"
}

// Does `data` satisfy a schema `type`? "number" also accepts integers.
fn type_ok(expected, data) = {
    let actual = json_type(data)
    if expected == "number" => (actual == "integer") || (actual == "number")
    else => actual == expected
}

// Is `data` numeric (integer or float)?
fn is_number(data) = {
    let t = json_type(data)
    (t == "integer") || (t == "number")
}

// The heart of it: return the list of error strings for `data` against
// `schema` at `path`. Nested objects and arrays recurse with an extended path.
share fn check(schema, data, path) = {
    let mut errs = []

    if map_has_key(schema, "type") => {
        let t = map_get(schema, "type")
        if !type_ok(t, data) =>
            { errs = errs + [path + ": expected " + t + ", got " + json_type(data)] }
    }

    if map_has_key(schema, "enum") => {
        let mut ok = false
        for allowed in map_get(schema, "enum") { if allowed == data => { ok = true } }
        if !ok => { errs = errs + [path + ": " + to_string(data) + " is not an allowed value"] }
    }

    if json_type(data) == "object" => {
        if map_has_key(schema, "required") => {
            for req in map_get(schema, "required") {
                if !map_has_key(data, req) =>
                    { errs = errs + [path + "." + req + ": required property is missing"] }
            }
        }
        if map_has_key(schema, "properties") => {
            let props = map_get(schema, "properties")
            for key in map_keys(props) {
                if map_has_key(data, key) =>
                    { errs = errs + check(map_get(props, key), map_get(data, key), path + "." + key) }
            }
        }
    }

    if json_type(data) == "array" => {
        if map_has_key(schema, "items") => {
            let item_schema = map_get(schema, "items")
            let mut i = 0
            for elem in data {
                errs = errs + check(item_schema, elem, path + "[" + to_string(i) + "]")
                i = i + 1
            }
        }
        if map_has_key(schema, "minItems") && (len(data) < map_get(schema, "minItems")) =>
            { errs = errs + [path + ": has " + to_string(len(data)) + " items, needs at least " + to_string(map_get(schema, "minItems"))] }
        if map_has_key(schema, "maxItems") && (len(data) > map_get(schema, "maxItems")) =>
            { errs = errs + [path + ": has " + to_string(len(data)) + " items, allows at most " + to_string(map_get(schema, "maxItems"))] }
    }

    if is_number(data) => {
        if map_has_key(schema, "minimum") && (data < map_get(schema, "minimum")) =>
            { errs = errs + [path + ": " + to_string(data) + " is below minimum " + to_string(map_get(schema, "minimum"))] }
        if map_has_key(schema, "maximum") && (data > map_get(schema, "maximum")) =>
            { errs = errs + [path + ": " + to_string(data) + " is above maximum " + to_string(map_get(schema, "maximum"))] }
    }

    if json_type(data) == "string" => {
        if map_has_key(schema, "minLength") && (str.length(data) < map_get(schema, "minLength")) =>
            { errs = errs + [path + ": length " + to_string(str.length(data)) + " is below minLength " + to_string(map_get(schema, "minLength"))] }
        if map_has_key(schema, "maxLength") && (str.length(data) > map_get(schema, "maxLength")) =>
            { errs = errs + [path + ": length " + to_string(str.length(data)) + " is above maxLength " + to_string(map_get(schema, "maxLength"))] }
    }

    errs
}

// Validate `data` against `schema`; returns the (possibly empty) error list.
share fn validate(schema, data) = check(schema, data, "$")
