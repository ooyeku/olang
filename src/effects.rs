//! The single classification of the stdlib's effectful surface.
//!
//! Three independent policies care about "what touches the world":
//!
//! - the **capability gate** (`caps::check` / `caps::required`) asks
//!   *may this call run under the current grant?*
//! - **macro expansion** (`Interpreter::expansion_denial`) asks *is this
//!   call allowed inside a `meta fn`, where expansion must be a pure
//!   function of the source?*
//! - **record/replay** (`Timeline::is_recorded`) asks *does this call's
//!   result have to be captured for a run to be reproducible?*
//!
//! These used to be three independently curated lists, and they drifted:
//! the expansion denylist predated `ods` file I/O (so a macro could read
//! and write files at expansion time), missed `dates.now` (so a macro
//! could bake the wall clock into a program), and missed
//! `crypto.random_*` — while the timeline knew all three were
//! nondeterministic. One classification, three consumers, so a new
//! stdlib function is classified exactly once and every policy sees it.
//!
//! **The rule for adding a stdlib function:** if it reads or writes
//! anything outside the program — files, network, processes, clock,
//! RNG state, environment, console — give it a row here (`cap` and/or
//! `recorded`). Everything else is pure computation and needs nothing.
//! `cross_consumer_agreement` in the tests below will fail loudly if the
//! consumers ever disagree about a name they both know.

use crate::caps::CapUse;

/// One builtin's classification. Absent aspects mean "not that kind of
/// effect": a pure function is `Effects { cap: None, recorded: false }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Effects {
    /// The capability this call exercises, if the gate restricts it.
    pub cap: Option<CapUse>,
    /// Whether record/replay must capture this call's result for a trace
    /// to replay deterministically. A subset of "nondeterministic or
    /// input-reading"; results must round-trip through the trace format,
    /// so calls returning native handles (`ods.open_csv`) are gated but
    /// not recorded — a documented replay limitation.
    pub recorded: bool,
}

/// Classify a fully-qualified builtin name ("fs.read_file", "par_map").
/// This is the one table; the consumers below derive their answers.
pub fn classify(full_name: &str) -> Effects {
    Effects {
        cap: cap_of(full_name),
        recorded: recorded_of(full_name),
    }
}

/// The capability a builtin exercises, or None for a pure helper. This is
/// the table `caps::required` re-exports; see the comments there for the
/// reasoning behind individual classifications (why `os.exit` is `proc`,
/// why `os.chdir` is a write, why `ods.next_chunk` is ungated).
fn cap_of(full_name: &str) -> Option<CapUse> {
    if let Some(f) = full_name.strip_prefix("fs.") {
        if fs_pure(f) {
            return None;
        }
        return Some(if fs_read_only(f) {
            CapUse::FsRead
        } else {
            CapUse::FsWrite
        });
    }
    if let Some(f) = full_name.strip_prefix("http.") {
        if http_pure(f) {
            return None;
        }
        return Some(CapUse::Net);
    }
    if full_name.starts_with("db.") {
        return Some(CapUse::Db);
    }
    // The data stack is pure except where it touches files. Those calls
    // demand `fs` at the matching level, so a program granted `db` and
    // `net` but not `fs` cannot read a CSV off disk through `ods` — the
    // latent-capability hole `db.open` had before it was gated. Every new
    // reader or writer belongs in one of these two lists; the parsers
    // that take text (`read_csv`, `read_jsonl`) stay pure and ungated.
    // `next_chunk` reads from a handle already obtained under a grant,
    // the way a Unix read(2) needs no fresh permission for an open fd.
    if matches!(
        full_name,
        "ods.read_csv_file"
            | "ods.open_csv"
            | "ods.read_jsonl_file"
            | "ods.open_jsonl"
            | "ods.read_frame"
            | "ods.frame_info"
    ) {
        return Some(CapUse::FsRead);
    }
    if matches!(
        full_name,
        "ods.write_csv" | "ods.write_jsonl" | "ods.write_frame"
    ) {
        return Some(CapUse::FsWrite);
    }
    // Process-affecting calls, whatever module they live in. `os.exit`
    // was the hole: a dependency denied `proc` could not spawn a process
    // but could still terminate the host, which is a larger power than
    // the one it was refused. `proc` means "may affect processes",
    // including this one.
    if full_name.starts_with("proc.") || matches!(full_name, "os.exec" | "os.exit") {
        return Some(CapUse::Proc);
    }
    if let Some(f) = full_name.strip_prefix("os.") {
        if os_env_gated(f) {
            return Some(CapUse::Env);
        }
        // chdir moves the fs cursor — a write-level fs demand.
        if f == "chdir" {
            return Some(CapUse::FsWrite);
        }
        return None;
    }
    None
}

