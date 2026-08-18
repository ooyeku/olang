// A mustache-style template engine, self-hosted in olang: the lexer turns
// raw text into tokens, the parser nests them into a tree, and the renderer
// walks the tree against a context. The context here is loaded from JSON, so
// the same `map_get` path access serves both native objects and parsed JSON.

use lib.lexer { tokenize }
use lib.parser { parse }
use lib.render { render }

let args = os.args()
let tmpl_path = if len(args) > 1 => args[1] else => "templates/report.tmpl"

let template = unwrap(fs.read_file(tmpl_path))

// The rendering context: a JSON document with nested objects, lists, a flag.
let ctx = unwrap(json.parse("{
    \"customer\": { \"name\": \"Ada Lovelace\", \"email\": \"ada@analytical.engine\", \"vip\": true },
    \"items\": [
        { \"name\": \"Difference Engine time\", \"qty\": 3, \"price\": 120 },
        { \"name\": \"Punch cards (box)\", \"qty\": 12, \"price\": 5 }
    ],
    \"tags\": [\"priority\", \"net-30\"],
    \"total\": 420
}"))

// The pipeline: text -> tokens -> nodes -> rendered text.
let tokens = tokenize(template)
let nodes = parse(tokens)
let output = render(nodes, ctx)

println("═══ rendered ═══")
print(output)
println("═══ ══════ ═══")
println(`tokens: ${len(tokens)}, top-level nodes: ${len(nodes)}`)
