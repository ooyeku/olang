//! `otc bench` — the project benchmark harness.
//!
//! Benches are ordinary olang programs in the project's `bench/`
//! directory, run as subprocesses (a fresh VM and JIT per run, the
//! wall-clock a user actually experiences). What makes the harness
//! worth having over a stopwatch:
//!
//! * **Scaling curves.** A bench that declares sizes
//!   (`// bench: sizes = 1000, 4000, 16000`) runs once per size with
//!   the size as `os.args()[1]`, and the harness fits the growth and
//!   names it — `~O(n)`, `~O(n log n)`, `~O(n²)`. Every real
//!   performance bug this codebase has hunted announced itself as a
//!   curve before it was a number.
//! * **Verified answers.** A `CHECKSUM <value>` line in a bench's
//!   output must agree across repetitions; a timing whose answer
//!   wobbles is measuring something else and is refused, not reported.
//! * **Memory.** Peak RSS per run, from the child's rusage.
//! * **Baselines.** `--save`/`--against` compare medians per size with
//!   a noise floor of max(5%, 2×CV); `--fail-on-regress` turns any red
//!   row into exit code 1. Baselines record a machine fingerprint and
//!   comparisons across machines warn instead of pretending.
//! * **`--profile`** reruns the slowest bench under `olang profile`,
//!   so "what regressed" arrives with "and here is where it went".

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// One bench file's declaration, read from `// bench:` comment
/// directives in its first lines. All optional: a plain .ol file with
/// no directives is a single-size bench.
#[derive(Debug, Default)]
struct BenchSpec {
    /// Sizes to run, passed as argv[1]. Empty = one run, no argument.
    sizes: Vec<u64>,
    /// A program to run once before timing (dataset generation). It
    /// receives the size argument too.
    setup: Option<String>,
}

fn parse_spec(source: &str) -> BenchSpec {
    let mut spec = BenchSpec::default();
    for line in source.lines().take(20) {
        let Some(rest) = line.trim().strip_prefix("// bench:") else {
            continue;
        };
        let Some((key, value)) = rest.split_once('=') else {
            continue;
        };
        match key.trim() {
            "sizes" => {
                spec.sizes = value
                    .split(',')
                    .filter_map(|t| t.trim().replace('_', "").parse().ok())
                    .collect();
            }
            "setup" => spec.setup = Some(value.trim().to_string()),
            _ => {}
        }
    }
    spec
}

/// One (bench, size) measurement.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct Measurement {
    median_ms: f64,
    min_ms: f64,
    max_ms: f64,
    cv_pct: f64,
    peak_rss_mb: f64,
    checksum: Option<String>,
    /// True when the program reported its own TIME (startup excluded).
    #[serde(default)]
    self_timed: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Baseline {
    machine: String,
    /// bench name -> size (0 for unsized) -> measurement
    benches: BTreeMap<String, BTreeMap<u64, Measurement>>,
}

fn machine_fingerprint() -> String {
    let brand = Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    format!(
        "{} · {} cores · {}",
        brand,
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        std::env::consts::OS
    )
}

/// The olang binary: `$OLANG`, the sibling of this executable, or PATH.
fn olang_binary() -> PathBuf {
    if let Ok(p) = std::env::var("OLANG") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("olang");
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from("olang")
}

/// Run one program once; wall time, peak RSS, and stdout.
fn run_once(
    olang: &Path,
    file: &Path,
    size: Option<u64>,
    cwd: &Path,
) -> anyhow::Result<(f64, f64, String)> {
    let mut cmd = Command::new(olang);
    cmd.arg("run").arg(file).current_dir(cwd);
    if let Some(n) = size {
        cmd.arg(n.to_string());
    }
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let start = Instant::now();
    let child = cmd.spawn()?;
    let (status, rss_bytes, output) = wait_with_rusage(child)?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    if !status.success() {
        anyhow::bail!(
            "bench {} failed (exit {:?}):\n{}",
            file.display(),
            status.code(),
            output
        );
    }
    Ok((elapsed_ms, rss_bytes / (1024.0 * 1024.0), output))
}

