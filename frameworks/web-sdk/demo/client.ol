// The browser half — served bundled with the SDK's browser modules.
use web { mount, action, action_arg, apply, rerender, input_value, call,
          stack, spread, card, muted, badge, btn, btn_primary,
          div, span, input, h1 }

fn todo_row(t) = spread(
    span(#{ "class": if map_get(t, "done") == 1 => "muted" else => "" },
        [map_get(t, "title")]),
    btn(if map_get(t, "done") == 1 => "undo" else => "done",
        "toggle:" + to_string(map_get(t, "id"))))

fn view(s) = stack([
    h1(#{}, ["todos"]),
    card([
        div(#{ "class": "row" }, [
            input(#{ "id": "title", "class": "field-input", "placeholder": "What next?" }),
            btn_primary("Add", "add")
        ])
    ]),
    stack(map(map_get(s, "todos"), (t) => card([todo_row(t)]))),
    muted(to_string(len(map_get(s, "todos"))) + " item(s)")
])

fn refresh() =
    call("todos.list", #{}, (r) => {
        match r {
            Ok(todos) => { apply((s) => map_set(s, "todos", todos)) },
            Err(msg) => ()
        }
    })

action("add", (ev) =>
    call("todos.create", #{ "title": input_value("title") }, (r) => refresh()))

action("toggle", (ev) =>
    call("todos.toggle", #{ "id": unwrap(str.parse_int(action_arg(ev))) },
        (r) => refresh()))

mount("#app", view, #{ "todos": [] })
refresh()
