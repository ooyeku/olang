// `olang fix units`: floor, ceil, round and trunc answer an Int.

fn cells(w) = to_int(math.floor(w / 8.0))
fn rows(h) = to_int(math.ceil(h)) + 1
fn near(x) = to_int(math.round(x * 2.0)) / 2
fn cut(x) = to_int(math.trunc(x))

// left: not floor, ceil, round or trunc
fn root(x) = to_int(math.sqrt(x))
