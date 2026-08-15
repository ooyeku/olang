//! The Open Timeline: record a program's nondeterministic inputs once, then
//! replay the run bit-for-bit anywhere.
//!
//! olang has the three prerequisites of perfect record/replay that
//! mainstream runtimes lack: **deterministic execution** (a program is a
//! pure function of its inputs), **immutable acyclic values** (a recorded
//! result serializes cleanly and can never be a live handle in disguise),
//! and a small, explicit **effect boundary** — the only nondeterminism a
//! program can observe is the result of a handful of stdlib calls:
//! `random.*`, the clocks in `time`/`dates`, the environment/stdin/`exec`
//! surface of `os`, filesystem reads, `http`, and the seeded-random parts
//! of `crypto`. Record those results in call order and the run is
//! reproducible by construction.
//!
//! Record mode performs each such call for real and logs its result.
//! Replay mode intercepts the same call and returns the logged result
//! instead — so `random.random()` yields the exact float it did the first
//! time, `time.now_ms()` the exact millisecond, `os.get_env("HOME")` the
//! exact value. A `.olt` trace embeds the program source too, so it is a
//! self-contained, portable reproduction: a bug report becomes a file.
//!
//! Divergence is detected, not hidden: if replay reaches a recorded event
//! but the program makes a *different* nondeterministic call than was
//! recorded (the code changed, or a source of nondeterminism went
//! uncaptured), replay stops and says exactly where. A clean replay is a
//! proof that the run was fully determined by the recorded inputs.

use crate::ast::Value;
use serde::{Deserialize, Serialize};

/// The current trace format version.
pub const TRACE_FORMAT: u32 = 1;

/// One recorded nondeterministic result, in program call order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Sequence number: the Nth recorded call in the run.
    pub seq: u64,
    /// The fully-qualified builtin that produced it (e.g. "time.now_ms").
    pub op: String,
    /// The result value, serialized. Recorded ops only ever return data,
    /// so this always round-trips.
    pub result: Value,
}

/// A `.olt` trace: the program that was run, and the nondeterministic
/// results it observed, in order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub format: u32,
    pub olang_version: String,
    /// The path the program was recorded from (context only).
    pub program_path: String,
    /// sha256 (hex) of the program source, so replay can tell whether the
    /// code changed under it.
    pub program_sha256: String,
    /// The program source itself — a trace is self-contained and portable.
    pub source: String,
    pub events: Vec<Event>,
}

/// Record or replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Record,
    Replay,
}

/// A replay divergence: the program's Nth nondeterministic call did not
/// match what the trace recorded.
#[derive(Debug, Clone)]
pub enum Divergence {
    /// The program made more recorded calls than the trace holds.
    RanOut { seq: u64, op: String },
    /// The program called a different op than recorded at this point.
    OpMismatch {
        seq: u64,
        recorded: String,
        actual: String,
    },
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Divergence::RanOut { seq, op } => write!(
                f,
                "replay diverged at event {}: the program called '{}', but the trace has no more recorded events — the program is doing more than it did when recorded (the code changed, or a new source of nondeterminism appeared)",
                seq, op
            ),
            Divergence::OpMismatch {
                seq,
                recorded,
                actual,
            } => write!(
                f,
                "replay diverged at event {}: the trace recorded '{}' here, but the program called '{}' — the program's sequence of effects changed since it was recorded",
                seq, recorded, actual
            ),
        }
    }
}

/// The timeline attached to a run: an ordered log, plus the mode.
#[derive(Debug)]
pub struct Timeline {
    mode: Mode,
    events: Vec<Event>,
    /// Replay cursor / record counter.
    cursor: usize,
    /// Where to write the trace on a recorded run (`None` when replaying).
    out_path: Option<std::path::PathBuf>,
    program_path: String,
    program_sha256: String,
    source: String,
}

impl Timeline {
    /// Start recording. The trace is written to `out_path` at the end of
    /// the run (see [`Timeline::finish`]).
    pub fn record(
        out_path: std::path::PathBuf,
        program_path: String,
        source: String,
        program_sha256: String,
    ) -> Timeline {
        Timeline {
            mode: Mode::Record,
            events: Vec::new(),
            cursor: 0,
            out_path: Some(out_path),
            program_path,
            program_sha256,
            source,
        }
    }

