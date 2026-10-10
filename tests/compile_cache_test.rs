//! The compile cache (`src/compile_cache.rs`): compiled bytecode kept
//! between runs is used only when it is what a compile would make now.
//!
//! Each test runs real programs through the binary with a temporary
//! `OLANG_HOME` (never the user's own caches), and holds every run with
//! the cache — cold, warm, after a change that makes entries stale, over
//! a damaged cache, two processes at once — to the output of the same
//! program with the cache off. `OLANG_BOOT_TRACE`'s summary at exit says
//! how many compiles came from the cache, so a test can also tell that
//! the cache was used (or, after a change, was not used for what
//! changed).

use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

struct Run {
    stdout: String,
    stderr: String,
    ok: bool,
    /// compiles that ran (not from the cache)
    compiles: u64,
    /// compiles taken from the cache
    hits: u64,
    /// entries tried and found stale
    stale: u64,
}

/// A scratch directory: a project and its own `OLANG_HOME`.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!(
            "olang_compile_cache_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("project/lib")).unwrap();
        std::fs::create_dir_all(root.join("home")).unwrap();
        Scratch { root }
    }

    fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn cache_dir(&self) -> PathBuf {
        self.home().join("state/compiled")
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.project().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn run(&self, main: &str, cache: bool) -> Run {
        run_in(&self.project(), &self.home(), main, cache)
    }

    fn packs(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.cache_dir())
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "olcp"))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run_in(project: &Path, home: &Path, main: &str, cache: bool) -> Run {
    let out = Command::new(olang())
        .arg(main)
        .current_dir(project)
        .env("OLANG_HOME", home)
        .env("OLANG_BOOT_TRACE", "1")
        .env("OLANG_WARM", "0")
        .env("OLANG_COMPILE_CACHE", if cache { "1" } else { "0" })
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let num = |label: &str| -> u64 {
        stderr
            .lines()
            .find_map(|l| {
                let at = l.find(label)? + label.len();
                l[at..].trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
            })
            .unwrap_or(0)
    };
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        ok: out.status.success(),
        compiles: num("bytecode "),
        hits: bytecode_hits(&stderr),
        stale: num("stale "),
        stderr,
    }
}

/// The bytecode part of the boot summary: "(from the cache H in …,
/// stale S, kept K)".
fn bytecode_hits(stderr: &str) -> u64 {
    stderr
        .lines()
        .find_map(|l| {
            let at = l.find("compiles in ")?;
            let rest = &l[at..];
            let f = rest.find("from the cache ")? + "from the cache ".len();
            rest[f..].split(' ').next()?.parse().ok()
        })
        .unwrap_or(0)
}

/// A program that reaches the compiler's every kind of scope question:
/// module functions and values, a Boolean folded at compile time,
/// enums and unit variants, struct literals, lambdas with and without
/// captures, nested recursive functions, defaulted parameters, closures
/// handed to higher-order builtins, mutual recursion.
const LIB: &str = r#"
share let DEBUG = false
share let RATE = 3
share type Shape = enum { Circle(Int), Square(Int), Dot }
share type Point = struct { x: Int, y: Int }

share fn helper(x) = x + 1
share fn scale(xs, k) = map(xs, (x) => x * k + RATE)
share fn area(s) = match s {
    Circle(r) => r * r * 3,
    Square(a) => a * a,
    Dot => 0
}
share fn mode() = if DEBUG => "debug" else => "release"
share fn shift(p, d) = Point { x: p.x + d, y: p.y - d }
share fn padded(x, width = 8) = x * width
"#;

const MAIN: &str = r#"
use lib.util { helper, scale, Shape, Circle, Square, Dot, area, mode, Point, shift, padded }

fn is_even(n) = if n == 0 => true else => is_odd(n - 1)
fn is_odd(n) = if n == 0 => false else => is_even(n - 1)

fn total(xs) = {
    let mut t = 0
    for x in xs { t = t + helper(x) }
    t
}

fn counter(start) = {
    fn go(n, acc) = if n == 0 => acc else => go(n - 1, acc + start)
    go(5, 0)
}

fn adder(k) = (x) => x + k
fn apply_all(xs, k) = map(xs, adder(k))
fn shapes() = [Circle(2), Square(3), Dot] |> map(area)
fn walk(n) = {
    let mut p = Point { x: 0, y: 0 }
    for i in 0..n { p = shift(p, i) }
    p.x - p.y
}
fn kind(s) = match s { Dot => "dot", _ => "solid" }
fn wide(x) = padded(x)

let mut out = []
for round in 0..3 {
    out = out + [
        to_string(total([1, 2, 3, round])),
        to_string(scale([1, 2], round + 1)),
        to_string(is_even(10 + round)),
        to_string(counter(round)),
        to_string(apply_all([1, 2, 3], round)),
        to_string(shapes()),
        mode(),
        to_string(walk(4 + round)),
        kind(Dot) + kind(Circle(1)),
        to_string(wide(round)),
    ]
}
println(join(out, "\n"))
"#;

fn project(s: &Scratch) {
    s.write("lib/util.ol", LIB);
    s.write("main.ol", MAIN);
}

