// Load a schema and one or more JSON documents from disk, validate each, and
// report the pathed errors. With `olang main.ol path/to/doc.json` a specific
// document is checked; otherwise the bundled valid/invalid samples run.

use lib.validate { validate }

fn load(path) = unwrap(json.parse(unwrap(fs.read_file(path))))

fn report(label, schema, data) = {
    let errs = validate(schema, data)
    println("── " + label + " ──")
    if len(errs) == 0 => { println("   ✓ valid") }
    else => {
        println("   ✗ " + to_string(len(errs)) + " error(s):")
        for e in errs { println("     • " + e) }
    }
    println("")
}

let schema = load("schema.json")

let args = os.args()
if len(args) > 1 => {
    report(args[1], schema, load(args[1]))
}
else => {
    report("data/valid.json", schema, load("data/valid.json"))
    report("data/invalid.json", schema, load("data/invalid.json"))
}