/// Wait for a child collecting its rusage (unix); the portable fallback
/// reports zero RSS rather than failing the bench.
#[cfg(unix)]
fn wait_with_rusage(
    mut child: std::process::Child,
) -> anyhow::Result<(std::process::ExitStatus, f64, String)> {
    use std::io::Read;
    let mut out = String::new();
    if let Some(stdout) = child.stdout.take() {
        let mut reader = std::io::BufReader::new(stdout);
        reader.read_to_string(&mut out)?;
    }
    let mut err = String::new();
    if let Some(stderr) = child.stderr.take() {
        let mut reader = std::io::BufReader::new(stderr);
        reader.read_to_string(&mut err)?;
    }
    let pid = child.id() as i32;
    let mut status: libc::c_int = 0;
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
    if r < 0 {
        // The child was already reaped somewhere; fall back to the
        // std wait with no usage numbers.
        let s = child.wait()?;
        return Ok((s, 0.0, out + &err));
    }
    // ru_maxrss: bytes on macOS, kilobytes on Linux.
    #[cfg(target_os = "macos")]
    let rss = usage.ru_maxrss as f64;
    #[cfg(not(target_os = "macos"))]
    let rss = usage.ru_maxrss as f64 * 1024.0;
    use std::os::unix::process::ExitStatusExt;
    Ok((std::process::ExitStatus::from_raw(status), rss, out + &err))
}

#[cfg(not(unix))]
fn wait_with_rusage(
    mut child: std::process::Child,
) -> anyhow::Result<(std::process::ExitStatus, f64, String)> {
    let output = child.wait_with_output()?;
    Ok((
        output.status,
        0.0,
        String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr),
    ))
}

/// A bench may self-time (`TIME <ms>` in its output) to exclude
/// interpreter startup and setup from the measurement — the same
/// convention the DP4 pipeline benchmark uses per stage. When present,
/// the reported time is the program's own.
fn extract_time_ms(output: &str) -> Option<f64> {
    output
        .lines()
        .filter_map(|l| l.trim().strip_prefix("TIME "))
        .next_back()
        .and_then(|t| t.trim().parse().ok())
}

fn extract_checksum(output: &str) -> Option<String> {
    output
        .lines()
        .filter_map(|l| l.trim().strip_prefix("CHECKSUM "))
        .next_back()
        .map(str::to_string)
}

/// Median of a sorted-in-place sample.
fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = xs.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    }
}

fn coefficient_of_variation(xs: &[f64]) -> f64 {
    let n = xs.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let mean = xs.iter().sum::<f64>() / n;
    if mean == 0.0 {
        return 0.0;
    }
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    var.sqrt() / mean * 100.0
}

/// Measure one (bench, size): a discarded warmup, then `runs` timed
/// repetitions. The answer (CHECKSUM line, when present) must agree
/// across every repetition, or the measurement is refused.
fn measure(
    olang: &Path,
    file: &Path,
    size: Option<u64>,
    runs: usize,
    cwd: &Path,
) -> anyhow::Result<Measurement> {
    let (_, _, warm_out) = run_once(olang, file, size, cwd)?;
    let expected = extract_checksum(&warm_out);
    let mut times = Vec::with_capacity(runs);
    let mut peak_rss: f64 = 0.0;
    let mut self_timed = false;
    for _ in 0..runs {
        let (wall_ms, rss, out) = run_once(olang, file, size, cwd)?;
        let ms = match extract_time_ms(&out) {
            Some(inner) => {
                self_timed = true;
                inner
            }
            None => wall_ms,
        };
        let checksum = extract_checksum(&out);
        if checksum != expected {
            anyhow::bail!(
                "bench {} size {:?}: the answer changed between runs ({:?} vs {:?}) — \
                 an unstable benchmark measures something else. Seed its randomness \
                 or fix its state.",
                file.display(),
                size,
                expected,
                checksum
            );
        }
        times.push(ms);
        peak_rss = peak_rss.max(rss);
    }
    let cv = coefficient_of_variation(&times);
    let med = median(&mut times);
    Ok(Measurement {
        median_ms: med,
        min_ms: times.first().copied().unwrap_or(med),
        max_ms: times.last().copied().unwrap_or(med),
        cv_pct: cv,
        peak_rss_mb: peak_rss,
        checksum: expected,
        self_timed,
    })
}

