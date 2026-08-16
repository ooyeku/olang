// Demonstrate the regex engine: compile a handful of patterns, show single
// matches and global find-all, then act as a tiny grep — with `olang main.ol
// PATTERN` the pattern is run against each of a few sample lines.

use lib.parse { parse }
use lib.matcher { matches, find, find_all }

fn demo(pat, s) = {
    let re = parse(pat)
    let f = find(re, s)
    let verdict = if f.found => "matched \"" + f.text + "\" at " + to_string(f.start)
        else => "no match"
    println("  " + str.pad_end("/" + pat + "/", 16, " ") + str.pad_end("\"" + s + "\"", 22, " ") + verdict)
}

println("═══ single matches ═══")
demo("a+b", "aaab")
demo("colou?r", "colour")
demo("(cat|dog)s?", "the dogs bark")
demo("^\\d+$", "2026")
demo("^\\d+$", "20x6")
demo("c.t", "cot")
demo("[A-Z][a-z]+", "hello World")

println("")
println("═══ find all ═══")
fn extract(pat, s) = {
    let hits = find_all(parse(pat), s)
    println("  /" + str.pad_end(pat + "/", 10, " ") + " in \"" + s + "\"")
    println("     => [" + join(hits |> map((h) => h.text), ", ") + "]")
}
extract("\\d+", "order 66 shipped 128 units over 3 days")
extract("\\w+@\\w+", "ping a@b, then cat@dog, done")
extract("(ab)+", "abab_ab_x_ababab")

let args = os.args()
if len(args) > 1 => {
    println("")
    println("═══ grep /" + args[1] + "/ ═══")
    let re = parse(args[1])
    let lines = ["the quick brown fox", "jumps over 42 lazy dogs", "in 2026, right?"]
    for line in lines {
        let mark = if matches(re, line) => "  ✓ " else => "    "
        println(mark + line)
    }
}
