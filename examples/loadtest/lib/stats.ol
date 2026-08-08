// Client-side measurement: each client worker returns one Stats value, and
// the coordinator folds them together after `await`. No shared mutable
// state anywhere — merging after the join is the whole concurrency story.

share type Stats = struct {
    requests: Int,
    oks: Int,          // 2xx
    expected_5xx: Int, // the /oops route SHOULD 500
    shed_503: Int,     // backpressure responses
    failures: Int,     // transport errors / wrong statuses
    total_ms: Int,
    max_ms: Int
}

share fn empty_stats() = Stats {
    requests: 0, oks: 0, expected_5xx: 0, shed_503: 0,
    failures: 0, total_ms: 0, max_ms: 0
}

// Record one request's outcome. `expect_error` marks the deliberate-500
// route, where a 500 is the correct answer.
share fn record(s, status, ms, expect_error) = {
    let ok2xx = (status >= 200) && (status < 300)
    Stats {
        requests: s.requests + 1,
        oks: s.oks + (if ok2xx && !expect_error => 1 else => 0),
        expected_5xx: s.expected_5xx + (if expect_error && (status == 500) => 1 else => 0),
        shed_503: s.shed_503 + (if status == 503 => 1 else => 0),
        failures: s.failures + (if !ok2xx && (status != 503) && !(expect_error && (status == 500)) => 1 else => 0),
        total_ms: s.total_ms + ms,
        max_ms: if ms > s.max_ms => ms else => s.max_ms
    }
}

share fn merge(a, b) = Stats {
    requests: a.requests + b.requests,
    oks: a.oks + b.oks,
    expected_5xx: a.expected_5xx + b.expected_5xx,
    shed_503: a.shed_503 + b.shed_503,
    failures: a.failures + b.failures,
    total_ms: a.total_ms + b.total_ms,
    max_ms: if a.max_ms > b.max_ms => a.max_ms else => b.max_ms
}
