// The task domain: priority labels and a status helper.

// Priority as a small tagged value; higher number = more urgent.
share fn priority_label(p) = match p {
    3 => "high",
    2 => "medium",
    1 => "low",
    _ => "none"
}

// A human status string from the `done` flag.
share fn status_label(done) = if done == 1 => "done" else => "open"
