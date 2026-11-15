//! The session-state differential harness (roadmap lane W4).
//!
//! Three session-poisoning bugs in one week — the help cache, the bridge
//! interpreter's frozen landscape, meta.eval losing the caller's tier —
//! were all found by a user, not a test. Each was invisible to the
//! existing suites because it needed *accumulated session state*: a
//! function redefined after promotion, an eval inside an already-warm
//! session.
//!
//! This harness generates action sequences from a seeded PRNG and pins
//! two properties on the real `olang repl` binary:
//!
//! 1. **Tier differential**: the same sequence through the tiered REPL
//!    and through `--no-ovm` (the pure tree-walker, the semantic oracle)
//!    must print the same thing at every step. The REPL promotes on the
//!    first call, so every sequence exercises the promoted boundary —
//!    this is the property the bridge-landscape and meta.eval-tier bugs
//!    violated.
//! 2. **Prefix-replay determinism**: step k's output in the long session
//!    equals step k's output in a fresh session replaying steps 0..=k —
//!    accumulated state never changes what a step prints beyond what the
//!    steps themselves established.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run lines through `olang repl` (optionally `--no-ovm`), returning the
/// per-step output segments split on the injected markers.
fn run_session(steps: &[String], no_ovm: bool) -> Vec<String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    if no_ovm {
        cmd.arg("--no-ovm");
    }
    let mut child = cmd
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn repl");
    let mut input = String::new();
    for (k, step) in steps.iter().enumerate() {
        input.push_str(step);
        input.push('\n');
        input.push_str(&format!("println(\"@@S{}@@\")\n", k));
    }
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    let text = strip_ansi(&String::from_utf8_lossy(&out.stdout));

    // Segment k is everything between marker k-1 and marker k. Segment 0
    // additionally carries the banner, identical across runs of the same
    // binary, so it still compares clean.
    let mut segments = Vec::new();
    let mut rest = text.as_str();
    for k in 0..steps.len() {
        let marker = format!("@@S{}@@", k);
        match rest.find(&marker) {
            Some(pos) => {
                segments.push(rest[..pos].to_string());
                rest = &rest[pos + marker.len()..];
            }
            None => {
                // A missing marker means the step crashed the session —
                // surface everything left so the assertion shows it.
                segments.push(format!("<<SESSION DIED>>\n{rest}"));
                rest = "";
            }
        }
    }
    segments
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI ... final byte in @..~
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('@'..='~').contains(&f) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Deterministic LCG — the sequences must be identical on every run so a
/// failure is reproducible from its seed alone.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % bound
    }
}

/// A generated sequence: definitions always precede their uses, and every
/// action prints, so every step has observable output to differ on.
fn generate_sequence(seed: u64, len: usize) -> Vec<String> {
    let mut rng = Lcg(seed);
    let mut steps: Vec<String> = Vec::new();
    // Step 0 always defines the workhorse; later actions may redefine it —
    // redefinition-after-promotion is the bridge-landscape bug class.
    let mut version = rng.next(7) + 2;
    steps.push(format!("fn work(x) = x * {version} + 1\nprintln(work(6))"));

    for _ in 1..len {
        match rng.next(6) {
            0 => {
                // Redefine the promoted function, then call it: the tier
                // must see the new body, not the one it compiled.
                version = rng.next(7) + 2;
                steps.push(format!("fn work(x) = x * {version} + 1\nprintln(work(6))"));
            }
            1 => {
                // A hot loop over the current definition (OSR territory).
                let n = 40 + rng.next(60);
                steps.push(format!(
                    "let mut acc{n} = 0\nfor i in 1..{n} {{ acc{n} = acc{n} + work(i) }}\nprintln(acc{n})"
                ));
            }
            2 => {
                // meta.eval inside the warm session — the tier-inheritance
                // bug class.
                let a = rng.next(90) + 1;
                steps.push(format!("println(unwrap(meta.eval(\"{a} + work(2)\")))"));
            }
            3 => {
                // Collections and strings through builtins.
                let a = rng.next(9) + 1;
                steps.push(format!(
                    "println(map([{a}, {}, {}], (v) => work(v)))",
                    a + 1,
                    a + 2
                ));
            }
            4 => {
                // A struct with a method-ish helper, printed.
                let f = rng.next(50);
                steps.push(format!(
                    "let pt{f} = #{{ \"x\": {f}, \"y\": work({f} % 5) }}\nprintln(map_get(pt{f}, \"y\"))"
                ));
            }
            _ => {
                // String machinery.
                let n = rng.next(4) + 1;
                steps.push(format!(
                    "println(str.length(str.repeat(\"ab\", work({n}) % 7 + 1)))"
                ));
            }
        }
    }
    steps
}

fn assert_segments_equal(seed: u64, label: &str, a: &[String], b: &[String]) {
    assert_eq!(a.len(), b.len());
    for (k, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(
            x, y,
            "seed {seed}: step {k} diverged ({label});\n  continuous/tiered: {x:?}\n  other: {y:?}"
        );
    }
}

#[test]
fn tiered_sessions_match_the_interpreter_oracle() {
    for seed in [11, 42, 977] {
        let steps = generate_sequence(seed, 8);
        let tiered = run_session(&steps, false);
        let oracle = run_session(&steps, true);
        assert_segments_equal(seed, "tiered vs --no-ovm oracle", &tiered, &oracle);
    }
}

#[test]
fn every_step_replays_identically_in_a_fresh_session() {
    for seed in [7, 301] {
        let steps = generate_sequence(seed, 6);
        let continuous = run_session(&steps, false);
        for k in 0..steps.len() {
            let fresh = run_session(&steps[..=k], false);
            assert_eq!(
                continuous[k], fresh[k],
                "seed {seed}: step {k} printed differently on fresh replay of its prefix"
            );
        }
    }
}
