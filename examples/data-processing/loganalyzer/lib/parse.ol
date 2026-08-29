// Parse one log line into a record. Returns Ok(record) or Err(line) so
// malformed lines surface rather than silently vanish.
let LINE = "^(\\S+ \\S+) \\[(\\w+)\\] (.+)$"

share fn parse_line(line) = match re.captures(LINE, line) {
    Ok(groups) => if len(groups) >= 4 =>
        Ok({ time: groups[1], level: groups[2], message: groups[3] })
    else => Err(line),
    Err(e) => Err(line)
}

// Extract an HTTP route from a message, or "" when there isn't one.
share fn route_of(message) = match re.find("/\\w[\\w/]*", message) {
    Ok(r) => r,
    Err(e) => ""
}
