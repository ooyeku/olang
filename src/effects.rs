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
//! - **simulation** (`crate::sim`) asks *inside a simulated world, does
//!   the world answer this call, or must it be refused?* — see
//!   [`sim_handling`]. A call the table knows to touch the world and the
//!   simulator does not answer is refused, never run for real.
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
    /// so calls returning native handles (`ods.open_csv`'s reader) are
    /// gated but not recorded — a documented replay limitation; a Frame
    /// read whole (`ods.read_csv_file`) is recorded in its columnar bytes.
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
    if full_name.starts_with("proc.")
        || full_name.starts_with("pty.")
        || matches!(full_name, "os.exec" | "os.exit")
    {
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
        // seed is a deterministic state-set; a generator (`random.rng`)
        // is a value whose draws follow from its seed alone, so neither
        // it nor its methods (`random.Rng.*`) touch hidden state.
        return !(matches!(f, "seed" | "rng") || f.starts_with("Rng."));
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
            | "time.monotonic"
            | "dates.now"
            | "dates.utc_now"
            | "dates.today"
            | "dates.stamp"
            | "dates.stamp_ms"
            // environment and console input.
            | "os.get_env"
            | "os.env"
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
            | "os.stdin_is_tty"
            // filesystem reads: external, mutable state (a Frame read
            // whole is data too: recorded in its columnar bytes).
            | "ods.read_csv_file"
            | "ods.read_jsonl_file"
            | "ods.read_frame"
            | "ods.frame_info"
            | "fs.read_file"
            | "fs.exists"
            | "fs.real_path"
            | "fs.is_file"
            | "fs.is_dir"
            | "fs.list_dir"
            | "fs.walk"
            | "fs.glob"
            | "fs.scan"
            | "fs.search"
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
            | "crypto.ed25519_keypair"
            | "crypto.generate_key_pair"
            | "crypto.hash_password"
    )
}

/// What a simulated world (`sim`) does with a builtin called inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimHandling {
    /// Pure computation (or output): runs as it always does.
    Pure,
    /// The world answers it: virtual time, the world's random streams,
    /// the cooperative scheduler for `spawn`, `task` and `chan`, (S2)
    /// `http` over the world's network, with each node's own `os.args`
    /// and environment, and (S3) `fs` and `db` on each node's own
    /// in-memory disk, and its stdin, shutdown handlers and exit.
    Simulated,
    /// It would touch the real world, and the world does not simulate it
    /// yet: refused with an error inside a world, never run for real.
    Refused,
}

/// The fourth consumer: how a simulated world handles `full_name`.
///
/// The rule that keeps a world sealed: anything the other three columns
/// call effectful (capability-gated or recorded) is `Refused` unless it is
/// listed here as `Simulated`. A new effectful builtin is therefore
/// refused inside a world the moment it is classified, before anyone
/// teaches the simulator to answer it. On top of that, a few ungated
/// effects the gate does not cover are refused by name: the terminal,
/// windows, processes, `os` as a whole, files read through `asset` and
/// `testing.snapshot`, the runtime's module loading, and the crypto
/// calls that draw from the operating system's entropy.
pub fn sim_handling(full_name: &str) -> SimHandling {
    if sim_simulated(full_name) {
        return SimHandling::Simulated;
    }
    const REFUSED_PREFIXES: &[&str] = &[
        "fs.",
        "http.",
        "db.",
        "proc.",
        "pty.",
        "os.",
        "tty.",
        "gui.",
        "asset.",
        "runtime.",
        "testing.snapshot",
    ];
    const REFUSED_NAMES: &[&str] = &[
        // entropy from the operating system, not the world's seed
        "crypto.encrypt_aes",
        "crypto.encrypt_rsa",
        "crypto.generate_key_pair",
        "crypto.hash_password",
        // introspection of real threads and real elapsed time
        "task.list",
        "task.parked",
        // worlds do not nest (yet)
        "sim.world",
    ];
    // what a world allows of `runtime`: pure facts about the build, and
    // the browser runtime the binary carries (its own bytes, the same in
    // every run)
    if matches!(
        full_name,
        "runtime.features" | "runtime.version" | "runtime.wasm"
    ) {
        return SimHandling::Pure;
    }
    let pure_helper = full_name.strip_prefix("fs.").is_some_and(fs_pure)
        || full_name.strip_prefix("http.").is_some_and(http_pure);
    if pure_helper {
        return SimHandling::Pure;
    }
    if REFUSED_PREFIXES.iter().any(|p| full_name.starts_with(p))
        || REFUSED_NAMES.contains(&full_name)
    {
        return SimHandling::Refused;
    }
    let e = classify(full_name);
    if e.cap.is_some() || e.recorded {
        return SimHandling::Refused;
    }
    SimHandling::Pure
}

