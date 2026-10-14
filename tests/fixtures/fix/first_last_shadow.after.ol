// left: the file binds `last` itself
fn last(xs) = 0
fn tail(xs: [Int]) = if len(xs) == 0 => () else => xs[len(xs) - 1]

// left: the script binds `first` inside a block, and the checker reads
// the script as one scope
let xs = [1, 2]
if len(xs) > 1 => {
    let mut first = true
    first = false
}
let y = if len(xs) > 0 => xs[0] else => ()