/// Whether record/replay captures this call's result. Keep in sync with
/// the trace format's expectations: everything here returns a plain
/// value that round-trips through the trace.
fn recorded_of(op: &str) -> bool {
    // random: every draw consumes hidden RNG state.
    if let Some(f) = op.strip_prefix("random.") {
        return !matches!(f, "seed"); // seed is a deterministic state-set
    }
    // The database: every call's answer depends on state outside the
    // program (the file, other writers), and every answer is plain data
    // — rows as maps, counts, and the connection handle itself, a struct
    // of an id and a path. Recording `db.open` too is what lets a replay
    // run without the database: the handle replays, and every call on it
    // replays after it.
    if op.starts_with("db.") {
        return true;
    }
    // What the main thread receives from other threads. A worker's own
    // effects are its business; what the program observed of the worker
    // — the message a channel delivered, the value a join returned — is
    // the main thread's input, and it is logged in the order it arrived.
    // `chan.send` is recorded so a replay, which runs no workers, does
    // not fill a channel nobody drains.
    if matches!(
        op,
        "chan.recv"
            | "chan.recv_timeout"
            | "chan.try_recv"
            | "chan.ask"
            | "chan.send"
            | "task.join"
            | "task.join_timeout"
    ) {
        return true;
    }
    // clocks: wall-clock and monotonic time.
    matches!(
        op,
        "time.now_ms"
            | "time.monotonic_ms"
            | "dates.now"
            | "dates.utc_now"
            | "dates.today"
            // environment and console input.
            | "os.get_env"
            | "os.list_env"
            | "os.has_env"
            | "os.hostname"
            | "os.username"
            | "os.home_dir"
            | "os.temp_dir"
            | "os.read_line"
            | "os.stdin"
            | "os.stdin_lines"
            | "os.exec"
            // machine identity and process context — the axes that
            // differ across machines and runs, so recording them is what
            // makes a trace portable (a program branching on os.arch()
            // replays the arch it was recorded on).
            | "os.os_type"
            | "os.arch"
            | "os.family"
            | "os.path_separator"
            | "os.args"
            | "os.cwd"
            | "os.exe_path"
            | "os.pid"
            | "os.is_tty"
            // filesystem reads: external, mutable state.
            | "fs.read_file"
            | "fs.exists"
            | "fs.is_file"
            | "fs.is_dir"
            | "fs.list_dir"
            | "fs.walk"
            | "fs.glob"
            | "fs.file_info"
            | "fs.file_size"
            // the network.
            | "http.get"
            | "http.post"
            | "http.put"
            | "http.delete"
            | "http.request"
            // seeded-random crypto.
            | "crypto.random_bytes"
            | "crypto.random_hex"
            | "crypto.generate_key_pair"
            | "crypto.hash_password"
    )
}

