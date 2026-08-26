//! The bridge interpreter's function landscape must track declarations.
//!
//! The VM's bridge interpreter (the exact-semantics fallback for
//! function values the VM declines) is seeded from the declaration
//! landscape — and it used to be seeded exactly once, at first use. A
//! bridge created during an early module load then answered for the
//! whole session: a later module's mutually recursive functions,
//! passed around as values, fell to the bridge and failed with
//! "Undefined variable" on names declared after the bridge was born.
//! The REPL surfaced it first (`use heap` before `:run` of the parser
//! example), but any early-bridging load could.
//!
//! The shape that reproduces it: `a` references `b` as a VALUE (an
//! argument, not a call) with `b` declared later — resolvable only
//! through the module's re-closed scope or a bridge whose seeding is
//! current.

use std::io::Write;
use std::process::{Command, Stdio};

fn repl(input: &str, cwd: &std::path::Path) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn repl");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_bridge_born_early_still_sees_later_declarations() {
    // The live reproduction: `use heap` makes the VM create its bridge
    // interpreter during the embedded module's load; the parser example
    // then runs, whose combinator grammar reaches later-declared
    // siblings (`expr`) as bare values through closures the VM declines
    // to the bridge. A bridge frozen at heap-time answers "Undefined
    // variable: expr". Minimized shapes are absorbed by the module
    // re-note refresh; only the real combinator nest exercises the
    // baked-constant path, so the test drives the example itself.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = repl(
        "use heap\n\
         heap.size(heap.new())\n\
         :run examples/parser/main.ol\n\
         quit\n",
        root,
    );
    assert!(
        !out.contains("Undefined variable"),
        "declarations after the bridge's birth must resolve:\n{out}"
    );
    assert!(
        out.contains("((7 - 2) * (3 + 1)) / 4 = 5"),
        "the evaluator ran through the bridged chain:\n{out}"
    );
}
