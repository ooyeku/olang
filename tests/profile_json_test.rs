//! The profiler and the bench as data over the real binary
//! (docs/tooling.md): `olang profile --format json` (frames with their
//! files and lines, every path with each frame's tier, the functions,
//! folded stacks, the process's series), its live snapshots
//! (`--live DIR`), an armed run that is attached to and detached from
//! (`--profile-live DIR`, `DIR/attach`), and `olang bench --format json`
//! with `--profile` and a baseline.

use serde_json::Value as J;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn project(name: &str, main: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "olang-profile-json-{}-{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("olang.toml"),
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\nauthors = []\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.ol"), main).unwrap();
    dir.canonicalize().unwrap()
}

const MAIN: &str = r#"fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)

fn parse(xs) = map(xs, (x) => to_string(x * 3))

fn churn(k) = fold(range(0, k), 0, (a, i) => a + len(parse([i, i + 1])))

let mut t = 0
for i in range(0, ROUNDS) { t = t + fib(18) + churn(1500) }
println(t)
"#;

fn olang(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run olang");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn read(p: &Path) -> J {
    serde_json::from_str(
        &std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())),
    )
    .unwrap()
}

fn frame_named<'a>(doc: &'a J, name: &str) -> Option<(usize, &'a J)> {
    doc["frames"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .find(|(_, f)| f["name"] == name)
}

#[test]
fn a_json_profile_names_frames_with_their_place_and_each_frame_s_tier() {
    let dir = project("json", &MAIN.replace("ROUNDS", "60"));
    let (code, _out, err) = olang(
        &dir,
        &[
            "profile",
            "main.ol",
            "--format",
            "json",
            "--out",
            "p.json",
            "--interval",
            "200",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let doc = read(&dir.join("p.json"));
    assert_eq!(doc["kind"], "profile");
    assert_eq!(doc["format"], 1);
    assert_eq!(doc["done"], true);
    assert_eq!(doc["exit"], 0);
    let samples = doc["samples"].as_u64().unwrap();
    assert!(samples > 10, "too few samples: {samples}");
    // tiers add up to the samples
    let tiers = &doc["tiers"];
    let sum: u64 = ["interpreter", "bytecode", "native", "builtin"]
        .iter()
        .map(|k| tiers[k].as_u64().unwrap())
        .sum();
    assert_eq!(sum, samples);
    // a frame is its function, its file, its line
    let (_, fib) = frame_named(&doc, "fib").expect("fib sampled");
    assert_eq!(fib["line"], 1);
    assert!(fib["file"].as_str().unwrap().ends_with("main.ol"));
    let (ci, churn) = frame_named(&doc, "churn").expect("churn sampled");
    assert_eq!(churn["line"], 5);
    // a stack names frames by index, with a tier each, and its count
    let stacks = doc["stacks"].as_array().unwrap();
    let mut counted = 0;
    for s in stacks {
        let f = s["f"].as_array().unwrap();
        let t = s["t"].as_array().unwrap();
        assert_eq!(f.len(), t.len());
        assert!(t.iter().all(|x| x.as_u64().unwrap() <= 3));
        counted += s["n"].as_u64().unwrap();
    }
    assert_eq!(counted, samples);
    // the functions: self and total, the total covering its callees
    let row = doc["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["frame"] == ci)
        .unwrap();
    assert!(row["total"].as_u64().unwrap() >= row["self"].as_u64().unwrap());
    // folded stacks: `a;b n`, their counts the samples
    let folded = doc["folded"].as_str().unwrap();
    let total: u64 = folded
        .lines()
        .map(|l| l.rsplit(' ').next().unwrap().parse::<u64>().unwrap())
        .sum();
    assert_eq!(total, samples);
    assert!(folded.contains("churn"));
    // the process's series: at least the last point, with memory
    let last = doc["series"].as_array().unwrap().last().unwrap().clone();
    assert!(last["heap"].as_u64().unwrap() > 0);
    assert!(last["rss_kb"].as_u64().unwrap() > 0);
    // the text report is unchanged by any of it
    let (code, out, _) = olang(&dir, &["profile", "main.ol"]);
    assert_eq!(code, 0);
    assert!(out.contains("TIME BY TIER") && out.contains("FUNCTIONS"));
}

#[test]
fn a_live_profile_writes_snapshots_while_it_runs() {
    let dir = project("live", &MAIN.replace("ROUNDS", "2500"));
    let live = dir.join("live");
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args([
            "profile",
            "main.ol",
            "--format",
            "json",
            "--out",
            "p.json",
            "--live",
            "live",
            "--live-every",
            "100",
        ])
        .current_dir(&dir)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    // a snapshot appears while the program is still running
    let mut mid: Option<J> = None;
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        if let Ok(rd) = std::fs::read_dir(&live) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "json")
                    && let Ok(t) = std::fs::read_to_string(&p)
                    && let Ok(d) = serde_json::from_str::<J>(&t)
                    && d["live"] == true
                    && d["samples"].as_u64().unwrap_or(0) > 0
                {
                    mid = Some(d);
                }
            }
        }
        if mid.is_some() {
            break;
        }
    }
    let status = child.wait().unwrap();
    assert!(status.success());
    let mid = mid.expect("a live snapshot while it ran");
    assert_eq!(mid["done"], false);
    assert!(!mid["series"].as_array().unwrap().is_empty());
    let last = read(&dir.join("p.json"));
    assert_eq!(last["done"], true);
    assert!(last["samples"].as_u64().unwrap() >= mid["samples"].as_u64().unwrap());
    // the last snapshot is the final document
    let snap = std::fs::read_dir(&live)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "json"))
        .unwrap();
    assert_eq!(read(&snap)["done"], true);
}

