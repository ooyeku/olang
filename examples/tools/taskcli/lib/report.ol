use model { priority_label }

// Render one task row (a map from the db) as a display line.
share fn format_task(task) = {
    let mark = if task.done == 1 => "[x]" else => "[ ]"
    let pri = priority_label(task.priority)
    `${mark} #${task.id} (${pri}) ${task.title}`
}

// A stats summary over a task list, using col + pipelines.
share fn summary(tasks) = {
    let total = len(tasks)
    let done = tasks |> filter((t) => t.done == 1) |> len()
    let by_priority = col.count_by(tasks, (t) => priority_label(t.priority))
    {
        total: total,
        done: done,
        open: total - done,
        by_priority: by_priority
    }
}
