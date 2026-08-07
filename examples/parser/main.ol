// Demonstrate the parser combinators: first a few small parsers directly,
// then the arithmetic evaluator built from them. An expression passed on the
// command line is evaluated too, so `olang main.ol "2*(3+4)"` works.

use lib.combinators {
    satisfy, is_digit, is_alpha, char_p, str_p, pmap, many1, sep_by1, run
}
use lib.calc { evaluate }

// ── a couple of parsers used on their own ──
let integer = pmap(many1(satisfy("digit", is_digit)), (ds) => unwrap(str.parse_int(join(ds, ""))))
let word = pmap(many1(satisfy("letter", is_alpha)), (cs) => join(cs, ""))
let csv_ints = sep_by1(integer, char_p(","))

println("═══ primitive parsers ═══")
println("  integer \"2026\"      => " + to_string(run(integer, "2026").val))
println("  word \"olang\"        => " + run(word, "olang").val)
let nums = run(csv_ints, "3,14,159,26")
println("  csv \"3,14,159,26\"   => " + to_string(len(nums.val)) + " ints, sum " +
    to_string(nums.val |> fold(0, (a, b) => a + b)))

println("")
println("═══ arithmetic evaluator ═══")
fn show(text) = {
    let r = evaluate(text)
    let out = if r.ok => to_string(r.value) else => "parse error at column " + to_string(r.at)
    println("  " + str.pad_end(text, 20, " ") + " = " + out)
}

show("1 + 2 * 3")
show("(1 + 2) * 3")
show("2 * 3 + 4 * 5")
show("100 / 5 / 2")
show("((7 - 2) * (3 + 1)) / 4")
show("1 +")          // deliberately malformed
show("(1 + 2")       // unbalanced

let args = unwrap(os.args())
if len(args) > 1 => {
    println("")
    println("═══ from the command line ═══")
    show(args[1])
}