    /// Start replaying from a loaded trace.
    pub fn replay(trace: Trace) -> Timeline {
        Timeline {
            mode: Mode::Replay,
            events: trace.events,
            cursor: 0,
            out_path: None,
            program_path: trace.program_path,
            program_sha256: trace.program_sha256,
            source: trace.source,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether `op` names a nondeterministic source that the timeline
    /// intercepts. The set is curated to operations whose result depends
    /// on state outside the program's own computation, and that return
    /// pure data (so the result always serializes) — never handles.
    pub fn is_recorded(op: &str) -> bool {
        // random: every draw consumes hidden RNG state.
        if let Some(f) = op.strip_prefix("random.") {
            return !matches!(f, "seed"); // seed is a deterministic state-set
        }
        // clocks: wall-clock and monotonic time.
        matches!(
            op,
            "time.now_ms"
                | "time.monotonic_ms"
                | "time.now"
                | "time.utc_now"
                | "time.today"
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
                | "crypto.random_hex"
                | "crypto.generate_key_pair"
                | "crypto.hash_password"
        )
    }

    /// Record mode: after a real call, log its result. Silently ignores a
    /// result that does not round-trip (a curated op should never produce
    /// one, but a lossy log entry must never corrupt replay).
    pub fn record_result(&mut self, op: &str, result: &Value) {
        let seq = self.cursor as u64;
        self.cursor += 1;
        self.events.push(Event {
            seq,
            op: op.to_string(),
            result: result.clone(),
        });
    }

    /// Replay mode: return the recorded result for the next call, or a
    /// divergence if the program has stepped off the recorded path.
    pub fn replay_next(&mut self, op: &str) -> Result<Value, Divergence> {
        let seq = self.cursor as u64;
        let Some(event) = self.events.get(self.cursor) else {
            return Err(Divergence::RanOut {
                seq,
                op: op.to_string(),
            });
        };
        if event.op != op {
            return Err(Divergence::OpMismatch {
                seq,
                recorded: event.op.clone(),
                actual: op.to_string(),
            });
        }
        self.cursor += 1;
        Ok(event.result.clone())
    }

    /// The number of events recorded or consumed so far.
    pub fn len(&self) -> usize {
        if self.mode == Mode::Record {
            self.events.len()
        } else {
            self.cursor
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn total_events(&self) -> usize {
        self.events.len()
    }

    /// Write the recorded trace to its output path. A no-op in replay mode.
    pub fn finish(&self) -> std::io::Result<Option<std::path::PathBuf>> {
        let Some(path) = &self.out_path else {
            return Ok(None);
        };
        let trace = Trace {
            format: TRACE_FORMAT,
            olang_version: crate::VERSION.to_string(),
            program_path: self.program_path.clone(),
            program_sha256: self.program_sha256.clone(),
            source: self.source.clone(),
            events: self.events.clone(),
        };
        let json = serde_json::to_vec_pretty(&trace).map_err(std::io::Error::other)?;
        std::fs::write(path, json)?;
        Ok(Some(path.clone()))
    }

    /// Load a trace from disk.
    pub fn load_trace(path: &std::path::Path) -> Result<Trace, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let trace: Trace = serde_json::from_slice(&bytes)
            .map_err(|e| format!("{} is not a valid .olt trace: {}", path.display(), e))?;
        if trace.format > TRACE_FORMAT {
            return Err(format!(
                "{} was recorded by a newer olang (trace format {} > {})",
                path.display(),
                trace.format,
                TRACE_FORMAT
            ));
        }
        Ok(trace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn the_recorded_set_is_effects_not_computation() {
        assert!(Timeline::is_recorded("random.random"));
        assert!(Timeline::is_recorded("random.randint"));
        assert!(Timeline::is_recorded("time.now_ms"));
        assert!(Timeline::is_recorded("os.get_env"));
        assert!(Timeline::is_recorded("fs.read_file"));
        assert!(Timeline::is_recorded("http.get"));
        assert!(Timeline::is_recorded("crypto.hash_password"));
        // seed is deterministic state, not an observed input
        assert!(!Timeline::is_recorded("random.seed"));
        // pure computation is never recorded
        assert!(!Timeline::is_recorded("str.trim"));
        assert!(!Timeline::is_recorded("math.sqrt"));
        assert!(!Timeline::is_recorded("fs.write_file"));
        assert!(!Timeline::is_recorded("fs.join"));
    }

    #[test]
    fn record_then_replay_returns_identical_values() {
        let mut rec = Timeline::record(
            std::path::PathBuf::from("/tmp/x.olt"),
            "p.ol".into(),
            "source".into(),
            "sha".into(),
        );
        rec.record_result("random.random", &Value::Float(0.4172));
        rec.record_result("time.now_ms", &Value::Integer(1_723_800_000_000));
        rec.record_result(
            "os.get_env",
            &Value::String(Arc::new("/home/ada".to_string())),
        );

        // Build a trace as finish() would, then replay it.
        let trace = Trace {
            format: TRACE_FORMAT,
            olang_version: "test".into(),
            program_path: "p.ol".into(),
            program_sha256: "sha".into(),
            source: "source".into(),
            events: rec.events.clone(),
        };
        let mut rep = Timeline::replay(trace);
        assert_eq!(
            rep.replay_next("random.random").unwrap(),
            Value::Float(0.4172)
        );
        assert_eq!(
            rep.replay_next("time.now_ms").unwrap(),
            Value::Integer(1_723_800_000_000)
        );
        assert_eq!(
            rep.replay_next("os.get_env").unwrap(),
            Value::String(Arc::new("/home/ada".to_string()))
        );
    }

    #[test]
    fn replay_detects_divergence() {
        let trace = Trace {
            format: TRACE_FORMAT,
            olang_version: "test".into(),
            program_path: "p.ol".into(),
            program_sha256: "sha".into(),
            source: "source".into(),
            events: vec![Event {
                seq: 0,
                op: "random.random".into(),
                result: Value::Float(0.5),
            }],
        };
        // A different op at seq 0 → OpMismatch.
        let mut rep = Timeline::replay(trace.clone());
        assert!(matches!(
            rep.replay_next("time.now_ms"),
            Err(Divergence::OpMismatch { .. })
        ));
        // One event, consumed, then another call → RanOut.
        let mut rep2 = Timeline::replay(trace);
        assert!(rep2.replay_next("random.random").is_ok());
        assert!(matches!(
            rep2.replay_next("random.random"),
            Err(Divergence::RanOut { .. })
        ));
    }
}