#[test]
fn an_armed_run_samples_only_while_attached() {
    let dir = project("armed", &MAIN.replace("ROUNDS", "5000"));
    let live = dir.join("live");
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["--profile-live", "live", "main.ol"])
        .env("OLANG_PROFILE_LIVE_EVERY", "100")
        .current_dir(&dir)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let snap = || -> Option<J> {
        let rd = std::fs::read_dir(&live).ok()?;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                return std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok());
            }
        }
        None
    };
    // vital signs before anyone attaches; no samples
    let mut before = None;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        if let Some(d) = snap()
            && !d["series"].as_array().unwrap().is_empty()
        {
            before = Some(d);
            break;
        }
    }
    let before = before.expect("an armed run writes its vital signs");
    assert_eq!(before["attached"], false);
    assert_eq!(before["samples"], 0);
    // attach: stacks arrive
    std::fs::write(live.join("attach"), "").unwrap();
    let mut attached = None;
    for _ in 0..150 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        if let Some(d) = snap()
            && d["attached"] == true
            && d["samples"].as_u64().unwrap_or(0) > 0
        {
            attached = Some(d);
            break;
        }
    }
    let attached = attached.expect("attached, it samples");
    assert!(frame_named(&attached, "fib").is_some() || frame_named(&attached, "churn").is_some());
    // detach: the profile so far is kept
    std::fs::remove_file(live.join("attach")).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success());
    let end = snap().unwrap();
    assert_eq!(end["done"], true);
    assert_eq!(end["attached"], false);
    assert_eq!(end["attaches"], 1);
    assert!(end["samples"].as_u64().unwrap() > 0);
}

#[test]
fn an_armed_run_attached_before_it_starts_samples_from_its_first_call() {
    // a run shorter than the snapshot period, attached before it starts
    // (an editor's "profile this file"): sampled from the start, not left
    // with nothing because the first look for `attach` came too late
    let dir = project("armed-short", &MAIN.replace("ROUNDS", "40"));
    let live = dir.join("live");
    std::fs::create_dir_all(&live).unwrap();
    std::fs::write(live.join("attach"), "").unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["--profile-live", "live", "main.ol"])
        .env("OLANG_PROFILE_LIVE_EVERY", "5000")
        .current_dir(&dir)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let snap = std::fs::read_dir(&live)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "json"))
        .unwrap();
    let end = read(&snap);
    assert_eq!(end["done"], true);
    assert_eq!(end["attaches"], 1);
    assert!(
        end["samples"].as_u64().unwrap() > 0,
        "samples: {}",
        end["samples"]
    );
    assert!(frame_named(&end, "fib").is_some() || frame_named(&end, "churn").is_some());
    // the series spans the run: a point at its start and one at its end
    let series = end["series"].as_array().unwrap();
    assert!(series.len() >= 2);
    assert!(series[0]["t_ms"].as_f64().unwrap() < series.last().unwrap()["t_ms"].as_f64().unwrap());
}

#[test]
fn a_bench_as_events_against_a_baseline_with_profiles() {
    let dir = project("bench", &MAIN.replace("ROUNDS", "20"));
    let (code, out, err) = olang(
        &dir,
        &[
            "bench",
            "main.ol",
            "--runs",
            "3",
            "--format",
            "json",
            "--save",
            "base.json",
            "--profile",
            "profiles",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let events: Vec<J> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l}: {e}")))
        .collect();
    assert_eq!(events.first().unwrap()["event"], "start");
    assert_eq!(events.last().unwrap()["event"], "finished");
    let r = events.iter().find(|e| e["event"] == "result").unwrap();
    assert_eq!(r["name"], "main");
    assert_eq!(r["verdict"], "new");
    assert_eq!(r["times_s"].as_array().unwrap().len(), 3);
    assert!(r["median_s"].as_f64().unwrap() > 0.0);
    let profile = PathBuf::from(r["profile"].as_str().unwrap());
    assert_eq!(read(&profile)["kind"], "profile");
    // against the baseline: a verdict with its band
    let (code, out, _) = olang(
        &dir,
        &[
            "bench",
            "main.ol",
            "--runs",
            "3",
            "--format",
            "json",
            "--against",
            "base.json",
        ],
    );
    assert_eq!(code, 0);
    let r = out
        .lines()
        .map(|l| serde_json::from_str::<J>(l).unwrap())
        .find(|e| e["event"] == "result")
        .unwrap();
    assert!(r["baseline_s"].as_f64().is_some());
    assert!(r["threshold_pct"].as_f64().unwrap() >= 5.0);
    assert!(["same", "faster", "slower"].contains(&r["verdict"].as_str().unwrap()));
    // a regression against a baseline much faster than this run
    std::fs::write(
        dir.join("fast.json"),
        r#"{"benchmarks":{"main":{"median_s":0.0001}}}"#,
    )
    .unwrap();
    let (code, out, _) = olang(
        &dir,
        &[
            "bench",
            "main.ol",
            "--runs",
            "3",
            "--format",
            "json",
            "--against",
            "fast.json",
            "--fail-on-regress",
        ],
    );
    assert_eq!(code, 1);
    let fin = out
        .lines()
        .map(|l| serde_json::from_str::<J>(l).unwrap())
        .last()
        .unwrap();
    assert_eq!(fin["regressed"], true);
}
