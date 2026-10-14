// `olang fix units`: floor, ceil, round and trunc answer an Int.

fn cells(w) = math.floor(w / 8.0)
fn rows(h) = math.ceil(h) + 1
fn near(x) = math.round(x * 2.0) / 2
fn cut(x) = math.trunc(x)

// left: not floor, ceil, round or trunc
fn root(x) = to_int(math.sqrt(x))
