use lib.parse { parse_line }
use lib.stats { level_counts, errors, top_routes }

let args = os.args()
let path = if len(args) > 1 => args[1] else => "data/app.log"

if !fs.exists(path) => {
    println("no such file: " + path)
} else => {
    let content = unwrap(fs.read_file(path))
    let lines = str.lines(content) |> filter((l) => str.trim(l) != "")

    // Parse every line; separate the good records from malformed lines.
    let parsed = lines |> map(parse_line)
    let records = parsed |> filter(is_ok) |> map(unwrap)
    let bad = parsed |> filter(is_err) |> len()

    println("═══ log analysis: " + path + " ═══")
    println("lines: " + to_string(len(lines)) + ", parsed: " + to_string(len(records)) + ", malformed: " + to_string(bad))

    println("── by level ──")
    let counts = level_counts(records)
    for level in sort(map_keys(counts)) {
        println("  " + str.pad_end(level, 6, " ") + to_string(map_get(counts, level)))
    }

    println("── errors ──")
    for msg in errors(records) {
        println("  " + msg)
    }

    println("── top routes ──")
    for pair in top_routes(records) {
        let (route, count) = pair
        println("  " + str.pad_end(route, 14, " ") + to_string(count))
    }
}
