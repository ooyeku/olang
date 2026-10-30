// A node's process, end to end: it reads its first stdin line, counts
// its boots on its disk, keeps a shutdown handler, and serves what it
// knows. A "quit" line on stdin ends it with os.exit(0).
//
//   GET /state  -> #{ first, mode, extra, boots, bye }
//   POST /clear -> removes /bye (and syncs the directory)

let first = match os.read_line() { Ok(l) => l, Err(e) => "(" + e + ")" }
let boots = match fs.read_file("/boots") { Ok(t) => unwrap(str.parse_int(t)), Err(e) => 0 } + 1
unwrap(fs.write_file("/boots", to_string(boots)))
unwrap(fs.sync("/boots"))
// this process's environment: a restart starts from the node's again
if !os.has_env("EXTRA") => os.set_env("EXTRA", "boot " + to_string(boots)) else => ()

unwrap(os.on_shutdown((why) => {
    unwrap(fs.write_file("/bye", why))
    unwrap(fs.sync("/bye"))
    http.shutdown()
}))

let lifeline = spawn {
    let mut open = true
    while open {
        match os.read_line() {
            Ok(line) => if line == "quit" => os.exit(0) else => (),
            Err(e) => { open = false }
        }
    }
}

fn handle(req) = {
    if req.path == "/state" => http.response(200, unwrap(json.stringify(#{
        "first": first,
        "mode": match os.get_env("MODE") { Ok(m) => m, Err(e) => "" },
        "extra": unwrap(os.get_env("EXTRA")),
        "boots": boots,
        "bye": fs.exists("/bye") })))
    else if req.path == "/clear" => {
        unwrap(fs.remove_file("/bye"))
        unwrap(fs.sync("/"))
        http.response(200, "cleared")
    }
    else => http.response(404, "no")
}

http.serve(7700, handle)
