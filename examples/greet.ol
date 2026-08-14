// greet — a self-contained olang CLI tool, and a target for `olang build`.
// It uses only stdlib and the embedded `cli`/`term` packages, so it
// bundles into a single standalone binary:
//
//   olang build examples/greet.ol -o greet
//   ./greet Ada --loud -n 2
//
// `who` defaults to "world", so running it with no arguments (as the
// examples harness does) just greets the world.

use cli
use term

let spec = #{
    "name": "greet", "about": "greet someone, in color",
    "flags": [
        #{ "name": "loud", "short": "l", "type": "bool", "help": "SHOUT the greeting" },
        #{ "name": "count", "short": "n", "type": "int", "default": 1, "help": "repeat N times" }
    ],
    "args": [ #{ "name": "who", "default": "world", "help": "who to greet" } ]
}

match cli.parse(spec, cli.args()) {
    Err(e) => {
        println(term.red("greet: ") + e)
        os.exit(2)
    },
    Ok(a) => if map_get(a, "help") => println(cli.help(spec))
        else => {
            let base = "Hello, " + map_get(a, "who") + "!"
            let line = if map_get(a, "loud") => str.to_upper(base) else => base
            for i in range(0, map_get(a, "count")) { println(term.green(line)) }
        }
}
