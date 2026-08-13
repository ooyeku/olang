//! `olang bench` — reproducible benchmarks for .ol programs.
//!
//! Each file runs as its own subprocess (a fresh VM and JIT every run,
//! and the wall-clock a user actually experiences): one discarded
//! warmup, then N timed runs — 7 by default, 3 when a run exceeds two
//! seconds, where long runs are stable. Reported per file: median, min,
//! max, and the coefficient of variation, with a warning when runs
//! disagree on their output (a benchmark that prints unstable output is
//! measuring something else).
//!
//! `--save results.json` stores the medians as a baseline;
//! `--against results.json` compares the current run to one, marking a
//! row changed only when it moves more than max(5%, 2×CV) — beneath
//! that it's noise, not news. `--fail-on-regress` turns red rows into
//! exit code 1, which is what makes the tool a regression guard.

use colored::Colorize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const DEFAULT_RUNS: usize = 7;
const SLOW_RUNS: usize = 3;
const SLOW_THRESHOLD_S: f64 = 2.0;

struct Measurement {
    median_s: f64,
    min_s: f64,
    max_s: f64,
    cv_pct: f64,
    runs: usize,
    stable_output: bool,
}

pub fn run(args: &[String]) -> i32 {
    let mut runs = DEFAULT_RUNS;
    let mut save: Option<PathBuf> = None;
    let mut against: Option<PathBuf> = None;
    let mut fail_on_regress = false;
    let mut paths: Vec<PathBuf> = Vec::new();

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--runs" => match it.next().and_then(|v| v.parse::<usize>().ok()) {
                Some(n) if n >= 1 => runs = n,
                _ => {
                    eprintln!("bench: --runs expects a positive integer");
                    return 2;
                }
            },
            "--save" => match it.next() {
                Some(p) => save = Some(PathBuf::from(p)),
                None => {
                    eprintln!("bench: --save expects a path");
                    return 2;
                }
            },
            "--against" => match it.next() {
                Some(p) => against = Some(PathBuf::from(p)),
                None => {
                    eprintln!("bench: --against expects a path");
                    return 2;
                }
            },
            "--fail-on-regress" => fail_on_regress = true,
            other => paths.push(PathBuf::from(other)),
        }
    }
    if paths.is_empty() {
        paths.push(PathBuf::from("."));
    }

    let files = match discover(&paths) {
        Ok(f) => f,
        Err(msg) => {
            eprintln!("bench: {msg}");
            return 2;
        }
    };
    if files.is_empty() {
        eprintln!("bench: no .ol files found");
        return 2;
    }

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("bench: cannot locate the olang binary: {e}");
            return 2;
        }
    };

    let baseline: Option<BTreeMap<String, f64>> = match &against {
        Some(p) => match load_baseline(p) {
            Ok(b) => Some(b),
            Err(msg) => {
                eprintln!("bench: {msg}");
                return 2;
            }
        },
        None => None,
    };

    println!(
        "{}",
        format!(
            "olang bench — {} file(s), 1 warmup + up to {} runs each",
            files.len(),
            runs
        )
        .bold()
    );

    let mut results: BTreeMap<String, f64> = BTreeMap::new();
    let mut regressed = false;
    for file in &files {
        let name = display_name(file);
        let m = match measure(&exe, file, runs) {
            Ok(m) => m,
            Err(msg) => {
                eprintln!("  {} {}", name.red(), msg);
                return 1;
            }
        };
        results.insert(name.clone(), m.median_s);

        let mut line = format!(
            "  {:<32} {:>9} {}",
            name,
            format_secs(m.median_s),
            format!(
                "(min {}, max {}, cv {:.1}%, n={})",
                format_secs(m.min_s),
                format_secs(m.max_s),
                m.cv_pct,
                m.runs
            )
            .dimmed()
        );
        if let Some(base) = baseline.as_ref().and_then(|b| b.get(&name)) {
            let delta_pct = (m.median_s - base) / base * 100.0;
            // Only call it a change when it clears both the fixed floor
            // and this row's own run-to-run noise.
            let threshold = 5.0_f64.max(2.0 * m.cv_pct);
            let verdict = if delta_pct.abs() < threshold {
                format!("~  vs {}", format_secs(*base)).dimmed().to_string()
            } else if delta_pct < 0.0 {
                format!("{:.0}% faster than {}", -delta_pct, format_secs(*base))
                    .green()
                    .to_string()
            } else {
                regressed = true;
                format!("{delta_pct:.0}% slower than {}", format_secs(*base))
                    .red()
                    .bold()
                    .to_string()
            };
            line.push_str(&format!("  {verdict}"));
        }
        if !m.stable_output {
            line.push_str(&format!("  {}", "output varies between runs!".yellow()));
        }
        println!("{line}");
    }

    if let Some(p) = save {
        match save_baseline(&p, &results) {
            Ok(()) => println!("{}", format!("baseline saved to {}", p.display()).dimmed()),
            Err(msg) => {
                eprintln!("bench: {msg}");
                return 2;
            }
        }
    }

    if regressed && fail_on_regress { 1 } else { 0 }
}

