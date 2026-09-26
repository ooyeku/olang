// size.ol — the terminal's size, and whether this is a terminal at all.
//
//   olang examples/tty/size.ol           80x24 (or whatever yours is)
//   olang examples/tty/size.ol | cat     not a terminal: says so, exits 0
//
// tty.size() needs only stdout to be a terminal; it does not enter raw
// mode. It answers Err when stdout is a pipe or a file.

match tty.size() {
    Ok((cols, rows)) => println(show(cols) + "x" + show(rows)),
    Err(e) => println("no terminal: " + e)
}
println("stdin and stdout are a terminal: " + show(tty.is_tty()))
