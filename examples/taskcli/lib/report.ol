use model { priority_label }

// Render one task row (a map from the db) as a display line.
share fn format_task(task) = {
    let mark = if map_get(task, "done") == 1 => "[x]" else => "[ ]"
    let pri = priority_label(map_get(task, "priority"))
    mark + " #" + to_string(map_get(task, "id")) + " (" + pri + ") " + map_get(task, "title")
}

// A stats summary over a task list, using col + pipelines.
share fn summary(tasks) = {
    let total = len(tasks)
    let done = tasks |> filter((t) => map_get(t, "done") == 1) |> len()
    let by_priority = col.count_by(tasks, (t) => priority_label(map_get(t, "priority")))
    {
        total: total,
        done: done,
        open: total - done,
        by_priority: by_priority
    }
}
