// left: the file binds `last` itself
fn last(xs) = 0
fn tail(xs: [Int]) = if len(xs) == 0 => () else => xs[len(xs) - 1]