/// Files as given; directories contribute their immediate .ol children
/// (sorted). Not recursive — a benchmark suite is a flat, deliberate set.
fn discover(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut children: Vec<PathBuf> = std::fs::read_dir(p)
                .map_err(|e| format!("cannot read {}: {e}", p.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|c| c.extension().is_some_and(|x| x == "ol"))
                .collect();
            children.sort();
            files.extend(children);
        } else if p.is_file() {
            files.push(p.clone());
        } else {
            return Err(format!("no such file or directory: {}", p.display()));
        }
    }
    Ok(files)
}

fn display_name(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn measure(exe: &Path, file: &Path, max_runs: usize) -> Result<Measurement, String> {
    let once = |_: usize| -> Result<(f64, Vec<u8>), String> {
        let t0 = Instant::now();
        let out = Command::new(exe)
            .arg(file)
            .output()
            .map_err(|e| format!("failed to run: {e}"))?;
        let dt = t0.elapsed().as_secs_f64();
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!(
                "exited with {}: {}",
                out.status,
                err.lines().next().unwrap_or("").trim()
            ));
        }
        Ok((dt, out.stdout))
    };

    // Warmup run, discarded — it also decides the run count.
    let (warm_dt, first_out) = once(0)?;
    let runs = if warm_dt >= SLOW_THRESHOLD_S {
        SLOW_RUNS.min(max_runs)
    } else {
        max_runs
    };

    let mut times = Vec::with_capacity(runs);
    let mut stable_output = true;
    for i in 0..runs {
        let (dt, out) = once(i + 1)?;
        if out != first_out {
            stable_output = false;
        }
        times.push(dt);
    }
    times.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    let median = if times.len() % 2 == 1 {
        times[times.len() / 2]
    } else {
        (times[times.len() / 2 - 1] + times[times.len() / 2]) / 2.0
    };
    let cv_pct = if times.len() > 1 && median > 0.0 {
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let var = times.iter().map(|t| (t - mean).powi(2)).sum::<f64>() / (times.len() - 1) as f64;
        var.sqrt() / median * 100.0
    } else {
        0.0
    };
    Ok(Measurement {
        median_s: median,
        min_s: times[0],
        max_s: times[times.len() - 1],
        cv_pct,
        runs,
        stable_output,
    })
}

fn format_secs(s: f64) -> String {
    if s < 0.1 {
        format!("{:.1}ms", s * 1000.0)
    } else {
        format!("{s:.3}s")
    }
}

fn save_baseline(path: &Path, results: &BTreeMap<String, f64>) -> Result<(), String> {
    let mut doc = serde_json::Map::new();
    doc.insert(
        "olang_version".to_string(),
        serde_json::Value::String(env!("CARGO_PKG_VERSION").to_string()),
    );
    let mut benches = serde_json::Map::new();
    for (name, median) in results {
        benches.insert(
            name.clone(),
            serde_json::json!({ "median_s": (median * 10_000.0).round() / 10_000.0 }),
        );
    }
    doc.insert("benchmarks".to_string(), serde_json::Value::Object(benches));
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(doc))
        .map_err(|e| format!("cannot encode baseline: {e}"))?;
    std::fs::write(path, text + "\n").map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn load_baseline(path: &Path) -> Result<BTreeMap<String, f64>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?;
    let benches = doc
        .get("benchmarks")
        .and_then(|b| b.as_object())
        .ok_or_else(|| format!("{} has no \"benchmarks\" object", path.display()))?;
    let mut out = BTreeMap::new();
    for (name, entry) in benches {
        if let Some(m) = entry.get("median_s").and_then(|v| v.as_f64()) {
            out.insert(name.clone(), m);
        }
    }
    if out.is_empty() {
        return Err(format!("{} contains no benchmark entries", path.display()));
    }
    Ok(out)
}