/// Name the growth between consecutive (size, time) points: the mean
/// log-log slope, mapped to the class a reader would say out loud.
fn growth_verdict(points: &[(u64, f64)]) -> Option<(f64, String, bool)> {
    if points.len() < 2 {
        return None;
    }
    let mut exponents = Vec::new();
    for pair in points.windows(2) {
        let (s1, t1) = pair[0];
        let (s2, t2) = pair[1];
        if t1 <= 0.0 || t2 <= 0.0 || s2 <= s1 {
            continue;
        }
        // Two points both under a quarter millisecond are inside timer
        // noise; a slope through them names the noise.
        if t1 < 0.25 && t2 < 0.25 {
            continue;
        }
        exponents.push((t2 / t1).ln() / ((s2 as f64) / (s1 as f64)).ln());
    }
    if exponents.is_empty() {
        return None;
    }
    let e = exponents.iter().sum::<f64>() / exponents.len() as f64;
    let (label, bad) = match e {
        e if e < 0.3 => ("~O(1)", false),
        e if e < 0.8 => ("sublinear", false),
        e if e < 1.25 => ("~O(n)", false),
        e if e < 1.6 => ("~O(n log n)", false),
        e if e < 2.4 => ("~O(n²) — check for a copy-per-iteration", true),
        _ => ("worse than quadratic", true),
    };
    Some((e, label.to_string(), bad))
}

pub struct BenchArgs {
    pub filter: Vec<String>,
    pub runs: usize,
    pub sizes: Option<String>,
    pub save: Option<String>,
    pub against: Option<String>,
    pub fail_on_regress: bool,
    pub profile: bool,
}