/// Is this builtin refused inside a `meta fn`? Expansion must be a pure
/// function of the source (macro Law 4), so the answer is: anything
/// capability-gated, anything the recorder would have to capture, plus
/// the thread constructs (worker interleaving is scheduling
/// nondeterminism) and, for defense in depth, every function of the
/// effectful modules — a pure helper like `fs.join` is refused too,
/// because a module whose *purpose* is effects should be entirely absent
/// from expansion rather than "mostly absent, check the list".
pub fn expansion_blocked(full_name: &str) -> bool {
    const BLOCKED_MODULE_PREFIXES: &[&str] = &[
        "fs.", "http.", "db.", "proc.", "os.", "time.", "random.", "task.", "chan.",
    ];
    const BLOCKED_NAMES: &[&str] = &["par_map", "par_filter"];
    if BLOCKED_MODULE_PREFIXES
        .iter()
        .any(|p| full_name.starts_with(p))
        || BLOCKED_NAMES.contains(&full_name)
    {
        return true;
    }
    let e = classify(full_name);
    e.cap.is_some() || e.recorded
}

/// Pure helpers inside gated modules: never gated — they compute, they
/// don't touch the world.
fn fs_pure(name: &str) -> bool {
    matches!(name, "join" | "basename" | "dirname" | "ext")
}

fn fs_read_only(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "read_bytes"
            | "exists"
            | "is_dir"
            | "is_file"
            | "list_dir"
            | "file_info"
            | "file_size"
            | "walk"
            | "glob"
            | "abs_path"
    )
}

fn http_pure(name: &str) -> bool {
    matches!(name, "encode_query" | "decode_query" | "parse_url")
}

fn os_env_gated(name: &str) -> bool {
    matches!(
        name,
        "get_env"
            | "set_env"
            | "list_env"
            | "has_env"
            | "remove_env"
            | "hostname"
            | "username"
            | "home_dir"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The drift this module exists to prevent, pinned: every capability-
    /// gated call and every recorded call is refused at expansion time.
    #[test]
    fn cross_consumer_agreement() {
        let surface = [
            // fs / http / db / proc / os — module-blocked and classified
            "fs.read_file",
            "fs.read_bytes",
            "fs.write_file",
            "http.get",
            "db.open",
            "proc.run",
            "os.exec",
            "os.exit",
            "os.get_env",
            "os.chdir",
            // the drift cases the 0.68 unification fixed
            "dates.now",
            "dates.utc_now",
            "dates.today",
            "crypto.random_bytes",
            "crypto.random_hex",
            "crypto.generate_key_pair",
            "crypto.hash_password",
            "ods.read_csv_file",
            "ods.open_csv",
            "ods.read_jsonl_file",
            "ods.open_jsonl",
            "ods.read_frame",
            "ods.frame_info",
            "ods.write_csv",
            "ods.write_jsonl",
            "ods.write_frame",
        ];
        for name in surface {
            let e = classify(name);
            assert!(
                e.cap.is_some() || e.recorded,
                "{name} should be classified as effectful"
            );
            assert!(
                expansion_blocked(name),
                "{name} is effectful but allowed at expansion time"
            );
        }
    }

    #[test]
    fn pure_computation_is_unclassified_and_expandable() {
        for name in [
            "dates.add_days",
            "dates.year",
            "crypto.sha256",
            "ods.read_csv", // text parser: reaches nothing
            "ods.group_by",
            "str.trim",
            "map_get",
            "cell.get",
            "meta.parse",
        ] {
            let e = classify(name);
            assert_eq!(e, Effects::default(), "{name} should be pure");
            assert!(
                !expansion_blocked(name),
                "{name} is pure but refused at expansion time"
            );
        }
    }

    #[test]
    fn effectful_module_helpers_stay_blocked_in_meta_even_when_pure() {
        // Defense in depth: fs.join is pure (and ungated) but still
        // refused inside a meta fn, because the module is effectful.
        assert_eq!(classify("fs.join"), Effects::default());
        assert!(expansion_blocked("fs.join"));
    }

    #[test]
    fn thread_constructs_are_expansion_blocked() {
        for name in ["par_map", "par_filter", "task.join", "chan.send"] {
            assert!(expansion_blocked(name), "{name} must be blocked in meta");
        }
    }
}
