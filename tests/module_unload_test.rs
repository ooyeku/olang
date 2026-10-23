//! An unloaded module's code never gets the host's authority.
//!
//! `runtime.unload_module(p)` used to forget the module's grant, so code
//! from that folder still running afterwards — a closure the host kept,
//! the module's own function, a thread the module started, a `par_map`
//! over its closure — fell to the host's grant and read what the module
//! was refused while loaded. The unload leaves a tombstone now: every
//! capability is refused to that code, with "module <dir> was unloaded",
//! on every tier, and a later `load_module` of the folder replaces it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

const TIERS: [&[&str]; 3] = [&["--no-ovm"], &["--ovm-tier=1"], &[]];

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_unload_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("plug")).unwrap();
    dir
}

fn run(dir: &Path, flags: &[&str]) -> (String, String) {
    let mut args: Vec<&str> = flags.to_vec();
    args.push("main.ol");
    let out = Command::new(olang())
        .args(&args)
        .current_dir(dir)
        .env("OLANG_HOME", dir.join(".home"))
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

const PLUGIN: &str = r#"
share fn reader() = () => fs.read_file("secret.txt")
share fn direct() = fs.read_file("secret.txt")
share fn start() = spawn {
    let mut out = []
    for i in range(0, 8) {
        out = out + [match attempt(() => fs.read_file("secret.txt")) { Ok(v) => "RAN", Err(e) => "DENIED" }]
        time.sleep(40)
    }
    out
}
"#;

const MAIN: &str = r#"
let p = fs.join(unwrap(fs.abs_path(".")), "plug/p.ol")
fn tag(r) = match r { Ok(v) => "RAN", Err(e) => if str.contains(e, "was unloaded") => "UNLOADED" else => "DENIED" }
let m = unwrap(runtime.load_module(p, #{ "fs": false }))
let f = m.reader()
let t = m.start()
println("loaded: " + tag(attempt(() => f())) + " " + tag(attempt(() => m.direct())))
time.sleep(100)
println("unload: " + show(runtime.unload_module(p)))
println("closure: " + tag(attempt(() => f())))
println("function: " + tag(attempt(() => m.direct())))
println("par_map: " + show(par_map([1, 2, 3, 4], (i) => tag(attempt(() => f())))))
println("thread: " + show(contains(unwrap(task.join(t)), "RAN")))
println("grant: " + show(runtime.module_grant(p)))
println("again: " + show(runtime.unload_module(p)))
let m2 = unwrap(runtime.load_module(p, #{ "fs": "read" }))
println("reloaded: " + tag(attempt(() => m2.direct())) + " " + tag(attempt(() => f())))
"#;

#[test]
fn an_unloaded_modules_code_is_refused_on_every_tier() {
    let dir = scratch("tiers");
    std::fs::write(dir.join("plug/p.ol"), PLUGIN).unwrap();
    std::fs::write(dir.join("secret.txt"), "the host's").unwrap();
    std::fs::write(dir.join("main.ol"), MAIN).unwrap();
    let want = "loaded: DENIED DENIED\n\
                unload: true\n\
                closure: UNLOADED\n\
                function: UNLOADED\n\
                par_map: [\"UNLOADED\", \"UNLOADED\", \"UNLOADED\", \"UNLOADED\"]\n\
                thread: false\n\
                grant: ()\n\
                again: false\n\
                reloaded: RAN RAN";
    for flags in TIERS {
        let (out, err) = run(&dir, flags);
        assert_eq!(out.trim_end(), want, "flags {flags:?}\nstderr: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