pub fn execute(args: BenchArgs) -> anyhow::Result<()> {
    let root = std::env::current_dir()
        .ok()
        .and_then(|cwd| olang::pkg::manifest::Manifest::find_root(&cwd))
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow::anyhow!("cannot determine the working directory"))?;
    let bench_dir = root.join("bench");
    if !bench_dir.is_dir() {
        anyhow::bail!(
            "no bench/ directory in {} — put .ol programs there (each is one bench; \
             `// bench: sizes = 1000, 4000` makes it a scaling curve)",
            root.display()
        );
    }

    let mut files: Vec<PathBuf> = std::fs::read_dir(&bench_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ol"))
        .filter(|p| {
            args.filter.is_empty()
                || args.filter.iter().any(|f| {
                    p.file_stem()
                        .is_some_and(|s| s.to_string_lossy().contains(f.as_str()))
                })
        })
        .collect();
    files.sort();
    if files.is_empty() {
        anyhow::bail!("no benches matched in {}", bench_dir.display());
    }

    let olang = olang_binary();
    let runs = args.runs.max(1);
    let machine = machine_fingerprint();
    let against: Option<Baseline> = match &args.against {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("cannot read baseline {}: {}", path, e))?;
            let base: Baseline = serde_json::from_str(&text)
                .map_err(|e| anyhow::anyhow!("baseline {} is not valid: {}", path, e))?;
            if base.machine != machine {
                println!(
                    "note: baseline is from a different machine\n  baseline: {}\n  here:     {}\n  deltas below compare across hardware — read them accordingly",
                    base.machine, machine
                );
            }
            Some(base)
        }
        None => None,
    };

    // Process startup is a constant every wall-clock point carries; the
    // growth fit subtracts it, or the smallest sizes read as "sublinear"
    // no matter what the algorithm does. Self-timed benches skip this.
    let startup_ms = {
        let empty = std::env::temp_dir().join(format!("otc_bench_empty_{}.ol", std::process::id()));
        std::fs::write(&empty, "let startup_probe = 0\nlet _ = startup_probe\n")?;
        let mut samples = Vec::new();
        for _ in 0..3 {
            let (ms, _, _) = run_once(&olang, &empty, None, &root)
                .map_err(|e| anyhow::anyhow!("startup probe failed: {}", e))?;
            samples.push(ms);
        }
        let _ = std::fs::remove_file(&empty);
        median(&mut samples)
    };
    println!(
        "otc bench · {} · {} runs per point · startup {:.1} ms (subtracted from growth fits)",
        machine, runs, startup_ms
    );
    let mut out_baseline = Baseline {
        machine,
        benches: BTreeMap::new(),
    };
    let mut regressed = false;
    let mut slowest: Option<(PathBuf, Option<u64>, f64)> = None;

    for file in &files {
        let name = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let source = std::fs::read_to_string(file)?;
        let mut spec = parse_spec(&source);
        if let Some(over) = &args.sizes {
            spec.sizes = over
                .split(',')
                .filter_map(|t| t.trim().replace('_', "").parse().ok())
                .collect();
        }

        println!("\n  {}", name);
        if let Some(setup) = &spec.setup {
            let setup_path = bench_dir.join(setup);
            let biggest = spec.sizes.iter().max().copied();
            let (ms, _, _) = run_once(&olang, &setup_path, biggest, &root)?;
            println!("    setup {} ({:.0} ms)", setup, ms);
        }

        let sizes: Vec<Option<u64>> = if spec.sizes.is_empty() {
            vec![None]
        } else {
            spec.sizes.iter().map(|s| Some(*s)).collect()
        };

        let mut points: Vec<(u64, f64)> = Vec::new();
        for size in &sizes {
            let m = measure(&olang, file, *size, runs, &root)?;
            let size_key = size.unwrap_or(0);
            let size_label = size
                .map(|s| format!("n={}", s))
                .unwrap_or_else(|| "-".to_string());
            let delta = against
                .as_ref()
                .and_then(|b| b.benches.get(&name))
                .and_then(|per| per.get(&size_key))
                .map(|prev| {
                    // Two points both under the timer's resolution have no
                    // meaningful ratio.
                    // Self-timed benches report integer milliseconds, so
                    // two points inside a couple of ms are quantization,
                    // not signal.
                    if prev.median_ms < 2.0 && m.median_ms < 2.0 {
                        return "  ~".to_string();
                    }
                    let change = (m.median_ms - prev.median_ms) / prev.median_ms.max(1.0) * 100.0;
                    let floor = (2.0 * m.cv_pct).max(5.0);
                    if change.abs() < floor {
                        "  ~".to_string()
                    } else if change > 0.0 {
                        regressed = true;
                        format!("  +{:.0}% SLOWER", change)
                    } else {
                        format!("  {:.0}% faster", -change)
                    }
                })
                .unwrap_or_default();
            let check = m
                .checksum
                .as_deref()
                .map(|c| format!("  checksum {}", c))
                .unwrap_or_default();
            println!(
                "    {:<10} {:>9.1} ms  (cv {:>4.1}%, rss {:>5.0} MB){}{}",
                size_label, m.median_ms, m.cv_pct, m.peak_rss_mb, delta, check
            );
            if let Some(s) = size {
                let fitted = if m.self_timed {
                    m.median_ms
                } else {
                    (m.median_ms - startup_ms).max(0.05)
                };
                points.push((*s, fitted));
            }
            if slowest
                .as_ref()
                .map(|(_, _, t)| m.median_ms > *t)
                .unwrap_or(true)
            {
                slowest = Some((file.clone(), *size, m.median_ms));
            }
            out_baseline
                .benches
                .entry(name.clone())
                .or_default()
                .insert(size_key, m);
        }

        if let Some((e, label, bad)) = growth_verdict(&points) {
            let line = format!("    growth: {} (exponent {:.2})", label, e);
            if bad {
                println!("    ⚠ {}", line.trim_start());
            } else {
                println!("{}", line);
            }
        }
    }

    if let Some(path) = &args.save {
        std::fs::write(path, serde_json::to_string_pretty(&out_baseline)?)?;
        println!("\nsaved baseline to {}", path);
    }

    if args.profile
        && let Some((file, size, ms)) = slowest
    {
        println!(
            "\nprofiling the slowest point: {} {} ({:.0} ms)",
            file.display(),
            size.map(|s| format!("n={}", s)).unwrap_or_default(),
            ms
        );
        let mut cmd = Command::new(&olang);
        cmd.arg("profile").arg(&file).current_dir(&root);
        if let Some(n) = size {
            cmd.arg(n.to_string());
        }
        let status = cmd.status()?;
        if !status.success() {
            println!("(profile run exited with {:?})", status.code());
        }
    }

    if regressed && args.fail_on_regress {
        anyhow::bail!("regression against the baseline");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_directives_parse_and_default() {
        let spec = parse_spec(
            "// a comment first\n// bench: sizes = 1_000, 4000 , 16000\n// bench: setup = gen.ol\nlet x = 1\n",
        );
        assert_eq!(spec.sizes, vec![1000, 4000, 16000]);
        assert_eq!(spec.setup.as_deref(), Some("gen.ol"));
        let none = parse_spec("let x = 1\n");
        assert!(none.sizes.is_empty());
        assert!(none.setup.is_none());
    }

    #[test]
    fn output_conventions_extract() {
        let out = "noise\nTIME 12.5\nCHECKSUM abc\nTIME 14\nCHECKSUM def\n";
        assert_eq!(extract_time_ms(out), Some(14.0));
        assert_eq!(extract_checksum(out), Some("def".to_string()));
        assert_eq!(extract_time_ms("plain"), None);
    }

    #[test]
    fn growth_names_the_classes() {
        // Perfect linear: t doubles when n doubles.
        let (e, label, bad) = growth_verdict(&[(1000, 10.0), (2000, 20.0), (4000, 40.0)]).unwrap();
        assert!((e - 1.0).abs() < 0.05, "{e}");
        assert_eq!(label, "~O(n)");
        assert!(!bad);
        // Quadratic: t quadruples.
        let (e, label, bad) = growth_verdict(&[(1000, 10.0), (2000, 40.0), (4000, 160.0)]).unwrap();
        assert!((e - 2.0).abs() < 0.05, "{e}");
        assert!(label.contains("O(n²)"), "{label}");
        assert!(bad, "quadratic must warn");
        // Constant.
        let (_, label, bad) = growth_verdict(&[(1000, 5.0), (4000, 5.0)]).unwrap();
        assert_eq!(label, "~O(1)");
        assert!(!bad);
        // One point: no verdict.
        assert!(growth_verdict(&[(1000, 5.0)]).is_none());
        // All points inside timer noise: no verdict either.
        assert!(growth_verdict(&[(1000, 0.1), (4000, 0.2)]).is_none());
    }

    #[test]
    fn median_and_cv_behave() {
        let mut xs = vec![3.0, 1.0, 2.0];
        assert_eq!(median(&mut xs), 2.0);
        let mut even = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(median(&mut even), 2.5);
        assert_eq!(coefficient_of_variation(&[5.0]), 0.0);
        let cv = coefficient_of_variation(&[10.0, 10.0, 10.0]);
        assert!(cv.abs() < 1e-9);
    }
}