/// The builtins a world answers itself (S1: the clock, randomness, the
/// scheduler; S2: `http` between nodes, a node's argv and environment;
/// S3: `fs` and `db` on the node's disk, its environment written, its
/// working directory, stdin, shutdown handlers and exit). Everything
/// else effectful is refused by `sim_handling`.
fn sim_simulated(full_name: &str) -> bool {
    if full_name.starts_with("random.") || full_name.starts_with("chan.") {
        return true;
    }
    // S3: every `db` call runs over the node's disk through the world's
    // VFS (sim/vfs.rs, sim/dbs.rs); a connection from outside is refused.
    if full_name.starts_with("db.") {
        return true;
    }
    // S3: the files a node reads and writes are on its own disk
    // (sim/files.rs). The tree walkers, watchers and the Trash are not.
    if let Some(f) = full_name.strip_prefix("fs.") {
        return sim_fs(f);
    }
    matches!(
        full_name,
        "time.now_ms"
            | "time.monotonic_ms"
            | "time.monotonic"
            | "time.sleep"
            | "dates.now"
            | "dates.utc_now"
            | "dates.today"
            | "dates.stamp"
            | "dates.stamp_ms"
            | "crypto.random_bytes"
            | "crypto.random_hex"
            | "task.join"
            | "task.join_timeout"
            | "task.watch"
            | "par_map"
            | "par_filter"
            // S2: the network between a world's nodes (sim/http.rs).
            // `response` and `response_with_headers` build values and run
            // as they always do; the world answers the rest.
            | "http.get"
            | "http.post"
            | "http.put"
            | "http.delete"
            | "http.request"
            | "http.serve"
            | "http.shutdown"
            | "http.defer"
            | "http.respond"
            | "http.hold"
            | "http.notify"
            | "http.notify_all"
            | "http.held"
            | "http.response"
            | "http.response_with_headers"
            // a node's own argv and environment, given when it is made
            | "os.args"
            | "os.get_env"
            | "os.has_env"
            // S3: its process (sim/life.rs)
            | "os.env"
            | "os.list_env"
            | "os.set_env"
            | "os.remove_env"
            | "os.cwd"
            | "os.read_line"
            | "os.on_shutdown"
            | "os.exit"
            | "os.pid"
            | "os.hostname"
            | "os.temp_dir"
            | "os.is_tty"
            | "os.stdin_is_tty"
            | "os.color_enabled"
            | "os.flush"
            | "os.path_separator"
            | "os.on_interrupt"
            | "os.interrupted"
            | "os.reset_interrupt"
    )
}

/// The `fs` calls a node's disk answers (S3).
fn sim_fs(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "write_file"
            | "read_bytes"
            | "write_bytes"
            | "append_file"
            | "exists"
            | "is_file"
            | "is_dir"
            | "list_dir"
            | "walk"
            | "glob"
            | "create_dir"
            | "create_dir_all"
            | "remove_dir"
            | "remove_dir_all"
            | "remove_file"
            | "copy_file"
            | "move_file"
            | "file_size"
            | "file_info"
            | "abs_path"
            | "real_path"
            | "sync"
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
///
/// Also refused, as ungated effects a file opened would otherwise make
/// (the language server and `olang check` expand every file they read):
/// `runtime.` (`runtime.load_module` runs another file and registers a
/// grant the real run inherits), `testing.` (a snapshot is a file
/// written), `asset.` (files read), and `print`/`println` — stdout is the
/// language server's protocol, so a meta fn printing a framed message
/// spoke to the editor as the server (`eprintln` stays for debugging a
/// macro: stderr is no channel).
pub fn expansion_blocked(full_name: &str) -> bool {
    const BLOCKED_MODULE_PREFIXES: &[&str] = &[
        "fs.", "http.", "db.", "proc.", "os.", "time.", "random.", "task.", "chan.", "tty.",
        "gui.", "pty.", "vt.", "runtime.", "testing.", "asset.",
    ];
    const BLOCKED_NAMES: &[&str] = &["par_map", "par_filter", "print", "println"];
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
            | "scan"
            | "search"
            | "watch"
            | "watch_next"
            | "watch_close"
            | "abs_path"
            | "real_path"
    )
}

fn http_pure(name: &str) -> bool {
    matches!(name, "encode_query" | "decode_query" | "parse_url")
}

