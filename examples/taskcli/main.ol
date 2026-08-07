use lib.store { open_store, add_task, complete_task, all_tasks, open_tasks, close_store }
use lib.report { format_task, summary }

// argv: [main.ol, <command>, <args...>]
let args = unwrap(os.args())
let cmd = if len(args) > 1 => args[1] else => "help"

let conn = open_store(":memory:")

// Seed a little data so the demo shows real output. (A real CLI would use a
// file path and persist; :memory: keeps the example self-contained.)
add_task(conn, "Write the parser", 3)
add_task(conn, "Fix the tier bug", 3)
add_task(conn, "Update the docs", 1)
add_task(conn, "Ship 0.25.0", 2)
complete_task(conn, 2)

match cmd {
    "list" => {
        println("All tasks:")
        for t in all_tasks(conn) {
            println("  " + format_task(t))
        }
    },
    "open" => {
        println("Open tasks:")
        for t in open_tasks(conn) {
            println("  " + format_task(t))
        }
    },
    "stats" => {
        let s = summary(all_tasks(conn))
        println("Tasks: " + to_string(s.total) + " total, " + to_string(s.done) + " done, " + to_string(s.open) + " open")
        println("By priority:")
        for p in sort(map_keys(s.by_priority)) {
            println("  " + p + ": " + to_string(map_get(s.by_priority, p)))
        }
    },
    _ => {
        println("taskcli — commands: list, open, stats")
        println("  e.g.  olang main.ol stats")
    }
}

close_store(conn)
