// taskcli — a tiny task tracker, and a dogfood of the `cli` package.
// The command surface is a declarative spec; `cli.parse` turns argv into
// a validated map (or a clean error), and `cli.help` renders the usage.
// Run with no arguments to see that help.

use cli
use term
use lib.store { open_store, add_task, complete_task, all_tasks, open_tasks, close_store }
use lib.report { format_task, summary }

let spec = #{
    "name": "taskcli", "about": "a tiny task tracker",
    "commands": [
        #{ "name": "list", "about": "list every task" },
        #{ "name": "open", "about": "list only open tasks" },
        #{ "name": "stats", "about": "summary counts by state and priority" },
        #{ "name": "add", "about": "add a task",
           "args": [ #{ "name": "title", "required": true, "help": "the task title" } ],
           "flags": [ #{ "name": "priority", "short": "p", "type": "int", "default": 2,
                         "help": "1 (low) – 3 (high)" } ] }
    ]
}

fn run(a) = {
    let conn = open_store(":memory:")
    // Seed a little data so the demo shows real output. (A real CLI
    // would take a file path and persist; :memory: keeps it hermetic.)
    add_task(conn, "Write the parser", 3)
    add_task(conn, "Fix the tier bug", 3)
    add_task(conn, "Update the docs", 1)
    add_task(conn, "Ship 0.25.0", 2)
    complete_task(conn, 2)

    match map_get(a, "command") {
        "list" => {
            println(term.bold("All tasks"))
            for t in all_tasks(conn) { println("  " + format_task(t)) }
        },
        "open" => {
            println(term.bold("Open tasks"))
            for t in open_tasks(conn) { println("  " + format_task(t)) }
        },
        "stats" => {
            let s = summary(all_tasks(conn))
            // A colored summary line and a term.table for the breakdown —
            // both fall back to plain text automatically when piped.
            println(term.bold(to_string(s.total)) + " tasks · "
                + term.green(`${s.done} done`) + " · "
                + term.yellow(`${s.open} open`))
            let rows = map(sort(map_keys(s.by_priority)),
                (p) => [p, to_string(map_get(s.by_priority, p))])
            println(term.table(["priority", "count"], rows))
        },
        "add" => {
            add_task(conn, map_get(a, "title"), map_get(a, "priority"))
            println(term.green("added") + ": \"" + map_get(a, "title") + "\" (priority "
                + show(map_get(a, "priority")) + ")")
        },
        _ => println(cli.help(spec))
    }
    close_store(conn)
}

// No arguments (or -h) prints the usage and exits cleanly; a bad
// command or flag prints the error and exits non-zero.
match cli.parse(spec, cli.args()) {
    Err(e) => {
        println("taskcli: " + e)
        os.exit(2)
    },
    Ok(a) => if map_get(a, "help") => println(cli.help(spec)) else => run(a)
}
