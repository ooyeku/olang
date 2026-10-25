//! The language's own olang-written modules are transparent to the gate.
//!
//! `gg`, `cli` and the other stdlib modules carried in the binary are
//! olang code, and the gate attributes an effect to the file of the code
//! making it. Theirs is `__embedded__/<name>`, under no granted folder,
//! so it fell to the process's grant: a module loaded with
//! `fs_roots`, or a dependency held to `fs = false`, wrote anywhere
//! through `gg.save` and read the environment through `cli.parse`. Such
//! code is judged now by its nearest caller that is not the language's,
//! as a native builtin is — on every tier, on a worker thread, and in a
//! `par_map`.
//!
//! Every embedded module that performs a gated effect is here: `gg.save`
//! (fs write) and `cli.parse` (env, through a flag's `env` fallback).
//! The others (`term`'s line read and flush, `cli.args`) touch nothing
//! the gate restricts.

use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

const TIERS: [&[&str]; 3] = [&["--no-ovm"], &["--ovm-tier=1"], &[]];

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_transp_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir).unwrap()
}

fn write(path: &Path, s: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, s).unwrap();
}

fn run(dir: &Path, flags: &[&str], file: &str) -> (String, String, bool) {
    let mut args: Vec<&str> = flags.to_vec();
    args.push(file);
    let out = Command::new(olang())
        .args(&args)
        .current_dir(dir)
        .env("OLANG_HOME", dir.join(".home"))
        .env("TRANSP_SECRET", "the host's")
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

/// The plugin: each of its calls reaches an effect only through the
/// language's own modules, directly, on a worker, and in a `par_map`.
const PLUGIN: &str = r#"
use gg
use cli
fn plot() = gg.point(gg.plot([#{ "x": 1, "y": 2 }], gg.aes(x: "x", y: "y")))
fn tag(r) = match r { Ok(v) => "RAN", Err(e) => if str.contains(e, "denied") => "DENIED" else => "ERR " + e }
share fn save(path) = {
    let mut out = []
    for i in range(0, 4) { out = out + [tag(attempt(() => gg.save(plot(), path)))] }
    out
}
share fn save_on_worker(path) = unwrap(task.join(spawn { tag(attempt(() => gg.save(plot(), path))) }))
share fn save_par(path) = par_map([1, 2, 3], (i) => tag(attempt(() => gg.save(plot(), path))))
share fn env() = {
    let spec = #{ "flags": [#{ "name": "who", "env": "TRANSP_SECRET" }] }
    let mut out = []
    for i in range(0, 4) { out = out + [tag(attempt(() => unwrap(cli.parse(spec, []))))] }
    out
}
"#;

#[test]
fn a_loaded_module_is_held_to_its_grant_through_the_stdlibs_olang_modules() {
    let dir = scratch("module");
    write(&dir.join("plug/p.ol"), PLUGIN);
    std::fs::create_dir_all(dir.join("plug/data")).unwrap();
    write(
        &dir.join("main.ol"),
        r#"
let m = unwrap(runtime.load_module("plug/p.ol", #{ "name": "p", "fs": true, "fs_roots": ["plug/data"] }))
println("inside: " + show(m.save("plug/data/in.svg")))
println("outside: " + show(m.save("escaped.svg")))
println("worker: " + m.save_on_worker("escaped-worker.svg"))
println("par_map: " + show(m.save_par("escaped-par.svg")))
println("env: " + show(m.env()))
"#,
    );
    let want = "inside: [\"RAN\", \"RAN\", \"RAN\", \"RAN\"]\n\
                outside: [\"DENIED\", \"DENIED\", \"DENIED\", \"DENIED\"]\n\
                worker: DENIED\n\
                par_map: [\"DENIED\", \"DENIED\", \"DENIED\"]\n\
                env: [\"DENIED\", \"DENIED\", \"DENIED\", \"DENIED\"]";
    for flags in TIERS {
        let (out, err, _) = run(&dir, flags, "main.ol");
        assert_eq!(out.trim_end(), want, "flags {flags:?}\nstderr: {err}");
        for f in ["escaped.svg", "escaped-worker.svg", "escaped-par.svg"] {
            assert!(
                !dir.join(f).exists(),
                "flags {flags:?}: {f} was written past the module's grant"
            );
        }
        assert!(dir.join("plug/data/in.svg").exists());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_host_keeps_its_own_grant_through_the_same_modules() {
    // The host calling gg.save itself is judged by the host's grant (here,
    // the whole disk): transparency hands the caller's grant through, it
    // does not shrink anyone's.
    let dir = scratch("host");
    write(
        &dir.join("main.ol"),
        r#"
use gg
let p = gg.point(gg.plot([#{ "x": 1, "y": 2 }], gg.aes(x: "x", y: "y")))
for i in range(0, 4) { let _s = gg.save(p, "host.svg") }
println(show(fs.exists("host.svg")))
"#,
    );
    for flags in TIERS {
        let _ = std::fs::remove_file(dir.join("host.svg"));
        let (out, err, ok) = run(&dir, flags, "main.ol");
        assert!(ok, "flags {flags:?}: {err}");
        assert_eq!(out.trim_end(), "true", "flags {flags:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_attenuated_dependency_is_held_to_its_grant_through_gg_and_cli() {
    // A manifest makes the run's table fixed, so the bytecode tier
    // pre-grants calls it can prove; a stdlib function compiled under the
    // app's grant must not carry that verdict into the dependency's call.
    let dir = scratch("dep");
    write(
        &dir.join("lib/olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"1.0.0\"\n",
    );
    write(
        &dir.join("lib/index.ol"),
        r#"
use gg
use cli
fn tag(r) = match r { Ok(v) => "RAN", Err(e) => if str.contains(e, "denied") => "DENIED" else => "ERR " + e }
fn plot() = gg.point(gg.plot([#{ "x": 1, "y": 2 }], gg.aes(x: "x", y: "y")))
share fn save(path) = tag(attempt(() => gg.save(plot(), path)))
share fn env() = tag(attempt(() => unwrap(cli.parse(#{ "flags": [#{ "name": "who", "env": "TRANSP_SECRET" }] }, []))))
"#,
    );
    write(
        &dir.join("app/olang.toml"),
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n\n\
         [dependencies]\nlib = { path = \"../lib\" }\n\n\
         [capabilities]\nfs = true\nenv = true\n\n\
         [capabilities.dependencies.lib]\nfs = false\nenv = false\n",
    );
    write(
        &dir.join("app/main.ol"),
        r#"
use gg
use lib
let p = gg.point(gg.plot([#{ "x": 1, "y": 2 }], gg.aes(x: "x", y: "y")))
let mut mine = []
let mut theirs = []
for i in range(0, 4) {
    mine = mine + [match attempt(() => gg.save(p, "app.svg")) { Ok(v) => "RAN", Err(e) => "DENIED" }]
    theirs = theirs + [lib.save("lib.svg")]
}
println("app: " + show(mine))
println("lib: " + show(theirs))
println("lib env: " + lib.env())
println("lib worker: " + unwrap(task.join(spawn { lib.save("lib-worker.svg") })))
"#,
    );
    let want = "app: [\"RAN\", \"RAN\", \"RAN\", \"RAN\"]\n\
                lib: [\"DENIED\", \"DENIED\", \"DENIED\", \"DENIED\"]\n\
                lib env: DENIED\n\
                lib worker: DENIED";
    for flags in TIERS {
        let (out, err, _) = run(&dir.join("app"), flags, "main.ol");
        assert_eq!(out.trim_end(), want, "flags {flags:?}\nstderr: {err}");
        assert!(!dir.join("app/lib.svg").exists(), "flags {flags:?}");
        assert!(!dir.join("app/lib-worker.svg").exists(), "flags {flags:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