fn os_env_gated(name: &str) -> bool {
    matches!(
        name,
        "get_env"
            | "env"
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
            "crypto.ed25519_keypair",
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

    /// S5: a world runs on the bytecode tier. Every builtin the tier
    /// bridges passes the one dispatch point where the world answers or
    /// refuses it; what the VM and its native code answer themselves must
    /// be pure, or a world on that tier would run it unseen.
    #[cfg(feature = "native")]
    #[test]
    fn the_bytecode_tiers_own_natives_are_pure() {
        let natives = crate::ovm::bytecode::BytecodeVm::natives_without_bridge();
        assert!(natives.len() > 30, "{natives:?}");
        assert!(
            natives.iter().any(|n| n.starts_with("cell.")),
            "{natives:?}"
        );
        for name in &natives {
            assert_eq!(
                sim_handling(name),
                SimHandling::Pure,
                "{name} runs natively on the bytecode tier but is not pure in a world"
            );
        }
    }

    /// The fourth consumer agrees with the other three: everything the
    /// gate restricts or the timeline records is either answered by a
    /// world or refused inside one — nothing effectful runs for real.
    #[test]
    fn sim_never_lets_a_classified_effect_through() {
        let mut surface: Vec<String> = Vec::new();
        for (module, value) in crate::stdlib::get_stdlib().iter() {
            if let crate::ast::Value::Struct { fields, .. } = value {
                for name in fields.keys() {
                    surface.push(format!("{module}.{name}"));
                }
            }
        }
        surface.extend(["par_map", "par_filter", "print", "println"].map(String::from));
        let mut checked = 0;
        for name in &surface {
            let e = classify(name);
            if e.cap.is_some() || e.recorded {
                checked += 1;
                assert_ne!(
                    sim_handling(name),
                    SimHandling::Pure,
                    "{name} is effectful but a world would run it for real"
                );
            }
        }
        assert!(checked > 50, "the stdlib surface was read ({checked})");
        for name in [
            "fs.scan",
            "fs.watch",
            "fs.trash",
            "proc.run",
            "os.exec",
            "os.username",
            "os.chdir",
            "os.stdin",
            "crypto.hash_password",
            "crypto.encrypt_aes",
            "tty.size",
            "ods.read_csv_file",
            "runtime.load_module",
            "sim.world",
        ] {
            assert_eq!(sim_handling(name), SimHandling::Refused, "{name}");
        }
        for name in [
            "time.now_ms",
            "time.sleep",
            "dates.now",
            "random.random",
            "random.seed",
            "chan.recv",
            "chan.send",
            "task.join",
            "crypto.random_hex",
            "http.get",
            "http.post",
            "http.request",
            "http.serve",
            "http.shutdown",
            "http.defer",
            "http.respond",
            "os.args",
            "os.get_env",
            // S3: the disk, SQLite on it, and a node's process
            "fs.read_file",
            "fs.write_file",
            "fs.append_file",
            "fs.sync",
            "fs.move_file",
            "db.open",
            "db.query",
            "db.transaction",
            "os.set_env",
            "os.read_line",
            "os.on_shutdown",
            "os.exit",
        ] {
            assert_eq!(sim_handling(name), SimHandling::Simulated, "{name}");
        }
        // Every member of `db` is answered by a world: none runs on the
        // real disk.
        for name in surface.iter().filter(|n| n.starts_with("db.")) {
            assert_eq!(sim_handling(name), SimHandling::Simulated, "{name}");
        }
        // Every member of `fs` a world does not answer is pure or refused,
        // never run.
        for name in surface.iter().filter(|n| n.starts_with("fs.")) {
            let f = &name[3..];
            let expected = if sim_fs(f) {
                SimHandling::Simulated
            } else if fs_pure(f) {
                SimHandling::Pure
            } else {
                SimHandling::Refused
            };
            assert_eq!(sim_handling(name), expected, "{name}");
        }
        // Every member of `http` is now answered by a world or pure: none
        // is refused, and none reaches a socket (sim/http.rs).
        for name in surface.iter().filter(|n| n.starts_with("http.")) {
            assert_ne!(sim_handling(name), SimHandling::Refused, "{name}");
        }
        // the rest of `os` stays refused: the machine, real processes
        for name in [
            "os.username",
            "os.home_dir",
            "os.exec",
            "os.stdin_lines",
            "os.arch",
        ] {
            assert_eq!(sim_handling(name), SimHandling::Refused, "{name}");
        }
        for name in [
            "str.trim",
            "fs.join",
            "http.parse_url",
            "map_get",
            "println",
            "sim.ms",
        ] {
            assert_eq!(sim_handling(name), SimHandling::Pure, "{name}");
        }
    }

    /// The clocks `dates` reads are recorded, every one of them: `stamp`
    /// and `stamp_ms` read the wall clock as `now` does.
    #[test]
    fn every_dates_clock_is_recorded() {
        for name in [
            "dates.now",
            "dates.utc_now",
            "dates.today",
            "dates.stamp",
            "dates.stamp_ms",
        ] {
            assert!(classify(name).recorded, "{name}");
        }
    }

    #[test]
    fn thread_constructs_are_expansion_blocked() {
        for name in ["par_map", "par_filter", "task.join", "chan.send"] {
            assert!(expansion_blocked(name), "{name} must be blocked in meta");
        }
    }
}
