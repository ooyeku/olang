// An inventory model whose boundaries are contracts.
//
// The contracts are macros from `use contracts`: violations quote the
// condition's own source, and the format strings in the report are
// checked against their arguments before the program runs. Run
// `olang expand main.ol` to see what the checks compile into.

use contracts

fn withdraw(stock, n) = {
    @require(n > 0)
    @require(stock >= n)
    let remaining = stock - n
    @ensure(remaining >= 0)
    remaining
}

println("═══ contracts: checked boundaries ═══")

let after = withdraw(10, 3)
println(@fmtc("stock {} after withdrawing {}", [after, 3]))

let line = @fmtc("{} unit(s) remain; reorder at {}", [after, 5])
println(line)

test "contracts hold on the good path" {
    assert_eq(withdraw(10, 3), 7)
    assert_eq(withdraw(5, 5), 0)
}

test "a violated contract quotes its own source" {
    let t = spawn withdraw(3, 9)
    match task.join(t) {
        Err(e) => {
            assert_true(str.contains(e, "contract violated (require)"))
            assert_true(str.contains(e, "stock >= n"))
        },
        v => assert_true(false)
    }
}

test "checked formatting renders" {
    assert_eq(@fmtc("{} + {} = {}", [1, 2, 3]), "1 + 2 = 3")
}
