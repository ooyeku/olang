// echo.ol — raw-mode key echo: every event the terminal sends is printed
// as it arrives, until q or ctrl+c.
//
//   olang examples/tty/echo.ol            raw mode on the main screen
//   olang examples/tty/echo.ol --alt      the alternate screen, with mouse,
//                                         bracketed paste, and focus events
//   olang examples/tty/echo.ol --kitty    ... and the kitty keyboard protocol
//
// Three keys exist to show the restore guard at work: `!` raises an
// uncaught error (the terminal is restored, then the error printed), `x`
// calls os.exit(3), and `z` suspends to the shell (`fg` brings it back).
// A SIGTERM from another shell restores it too.

if !tty.is_tty() => {
    println("echo.ol needs a terminal on stdin and stdout")
    os.exit(0)
}

let args = os.args()
let alt = contains(args, "--alt")
let kitty = contains(args, "--kitty")
let h = unwrap(tty.enter(#{ "alt_screen": alt, "mouse": alt, "paste": true, "focus": true, "kitty_keys": kitty }))
let events = tty.events(h)

// Raw mode: "\n" only moves down a line, so every line ends "\r\n".
fn say(h, line) = { let _ = tty.write(h, line + "\r\n"); () }

// Events are maps: read them with map_get.
fn f(ev, k) = map_get(ev, k)

fn describe(ev) = match f(ev, "kind") {
    "key" => "key    " + f(ev, "chord") + (if f(ev, "text") != "" && f(ev, "text") != f(ev, "chord") => "  text " + show(f(ev, "text")) else => ""),
    "paste" => "paste  " + show(f(ev, "text")),
    "mouse" => "mouse  " + f(ev, "action") + " " + f(ev, "button") + " at " + show(f(ev, "x")) + "," + show(f(ev, "y")),
    "resize" => "resize " + show(f(ev, "cols")) + "x" + show(f(ev, "rows")),
    "focus" => "focus  " + (if f(ev, "on") => "in" else => "out"),
    "signal" => "signal " + f(ev, "name"),
    other => show(ev)
}

let (cols, rows) = unwrap(tty.size())
say(h, "echo: " + show(cols) + "x" + show(rows) + " — press keys; q or ctrl+c quits, ! raises, x exits 3, z suspends")

let mut going = true
while going {
    match chan.recv(events) {
        Ok(ev) => {
            say(h, describe(ev))
            let chord = if f(ev, "kind") == "key" => f(ev, "chord") else => ""
            if chord == "q" || chord == "ctrl+c" => { going = false }
            else if chord == "!" => { unwrap(Err("raised on purpose from echo.ol")) }
            else if chord == "x" => { os.exit(3) }
            else if chord == "z" => { let _ = tty.suspend(h) }
        },
        Err(e) => { going = false }
    }
}
tty.leave(h)
println("echo: bye")
