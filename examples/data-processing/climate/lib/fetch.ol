//! fetch — cached, retrying dataset downloads.
//!
//! A dataset is fetched at most once: a cached file that passes the
//! size sanity check is reused, a file that fails it (a partial or
//! truncated earlier download) is deleted and re-fetched. Downloads
//! write to a `.part` path and move into place only when complete, so
//! a crash mid-download can never leave a file the cache would trust.

/// Download `url` to `dest` unless a sane cached copy exists.
/// `min_bytes` is the sanity floor — these datasets are megabytes, so
/// anything smaller is a partial file or an error page, not data.
/// Returns Ok(#{ "path", "bytes", "cached" }) or Err(message).
share fn fetch(url, dest, min_bytes) = {
    unwrap(fs.create_dir_all(fs.dirname(dest)))
    if fs.exists(dest) => {
        let size = unwrap(fs.file_size(dest))
        if size >= min_bytes => {
            return Ok(#{ "path": dest, "bytes": size, "cached": true })
        }
        println(`  cached ${dest} is ${size} bytes — too small, re-fetching`)
        unwrap(fs.remove_file(dest))
    }

    let mut attempt = 1
    let mut last_error = ""
    while attempt <= 3 {
        match try_fetch(url, dest, min_bytes) {
            Ok(bytes) => return Ok(#{ "path": dest, "bytes": bytes, "cached": false }),
            Err(e) => {
                last_error = e
                println(`  attempt ${attempt} failed: ${e}`)
                if attempt < 3 => time.sleep(1000 * attempt)
            }
        }
        attempt = attempt + 1
    }
    Err(`download failed after 3 attempts: ${last_error}`)
}

fn try_fetch(url, dest, min_bytes) = {
    let response = http.get(url, #{ "timeout_ms": 120000 })?
    let status = map_get(response, "status")
    if status != 200 => return Err(`HTTP ${status} from ${url}`)
    let body = map_get(response, "body")
    let size = str.length(body)
    if size < min_bytes => return Err(`response is ${size} bytes — expected at least ${min_bytes}`)
    let part = dest + ".part"
    fs.write_file(part, body)?
    fs.move_file(part, dest)?
    Ok(size)
}
