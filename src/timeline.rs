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

/// The current trace format version. v2 adds `Event.args` — a fingerprint
/// of each recorded call's arguments, so replay detects an
/// argument-level divergence instead of silently serving one call's result
/// to another call of the same op.
pub const TRACE_FORMAT: u32 = 2;

/// One recorded nondeterministic result, in program call order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Sequence number: the Nth recorded call in the run.
    pub seq: u64,
    /// The fully-qualified builtin that produced it (e.g. "time.now_ms").
    pub op: String,
    /// A deterministic fingerprint of the call's arguments (see
    /// [`Timeline::fingerprint`]). Empty on v1 traces, where it is not
    /// compared — so old traces still replay, just without argument checks.
    #[serde(default)]
    pub args: String,
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
    /// The program called the recorded op, but with different arguments —
    /// so the recorded result belongs to a different call. Without this
    /// check the wrong value would be served silently.
    ArgMismatch { seq: u64, op: String },
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
            Divergence::ArgMismatch { seq, op } => write!(
                f,
                "replay diverged at event {}: '{}' was called with different arguments than the trace recorded — the recorded result belongs to a different call, so replaying it would be wrong",
                seq, op
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
        // The classification lives in `crate::effects` — one table shared
        // with the capability gate and macro-expansion purity, so the three
        // policies cannot drift about what counts as nondeterministic.
        crate::effects::classify(op).recorded
    }

    /// A deterministic fingerprint of a recorded call's arguments, so replay
    /// can tell two calls of the same op apart. Canonical: map and struct
    /// fields are emitted in sorted-key order (a `HashMap`'s native order is
    /// process-randomized and would make the fingerprint unstable), then the
    /// whole rendering is hashed for a compact, collision-resistant tag.
    pub fn fingerprint(args: &[Value]) -> String {
        use sha2::{Digest, Sha256};
        let mut rendered = String::new();
        for a in args {
            canon(a, &mut rendered);
            rendered.push('\u{1f}'); // unit separator between arguments
        }
        let mut h = Sha256::new();
        h.update(rendered.as_bytes());
        format!("{:x}", h.finalize())[..16].to_string()
    }

    /// Record mode: after a real call, log its arguments' fingerprint and its
    /// result, in call order. A result that does not serialize round-trip
    /// (a non-finite float — `NaN`/`Infinity` are not valid JSON) is *not*
    /// recorded: replay then diverges cleanly at that call rather than
    /// serving a value that was silently corrupted through the trace.
    pub fn record_result(&mut self, op: &str, args_fp: &str, result: &Value) {
        if !round_trips(result) {
            eprintln!(
                "olang --record: '{}' returned a value that does not serialize (e.g. NaN/Infinity); it was not recorded, so replay will report a divergence here rather than a wrong value",
                op
            );
            return;
        }
        let seq = self.cursor as u64;
        self.cursor += 1;
        self.events.push(Event {
            seq,
            op: op.to_string(),
            args: args_fp.to_string(),
            result: result.clone(),
        });
    }

    /// Replay mode: return the recorded result for the next call, or a
    /// divergence if the program has stepped off the recorded path — a
    /// different op, or the same op with different arguments.
    pub fn replay_next(&mut self, op: &str, args_fp: &str) -> Result<Value, Divergence> {
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
        // Argument check, skipped on v1 traces (empty fingerprint) for
        // backward compatibility.
        if !event.args.is_empty() && event.args != args_fp {
            return Err(Divergence::ArgMismatch {
                seq,
                op: op.to_string(),
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

/// Whether a value serializes and deserializes back to an equal value.
/// False for a non-finite float (serde_json rejects `NaN`/`Infinity`), so a
/// recorded result that would be corrupted by the trace is caught.
fn round_trips(v: &Value) -> bool {
    match serde_json::to_vec(v) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .map(|back| &back == v)
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Render a value deterministically for [`Timeline::fingerprint`]: maps and
/// structs in sorted-key order (so a randomized `HashMap` order can't make
/// the fingerprint unstable), lists and tuples in order, scalars via their
/// display form.
fn canon(v: &Value, out: &mut String) {
    match v {
        Value::Map(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for k in keys {
                out.push_str(k);
                out.push(':');
                canon(m.get(k).unwrap_or(&Value::Unit), out);
                out.push(',');
            }
            out.push('}');
        }
        Value::Struct { type_name, fields } => {
            out.push_str(type_name);
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort();
            out.push('{');
            for k in keys {
                out.push_str(k);
                out.push(':');
                canon(fields.get(k).unwrap_or(&Value::Unit), out);
                out.push(',');
            }
            out.push('}');
        }
        Value::List(items) => {
            out.push('[');
            for it in items.iter() {
                canon(it, out);
                out.push(',');
            }
            out.push(']');
        }
        Value::Tuple(items) => {
            out.push('(');
            for it in items.iter() {
                canon(it, out);
                out.push(',');
            }
            out.push(')');
        }
        other => out.push_str(&other.to_string()),
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
        assert!(Timeline::is_recorded("crypto.random_bytes"));
        // machine identity / process context — recorded so a trace is portable
        assert!(Timeline::is_recorded("os.arch"));
        assert!(Timeline::is_recorded("os.os_type"));
        assert!(Timeline::is_recorded("os.args"));
        assert!(Timeline::is_recorded("os.cwd"));
        assert!(Timeline::is_recorded("os.pid"));
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
        let no_args = Timeline::fingerprint(&[]);
        rec.record_result("random.random", &no_args, &Value::Float(0.4172));
        rec.record_result("time.now_ms", &no_args, &Value::Integer(1_723_800_000_000));
        let home_fp = Timeline::fingerprint(&[Value::String(Arc::new("HOME".into()))]);
        rec.record_result(
            "os.get_env",
            &home_fp,
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
            rep.replay_next("random.random", &no_args).unwrap(),
            Value::Float(0.4172)
        );
        assert_eq!(
            rep.replay_next("time.now_ms", &no_args).unwrap(),
            Value::Integer(1_723_800_000_000)
        );
        assert_eq!(
            rep.replay_next("os.get_env", &home_fp).unwrap(),
            Value::String(Arc::new("/home/ada".to_string()))
        );
    }

    #[test]
    fn replay_detects_divergence() {
        let event = |op: &str, args: &str, result: Value| Event {
            seq: 0,
            op: op.into(),
            args: args.into(),
            result,
        };
        let trace = |ev: Vec<Event>| Trace {
            format: TRACE_FORMAT,
            olang_version: "test".into(),
            program_path: "p.ol".into(),
            program_sha256: "sha".into(),
            source: "source".into(),
            events: ev,
        };
        let no_args = Timeline::fingerprint(&[]);

        // A different op at seq 0 → OpMismatch.
        let mut rep = Timeline::replay(trace(vec![event(
            "random.random",
            &no_args,
            Value::Float(0.5),
        )]));
        assert!(matches!(
            rep.replay_next("time.now_ms", &no_args),
            Err(Divergence::OpMismatch { .. })
        ));

        // Same op, different arguments → ArgMismatch (the silent-wrong-replay
        // bug this check closes).
        let path_a = Timeline::fingerprint(&[Value::String(Arc::new("a.txt".into()))]);
        let path_b = Timeline::fingerprint(&[Value::String(Arc::new("b.txt".into()))]);
        let mut rep_args = Timeline::replay(trace(vec![event(
            "fs.read_file",
            &path_a,
            Value::String(Arc::new("A".into())),
        )]));
        assert!(matches!(
            rep_args.replay_next("fs.read_file", &path_b),
            Err(Divergence::ArgMismatch { .. })
        ));

        // One event, consumed, then another call → RanOut.
        let mut rep2 = Timeline::replay(trace(vec![event(
            "random.random",
            &no_args,
            Value::Float(0.5),
        )]));
        assert!(rep2.replay_next("random.random", &no_args).is_ok());
        assert!(matches!(
            rep2.replay_next("random.random", &no_args),
            Err(Divergence::RanOut { .. })
        ));
    }

    #[test]
    fn non_round_tripping_results_are_not_recorded() {
        let mut rec = Timeline::record(
            std::path::PathBuf::from("/tmp/x.olt"),
            "p.ol".into(),
            "source".into(),
            "sha".into(),
        );
        let fp = Timeline::fingerprint(&[]);
        // A finite float records; NaN does not (it is not valid JSON), so the
        // event is dropped rather than corrupted to null.
        rec.record_result("random.random", &fp, &Value::Float(0.5));
        rec.record_result("random.random", &fp, &Value::Float(f64::NAN));
        assert_eq!(rec.total_events(), 1, "NaN result must not be recorded");
        assert!(round_trips(&Value::Float(0.5)));
        assert!(!round_trips(&Value::Float(f64::NAN)));
        assert!(!round_trips(&Value::Float(f64::INFINITY)));
    }

    #[test]
    fn fingerprint_is_deterministic_across_map_order() {
        use std::collections::HashMap;
        // Two maps with the same entries inserted in different orders must
        // fingerprint identically (sorted-key canonicalization).
        let mut a = HashMap::new();
        a.insert("x".to_string(), Value::Integer(1));
        a.insert("y".to_string(), Value::Integer(2));
        let mut b = HashMap::new();
        b.insert("y".to_string(), Value::Integer(2));
        b.insert("x".to_string(), Value::Integer(1));
        let fa = Timeline::fingerprint(&[Value::Map(Arc::new(a))]);
        let fb = Timeline::fingerprint(&[Value::Map(Arc::new(b))]);
        assert_eq!(fa, fb);
        // Different contents fingerprint differently.
        let mut c = HashMap::new();
        c.insert("x".to_string(), Value::Integer(9));
        let fc = Timeline::fingerprint(&[Value::Map(Arc::new(c))]);
        assert_ne!(fa, fc);
    }
}