/// The same answers with the cache cold, warm, and warm again as with it
/// off; the warm runs take their compiles from it.
#[test]
fn cold_warm_and_off_agree() {
    let s = Scratch::new("agree");
    project(&s);
    let off = s.run("main.ol", false);
    assert!(off.ok, "the program runs: {}", off.stderr);
    assert!(off.stdout.contains("release"), "{}", off.stdout);
    let cold = s.run("main.ol", true);
    assert!(cold.ok, "{}", cold.stderr);
    assert_eq!(cold.stdout, off.stdout, "a cold cache changes nothing");
    assert!(!s.packs().is_empty(), "a cold run keeps what it compiled");
    for _ in 0..2 {
        let warm = s.run("main.ol", true);
        assert!(warm.ok, "{}", warm.stderr);
        assert_eq!(warm.stdout, off.stdout, "a warm cache changes nothing");
        assert!(warm.hits >= 10, "a warm run takes its compiles from the cache: {}", warm.stderr);
        assert!(
            warm.compiles < cold.compiles,
            "fewer compiles warm ({}) than cold ({})",
            warm.compiles,
            cold.compiles
        );
    }
    assert_eq!(off.hits, 0);
}

/// What changed is compiled again; the answers are the new source's.
fn changes_agree(name: &str, change: impl Fn(&Scratch)) {
    let s = Scratch::new(name);
    project(&s);
    let first = s.run("main.ol", true);
    assert!(first.ok, "{}", first.stderr);
    let warm = s.run("main.ol", true);
    assert!(bytecode_hits(&warm.stderr) > 0, "{}", warm.stderr);
    change(&s);
    let off = s.run("main.ol", false);
    let on = s.run("main.ol", true);
    assert_eq!(on.ok, off.ok, "{}\n---\n{}", on.stderr, off.stderr);
    assert_eq!(on.stdout, off.stdout, "after the change, the cache's answers are the new source's");
    if !off.ok {
        // the same error, the same words
        let err = |r: &Run| {
            r.stderr
                .lines()
                .filter(|l| !l.starts_with("olang boot:"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(err(&on), err(&off));
    }
    // and again, warm over the change
    let again = s.run("main.ol", true);
    assert_eq!(again.stdout, off.stdout);
}

#[test]
fn a_changed_function_body_is_compiled_again() {
    changes_agree("body", |s| {
        s.write("lib/util.ol", &LIB.replace("share fn helper(x) = x + 1", "share fn helper(x) = x * 10"));
    });
}

#[test]
fn a_changed_boolean_the_compile_folded_is_seen() {
    // `mode` folds `if DEBUG` at compile time: its entry must not survive
    // DEBUG becoming true
    changes_agree("fold", |s| {
        s.write("lib/util.ol", &LIB.replace("share let DEBUG = false", "share let DEBUG = true"));
    });
}

#[test]
fn a_changed_value_a_lambda_closes_over_is_seen() {
    changes_agree("captured", |s| {
        s.write("lib/util.ol", &LIB.replace("share let RATE = 3", "share let RATE = 7"));
    });
}

#[test]
fn a_callee_whose_arity_changed_is_called_as_it_is_now() {
    // `wide` calls `padded(x)`, whose default the compile spliced in as a
    // constant: a new default, then a parameter more, then one fewer
    changes_agree("arity_default", |s| {
        s.write(
            "lib/util.ol",
            &LIB.replace("share fn padded(x, width = 8) = x * width", "share fn padded(x, width = 2) = x * width"),
        );
    });
    changes_agree("arity_more", |s| {
        s.write(
            "lib/util.ol",
            &LIB.replace("share fn padded(x, width = 8) = x * width", "share fn padded(x, width = 8, add = 5) = x * width + add"),
        );
    });
    changes_agree("arity_fewer", |s| {
        s.write(
            "lib/util.ol",
            &LIB.replace("share fn helper(x) = x + 1", "share fn helper() = 1"),
        );
    });
}

#[test]
fn a_renamed_module_is_read_as_it_is_now() {
    changes_agree("rename", |s| {
        let lib = s.project().join("lib/util.ol");
        std::fs::rename(&lib, s.project().join("lib/tools.ol")).unwrap();
        s.write("main.ol", &MAIN.replace("use lib.util {", "use lib.tools {"));
    });
}

#[test]
fn a_name_made_ambiguous_is_resolved_as_it_is_now() {
    // a second module defines `helper` too, used by the main file
    changes_agree("ambiguous", |s| {
        s.write("lib/other.ol", "share fn helper(x) = x - 100\nshare fn other_total(xs) = map(xs, helper)\n");
        s.write(
            "main.ol",
            &format!(
                "use lib.other {{ other_total }}\n{}\nprintln(to_string(other_total([1, 2])))\n",
                MAIN
            ),
        );
    });
}

/// A pack from another build of olang is not read: its header names the
/// build. (A rebuilt olang also looks for packs under other names; this
/// holds even where one would find an old pack.)
#[test]
fn a_pack_from_another_build_is_ignored() {
    let s = Scratch::new("build");
    project(&s);
    let off = s.run("main.ol", false);
    s.run("main.ol", true);
    for p in s.packs() {
        let mut bytes = std::fs::read(&p).unwrap();
        // the stamp follows magic (4), format (2) and its length (2): the
        // version's first digit becomes another
        bytes[8] = if bytes[8] == b'9' { b'8' } else { b'9' };
        std::fs::write(&p, bytes).unwrap();
    }
    let run = s.run("main.ol", true);
    assert!(run.ok, "{}", run.stderr);
    assert_eq!(run.stdout, off.stdout);
    assert_eq!(bytecode_hits(&run.stderr), 0, "nothing from another build is used: {}", run.stderr);
    let again = s.run("main.ol", true);
    assert_eq!(again.stdout, off.stdout);
    assert!(bytecode_hits(&again.stderr) > 0, "the packs are written again: {}", again.stderr);
}

/// A truncated or garbled pack is ignored and written again, never
/// trusted.
#[test]
fn a_damaged_cache_is_ignored_and_rebuilt() {
    let s = Scratch::new("damage");
    project(&s);
    let off = s.run("main.ol", false);
    s.run("main.ol", true);
    let packs = s.packs();
    assert!(!packs.is_empty());
    for (i, p) in packs.iter().enumerate() {
        let bytes = std::fs::read(p).unwrap();
        let damaged = match i % 4 {
            0 => bytes[..bytes.len() / 2].to_vec(),
            1 => b"not a pack".to_vec(),
            2 => {
                // a byte flipped near the end: inside an entry's body
                let mut b = bytes.clone();
                let n = b.len();
                b[n - 3] ^= 0xff;
                b
            }
            _ => Vec::new(),
        };
        std::fs::write(p, damaged).unwrap();
    }
    let run = s.run("main.ol", true);
    assert!(run.ok, "{}", run.stderr);
    assert_eq!(run.stdout, off.stdout, "a damaged cache changes nothing");
    let again = s.run("main.ol", true);
    assert_eq!(again.stdout, off.stdout);
    assert!(bytecode_hits(&again.stderr) > 0, "the damaged packs were written again: {}", again.stderr);
    // garbage among the cache's files is left alone and ignored
    std::fs::write(s.cache_dir().join("stray.olcp.tmp.1.1"), b"half a pack").unwrap();
    let stray = s.run("main.ol", true);
    assert_eq!(stray.stdout, off.stdout);
}

/// Processes compiling the same program at once, from a cold cache:
/// every one answers right, and the cache they leave is whole.
#[test]
fn processes_compiling_at_once_agree() {
    let s = Scratch::new("concurrent");
    project(&s);
    let off = s.run("main.ol", false);
    for _ in 0..3 {
        let _ = std::fs::remove_dir_all(s.cache_dir());
        let children: Vec<_> = (0..4)
            .map(|_| {
                Command::new(olang())
                    .arg("main.ol")
                    .current_dir(s.project())
                    .env("OLANG_HOME", s.home())
                    .env("OLANG_BOOT_TRACE", "1")
                    .env("OLANG_WARM", "0")
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for c in children {
            let out = c.wait_with_output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            assert_eq!(String::from_utf8_lossy(&out.stdout), off.stdout);
        }
        let warm = s.run("main.ol", true);
        assert_eq!(warm.stdout, off.stdout);
        assert!(bytecode_hits(&warm.stderr) > 0, "{}", warm.stderr);
    }
}

/// `OLANG_COMPILE_CACHE=0` reads and writes nothing.
#[test]
fn turned_off_it_writes_nothing() {
    let s = Scratch::new("off");
    project(&s);
    let off = s.run("main.ol", false);
    assert!(off.ok);
    assert!(s.packs().is_empty(), "nothing written with the cache off");
    assert_eq!(off.stale, 0);
}

/// The cache directory is held under its size: an over-full directory
/// loses its oldest packs when a run next writes.
#[test]
fn an_over_full_cache_is_trimmed() {
    let s = Scratch::new("bound");
    project(&s);
    std::fs::create_dir_all(s.cache_dir()).unwrap();
    // 3 MB of old packs from "other programs"
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 86_400);
    for i in 0..6 {
        let p = s.cache_dir().join(format!("old{i:02}.olcp"));
        std::fs::write(&p, vec![0u8; 512 * 1024]).unwrap();
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(old).unwrap();
    }
    let out = Command::new(olang())
        .arg("main.ol")
        .current_dir(s.project())
        .env("OLANG_HOME", s.home())
        .env("OLANG_WARM", "0")
        .env("OLANG_COMPILE_CACHE_MB", "1")
        .output()
        .unwrap();
    assert!(out.status.success());
    let total: u64 = std::fs::read_dir(s.cache_dir())
        .unwrap()
        .flatten()
        .map(|e| e.metadata().unwrap().len())
        .sum();
    assert!(total <= 1024 * 1024, "held under 1 MB: {total}");
    assert!(
        s.packs().iter().any(|p| !p.file_name().unwrap().to_string_lossy().starts_with("old")),
        "this run's packs are kept"
    );
}
