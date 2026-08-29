// The search worker: everything a spawned thread runs. A worker takes a
// chunk of file paths and a pattern and returns its hits — no shared state,
// no locks; the merge happens after `task.join` in the coordinator, which is
// the whole trick of capture-by-value concurrency.

share type FileHits = struct { file: String, lines: List, count: Int }

// Search one file: the matching line numbers (1-based) for a regex pattern.
// Unreadable files count as zero hits rather than failing the worker.
share fn search_file(path, pattern) = {
    let text = unwrap_or(fs.read_file(path), "")
    let mut lines = []
    for (i, line) in enumerate(str.lines(text)) {
        if unwrap_or(re.is_match(pattern, line), false) => {
            lines = lines + [i + 1]
        }
    }
    FileHits { file: path, lines: lines, count: len(lines) }
}

// The chunk body a worker executes: search every file, keep the ones with
// hits.
share fn search_chunk(paths, pattern) = {
    paths
        |> map((p) => search_file(p, pattern))
        |> filter((h) => h.count > 0)
}
