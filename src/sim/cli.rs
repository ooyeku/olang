//! `olang sim FILE [--seed S] [--seeds N] [--jobs J] [--shrink] [--for 6h]
//! [--events]`, `olang sim FILE --replay PLAN`, `olang sim shrink PLAN`:
//! run a program as the main program of a simulated world, once with one
//! seed or once per seed for many, and say which seeds fail.
//!
//! - `--seed S` runs seed S with the program's output shown, then a
//!   summary; a failure prints its error and the events that led to it.
//!   Running the seed a failure was reported with reproduces it.
//! - `--seeds N` runs N seeds (from `--seed`, default 1), the program's
//!   output kept quiet, and prints each failing seed with what went wrong
//!   (the property and the virtual time it broke at) and the line that
//!   reproduces it. `--jobs J` runs them in J worker processes (each an
//!   `olang sim FILE --worker` fed seeds one at a time); every world is
//!   still one thread and one seed, so the results, and the digest the
//!   summary ends with, are the same for every J.
//! - `--shrink` minimises the first failing seed (`shrink.rs`): it says
//!   how many faults it took ("shrunk: 11 faults → 2"), lists them, and
//!   saves the minimal run as a plan file, `.sim/<seed>.simplan`, which
//!   `--replay` runs again bit for bit (its digest checked).
//!
//! **Two kinds of file.** A program (S1) runs as the main program of one
//! world. A file that builds worlds (S2: it calls `sim.world(...)`, adds
//! nodes, and runs them with `w.run`) runs as itself, outside any world,
//! with every world it makes taking the seed asked for; it fails when it
//! raises (an `assert` on a run's history, say) or when one of its runs
//! fails.

use super::plan::RunPlan;
use super::{Config, Event, Report};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// What the command was asked.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub seed: Option<u64>,
    pub seeds: Option<u64>,
    pub run_for: Option<u64>,
    pub events: bool,
    pub max_steps: Option<u64>,
    /// Worker processes for `--seeds` (and the shrinker's trials).
    pub jobs: Option<usize>,
    pub shrink: bool,
    /// Runs the shrinker may spend (default `shrink::DEFAULT_BUDGET`).
    pub shrink_budget: Option<usize>,
    /// A plan file to replay.
    pub replay: Option<PathBuf>,
    /// Serve seeds on stdin, results on stdout (`--jobs`' workers).
    pub worker: bool,
}

/// "6h", "30m", "10s", "250ms", or a bare number of ms.
pub fn parse_duration(text: &str) -> Result<u64, String> {
    let t = text.trim();
    let (number, scale) = if let Some(n) = t.strip_suffix("ms") {
        (n, 1.0)
    } else if let Some(n) = t.strip_suffix('h') {
        (n, 3_600_000.0)
    } else if let Some(n) = t.strip_suffix('m') {
        (n, 60_000.0)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1_000.0)
    } else {
        (t, 1.0)
    };
    match number.trim().parse::<f64>() {
        Ok(n) if n.is_finite() && n >= 0.0 => Ok((n * scale).round() as u64),
        _ => Err(format!(
            "`{text}` is not a duration (6h, 30m, 10s, 250ms, or ms)"
        )),
    }
}

/// Virtual ms as a person reads it: `1h 02m 03.250s`, `2m 03.500s`,
/// `0.250s`.
pub fn fmt_virtual(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms / 60_000) % 60;
    let s = (ms % 60_000) as f64 / 1000.0;
    if h > 0 {
        format!("{h}h {m:02}m {s:06.3}s")
    } else if m > 0 {
        format!("{m}m {s:06.3}s")
    } else {
        format!("{s:.3}s")
    }
}

/// One event as a line.
pub fn fmt_event(e: &Event) -> String {
    let task = match e.task {
        Some(t) => format!("task {t:<3}"),
        None => "net     ".to_string(),
    };
    format!(
        "  {:>14}  {:<8} {} {:<8} {}",
        fmt_virtual(e.t),
        e.node,
        task,
        e.kind,
        e.detail
    )
}

/// Does `source` build worlds (S2) rather than run inside one (S1)? A
/// program that runs inside a world cannot call `sim.world`: worlds do
/// not nest.
pub fn builds_worlds(source: &str) -> bool {
    source
        .lines()
        .any(|l| l.split("//").next().unwrap_or("").contains("sim.world("))
}

fn read(path: &Path) -> Result<(String, crate::ast::Program, std::path::PathBuf), String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let program = crate::parser::Parser::new()
        .parse(&source)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    Ok((source, program, absolute))
}

/// Run `path` as a world's main program with `cfg`.
pub fn run_file(path: &Path, cfg: Config) -> Result<Report, String> {
    run_file_forced(path, cfg, None).map(|(r, _)| r)
}

/// `run_file`, forced by `plans[0]` when given, with the run's recorded plan.
pub fn run_file_forced(
    path: &Path,
    mut cfg: Config,
    plans: Option<Vec<RunPlan>>,
) -> Result<(Report, Vec<RunPlan>), String> {
    if let Some(p) = plans {
        cfg.forced = Some(p.into_iter().next().unwrap_or_default());
    }
    let (_, program, absolute) = read(path)?;
    // The bytecode tier unless asked otherwise (`super::bytecode_tier`):
    // every builtin it calls still passes the world's dispatch hook.
    let mut interpreter = crate::interpreter::Interpreter::new();
    super::tier_for(&mut interpreter);
    interpreter.set_current_file(&absolute);
    if let Some(deps) = super::surface::package_deps(&absolute) {
        interpreter.set_dependency_map(deps);
    }
    let argv = vec![path.to_string_lossy().to_string()];
    let report = super::run_main(cfg, super::NodeInfo::main(argv), move || {
        let r = interpreter.eval_program(program);
        super::sited(&mut interpreter, r)
    })?;
    let plans = vec![report.plan.clone()];
    Ok((report, plans))
}

/// Run `path`, a file that builds worlds, with every world taking
/// `seed`: one report standing for them all (the last failing run, or
/// the last run; the file's own error, if it raised, as `failed`).
pub fn run_builder(path: &Path, seed: u64) -> Result<Report, String> {
    run_builder_forced(path, seed, None).map(|(r, _)| r)
}

/// `run_builder`, with each world run forced by its plan, and every run's
/// recorded plan (by its number in the file).
pub fn run_builder_forced(
    path: &Path,
    seed: u64,
    plans: Option<Vec<RunPlan>>,
) -> Result<(Report, Vec<RunPlan>), String> {
    let (_, program, absolute) = read(path)?;
    let mut interpreter = crate::interpreter::Interpreter::new();
    super::tier_for(&mut interpreter);
    interpreter.set_current_file(&absolute);
    // its packages, as `olang run` resolves them (fetching nothing)
    if let Some(deps) = super::surface::package_deps(&absolute) {
        interpreter.set_dependency_map(deps);
    }
    super::surface::collect_begin_forced(seed, plans);
    let result = interpreter.eval_program(program);
    let reports = super::surface::collect_end();
    let recorded = recorded_plans(&reports);
    let mut report = reports
        .iter()
        .rev()
        .find(|r| r.failed())
        .or(reports.last())
        .cloned()
        .unwrap_or_else(|| Report::empty(seed));
    // every run's virtual time counts toward the rate
    report.now = reports.iter().map(|r| r.now).max().unwrap_or(report.now);
    if let Err(e) = result
        && !report.failed()
    {
        report.outcome = "failed";
        report.error = Some(super::error_text(&e));
        report.site = super::error_site(&mut interpreter);
    }
    Ok((report, recorded))
}

/// Each run's recorded plan, by its number in the file.
fn recorded_plans(reports: &[Report]) -> Vec<RunPlan> {
    let n = reports.iter().map(|r| r.run_index + 1).max().unwrap_or(0);
    let mut plans = vec![RunPlan::default(); n];
    for r in reports {
        plans[r.run_index] = r.plan.clone();
    }
    plans
}

/// Does the file at `path`, or a local module it `use`s (`use log_world
/// { run_store }` naming `log_world.ol` beside it, and so on down), build
/// worlds?
pub fn file_builds_worlds(path: &Path) -> bool {
    fn walk(path: &Path, seen: &mut Vec<std::path::PathBuf>) -> bool {
        if seen.iter().any(|p| p == path) || seen.len() > 64 {
            return false;
        }
        seen.push(path.to_path_buf());
        let Ok(source) = std::fs::read_to_string(path) else {
            return false;
        };
        if builds_worlds(&source) {
            return true;
        }
        let dir = path.parent().unwrap_or(Path::new("."));
        source.lines().any(|line| {
            let Some(rest) = line.trim_start().strip_prefix("use ") else {
                return false;
            };
            let module: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            let file = dir.join(format!("{}.ol", module.replace('.', "/")));
            file.is_file() && walk(&file, seen)
        })
    }
    walk(path, &mut Vec::new())
}

/// `run_file` or `run_builder`, as the file asks; forced by `plans` when
/// given; with every run's recorded plan.
pub fn run_any(
    path: &Path,
    cfg: Config,
    plans: Option<Vec<RunPlan>>,
) -> Result<(Report, Vec<RunPlan>), String> {
    if file_builds_worlds(path) {
        run_builder_forced(path, cfg.seed, plans)
    } else {
        run_file_forced(path, cfg, plans)
    }
}

fn failure_line(r: &Report) -> String {
    if let Some(v) = &r.violation {
        return v.describe();
    }
    let what = r.error.clone().unwrap_or_default();
    format!(
        "{} at {} virtual: {}",
        r.outcome,
        fmt_virtual(r.now),
        what.trim()
    )
}

/// Virtual time per real second as a person reads it: `3h 14m`, `12m
/// 05s`, `41.2s`.
pub fn fmt_rate(virtual_ms_per_s: f64) -> String {
    let ms = virtual_ms_per_s.max(0.0);
    let h = (ms / 3_600_000.0).floor();
    let m = ((ms - h * 3_600_000.0) / 60_000.0).floor();
    let s = (ms - h * 3_600_000.0 - m * 60_000.0) / 1000.0;
    if h > 0.0 {
        format!("{h:.0}h {m:02.0}m")
    } else if m > 0.0 {
        format!("{m:.0}m {s:02.0}s")
    } else {
        format!("{s:.1}s")
    }
}

/// `olang sim`: the exit code.
pub fn main(file: &Path, opts: &Options) -> i32 {
    let base = Config {
        run_for: opts.run_for,
        max_steps: opts.max_steps.unwrap_or(super::DEFAULT_MAX_STEPS),
        ..Config::default()
    };
    if opts.worker {
        return worker(file, &base);
    }
    if opts.events && opts.seeds.is_some() {
        eprintln!(
            "olang sim: --events prints one run's events; give it one seed (--seed S) \
             rather than --seeds, or see a run with `olang sim view FILE --seed S`"
        );
        return 2;
    }
    if let Some(plan) = &opts.replay {
        return replay(plan, opts.events);
    }
    match opts.seeds {
        None if opts.shrink => {
            let seed = opts.seed.unwrap_or(1);
            let code = one_seed(file, seed, base.clone(), opts.events);
            if code == 1 {
                shrink_seed(file, seed, &base, opts, None);
            }
            code
        }
        None => one_seed(file, opts.seed.unwrap_or(1), base, opts.events),
        Some(n) => many_seeds(file, opts.seed.unwrap_or(1), n, base, opts),
    }
}

fn one_seed(file: &Path, seed: u64, base: Config, all_events: bool) -> i32 {
    let started = crate::clock::Instant::now();
    let report = match run_any(file, Config { seed, ..base }, None) {
        Ok((r, _)) => r,
        Err(e) => {
            eprintln!("olang sim: {e}");
            return 2;
        }
    };
    let real = started.elapsed().as_secs_f64();
    show_run(&report, seed, real, all_events)
}

/// A run, told as `--seed` tells it: the code it exits with.
fn show_run(report: &Report, seed: u64, real: f64, all_events: bool) -> i32 {
    if all_events {
        println!("events:");
        for e in &report.events {
            println!("{}", fmt_event(e));
        }
    }
    for e in report.events.iter().filter(|e| e.kind == "fail") {
        println!(
            "olang sim: {} task {} failed: {}",
            e.node,
            e.task.unwrap_or(0),
            e.detail
        );
    }
    if report.failed() {
        if !all_events {
            let tail = report.events.len().saturating_sub(20);
            println!(
                "the last {} of {} events:",
                report.events.len() - tail,
                report.events.len()
            );
            for e in &report.events[tail..] {
                println!("{}", fmt_event(e));
            }
        }
        println!("olang sim: seed {seed}: {}", failure_line(report));
        return 1;
    }
    println!(
        "olang sim: seed {seed}: {} at {} virtual · {} task(s) · {} events · {:.3} s real",
        report.outcome,
        fmt_virtual(report.now),
        report.tasks,
        report.events.len(),
        real
    );
    if report.stuck {
        println!("olang sim: note: a task did not stop when the world ended");
    }
    0
}

// ─── seeds in parallel ───────────────────────────────────────────────

/// One run asked of a worker: a seed, and the plans forcing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub seed: u64,
    #[serde(default)]
    pub plans: Option<Vec<RunPlan>>,
    /// Send back every run's recorded plan.
    #[serde(default)]
    pub want_plans: bool,
}

/// What one run came to, as a worker reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeedResult {
    pub seed: u64,
    pub failed: bool,
    pub outcome: String,
    /// Virtual ms at the end.
    pub now: u64,
    pub steps: u64,
    /// The failure, as a line (`failure_line`).
    pub failure: Option<String>,
    pub property: Option<String>,
    pub broke_at: Option<u64>,
    pub signature: String,
    pub digest: String,
    /// Faults a plan can force.
    pub faults: usize,
    pub preemptions: u64,
    pub run_index: usize,
    pub optional: Vec<String>,
    #[serde(default)]
    pub plans: Option<Vec<RunPlan>>,
    /// The engine could not run it (a parse error, a worker that died).
    #[serde(default)]
    pub error: Option<String>,
}

impl SeedResult {
    fn of(seed: u64, run: Result<(Report, Vec<RunPlan>), String>, want_plans: bool) -> SeedResult {
        match run {
            Ok((r, plans)) => SeedResult {
                seed,
                failed: r.failed(),
                outcome: r.outcome.to_string(),
                now: r.now,
                steps: r.steps,
                failure: r.failed().then(|| failure_line(&r)),
                property: r.violation.as_ref().map(|v| v.property.clone()),
                broke_at: r.violation.as_ref().map(|v| v.t),
                signature: r.signature(),
                digest: format!("{:016x}", r.digest()),
                faults: r.plannable_faults(),
                preemptions: r.preemptions,
                run_index: r.run_index,
                optional: r.optional.clone(),
                plans: want_plans.then_some(plans),
                error: None,
            },
            Err(e) => SeedResult::broken(seed, e),
        }
    }

    fn broken(seed: u64, e: String) -> SeedResult {
        SeedResult {
            seed,
            failed: true,
            outcome: "error".into(),
            now: 0,
            steps: 0,
            failure: Some(e.clone()),
            property: None,
            broke_at: None,
            signature: format!("error: {e}"),
            digest: String::new(),
            faults: 0,
            preemptions: 0,
            run_index: 0,
            optional: Vec::new(),
            plans: None,
            error: Some(e),
        }
    }
}

/// The marker a worker's result lines start with (anything else it
/// writes to stdout is passed over).
const MARK: &str = "\u{1e}olang-sim ";

/// Run one request here, the program's output kept quiet.
pub fn run_request(file: &Path, base: &Config, req: &Request) -> SeedResult {
    let cfg = Config {
        seed: req.seed,
        ..base.clone()
    };
    let run = run_any(file, cfg, req.plans.clone());
    #[cfg(feature = "native")]
    let _ = crate::output::take_collected();
    SeedResult::of(req.seed, run, req.want_plans)
}

/// `olang sim FILE --worker`: requests on stdin, one JSON line each;
/// results on stdout, one marked line each.
fn worker(file: &Path, base: &Config) -> i32 {
    #[cfg(feature = "native")]
    crate::output::discard_output();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let result = match serde_json::from_str::<Request>(&line) {
            Ok(req) => run_request(file, base, &req),
            Err(e) => SeedResult::broken(0, format!("worker: bad request: {e}")),
        };
        let text = serde_json::to_string(&result).unwrap_or_default();
        if writeln!(out, "{MARK}{text}")
            .and_then(|_| out.flush())
            .is_err()
        {
            break;
        }
    }
    0
}

/// Run `reqs`, here (`jobs` ≤ 1) or across `jobs` worker processes, each
/// taking the next request as it finishes one; the results in the order
/// asked. `progress(done)` is told as each comes in.
pub fn run_all(
    file: &Path,
    base: &Config,
    reqs: &[Request],
    jobs: usize,
    progress: &(dyn Fn(usize, &SeedResult) + Sync),
) -> Vec<SeedResult> {
    if jobs <= 1 || reqs.len() <= 1 {
        #[cfg(feature = "native")]
        crate::output::collect_output();
        return reqs
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let res = run_request(file, base, r);
                progress(i + 1, &res);
                res
            })
            .collect();
    }
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            return reqs
                .iter()
                .map(|r| SeedResult::broken(r.seed, format!("no olang binary to run workers: {e}")))
                .collect();
        }
    };
    let next = std::sync::Mutex::new(0usize);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let results: std::sync::Mutex<Vec<Option<SeedResult>>> =
        std::sync::Mutex::new(vec![None; reqs.len()]);
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(reqs.len()) {
            scope.spawn(|| {
                let mut cmd = std::process::Command::new(&exe);
                cmd.arg("sim").arg(file).arg("--worker");
                // the tier this process runs worlds on, the workers too
                cmd.env(
                    "OLANG_SIM_TIER",
                    if super::bytecode_tier() {
                        "bytecode"
                    } else {
                        "interp"
                    },
                );
                if let Some(ms) = base.run_for {
                    cmd.arg("--for").arg(ms.to_string());
                }
                cmd.arg("--max-steps").arg(base.max_steps.to_string());
                cmd.stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null());
                let mut child = match cmd.spawn() {
                    Ok(c) => Some(c),
                    Err(_) => None,
                };
                let mut io = child.as_mut().and_then(|c| {
                    Some((c.stdin.take()?, std::io::BufReader::new(c.stdout.take()?)))
                });
                loop {
                    let i = {
                        let mut n = next.lock().unwrap_or_else(|e| e.into_inner());
                        if *n >= reqs.len() {
                            break;
                        }
                        *n += 1;
                        *n - 1
                    };
                    let req = &reqs[i];
                    let res = match io.as_mut() {
                        None => SeedResult::broken(req.seed, "could not start a worker".into()),
                        Some((tx, rx)) => ask(tx, rx, req),
                    };
                    if res
                        .error
                        .as_deref()
                        .is_some_and(|e| e.starts_with("worker died"))
                    {
                        io = None;
                    }
                    let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    progress(n, &res);
                    results.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(res);
                }
                drop(io);
                if let Some(mut c) = child {
                    let _ = c.wait();
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .zip(reqs)
        .map(|(r, q)| r.unwrap_or_else(|| SeedResult::broken(q.seed, "not run".into())))
        .collect()
}

/// One request to a worker, and its answer.
fn ask(
    tx: &mut std::process::ChildStdin,
    rx: &mut std::io::BufReader<std::process::ChildStdout>,
    req: &Request,
) -> SeedResult {
    let line = serde_json::to_string(req).unwrap_or_default();
    if writeln!(tx, "{line}").and_then(|_| tx.flush()).is_err() {
        return SeedResult::broken(req.seed, "worker died (its stdin closed)".into());
    }
    let mut buf = String::new();
    loop {
        buf.clear();
        match rx.read_line(&mut buf) {
            Ok(0) | Err(_) => {
                return SeedResult::broken(req.seed, "worker died before answering".into());
            }
            Ok(_) => {
                if let Some(json) = buf.trim_end().strip_prefix(MARK) {
                    return serde_json::from_str(json).unwrap_or_else(|e| {
                        SeedResult::broken(req.seed, format!("worker: bad answer: {e}"))
                    });
                }
            }
        }
    }
}

/// The digest of a sweep: every result's digest, in seed order.
pub fn sweep_digest(results: &[SeedResult]) -> String {
    let mut h = super::Fnv::new();
    for r in results {
        h.u64(r.seed);
        h.str(&r.digest);
        h.str(&r.outcome);
    }
    format!("{:016x}", h.0)
}

fn jobs_of(opts: &Options) -> usize {
    opts.jobs.unwrap_or(1).max(1)
}

fn many_seeds(file: &Path, from: u64, n: u64, base: Config, opts: &Options) -> i32 {
    let jobs = jobs_of(opts);
    let started = crate::clock::Instant::now();
    let reqs: Vec<Request> = (from..from.saturating_add(n))
        .map(|seed| Request {
            seed,
            plans: None,
            want_plans: false,
        })
        .collect();
    let tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let failed_so_far = std::sync::atomic::AtomicUsize::new(0);
    let progress = |done: usize, r: &SeedResult| {
        if r.failed {
            failed_so_far.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if tty {
            eprint!(
                "\r  {done}/{n} seeds · {} failed",
                failed_so_far.load(std::sync::atomic::Ordering::SeqCst)
            );
        }
    };
    let results = run_all(file, &base, &reqs, jobs, &progress);
    if tty {
        eprint!("\r\x1b[2K");
    }
    let real = started.elapsed().as_secs_f64().max(1e-9);
    if let Some(e) = results.iter().find_map(|r| r.error.clone())
        && results.iter().all(|r| r.error.is_some())
    {
        eprintln!("olang sim: {e}");
        return 2;
    }
    let failed: Vec<&SeedResult> = results.iter().filter(|r| r.failed).collect();
    let virtual_ms: u128 = results.iter().map(|r| r.now as u128).sum();
    let rate = virtual_ms as f64 / real;
    for r in &failed {
        println!("seed {}: {}", r.seed, r.failure.clone().unwrap_or_default());
    }
    println!(
        "olang sim: {} seed(s) · {} passed · {} failed · {} simulated per second ({:.2} virtual hours) · {} job(s) · {:.3} s real",
        n,
        results.len() - failed.len(),
        failed.len(),
        fmt_rate(rate),
        rate / 3_600_000.0,
        jobs,
        real
    );
    println!("digest: {}", sweep_digest(&results));
    if let Some(first) = failed.first() {
        println!(
            "reproduce: olang sim {} --seed {}",
            file.display(),
            first.seed
        );
        if opts.shrink {
            shrink_seed(file, first.seed, &base, opts, None);
        }
        return 1;
    }
    0
}

// ─── shrinking and replaying ─────────────────────────────────────────

/// A minimal failing run, as `.sim/<seed>.simplan` holds it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanFile {
    /// The format's version.
    pub olang_sim_plan: u32,
    /// The file the world comes from (absolute).
    pub file: String,
    pub seed: u64,
    #[serde(rename = "for")]
    pub run_for: Option<u64>,
    pub max_steps: u64,
    /// The plan forcing each world run of the file, by its number.
    pub runs: Vec<RunPlan>,
    /// How it fails: `failed`, `violated`, ...; the signature the shrinker
    /// kept; the failure as a line.
    pub outcome: String,
    pub signature: String,
    pub failure: String,
    /// The run's digest: a replay that differs did not reproduce it.
    pub digest: String,
}

pub fn load_plan(path: &Path) -> Result<PlanFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let plan: PlanFile = serde_json::from_str(&text)
        .map_err(|e| format!("{}: not a sim plan: {e}", path.display()))?;
    if plan.olang_sim_plan != 1 {
        return Err(format!(
            "{}: plan format {} (this olang reads 1)",
            path.display(),
            plan.olang_sim_plan
        ));
    }
    Ok(plan)
}

/// `--shrink`: minimise `seed`'s failure, print it, save it.
fn shrink_seed(
    file: &Path,
    seed: u64,
    base: &Config,
    opts: &Options,
    start: Option<Vec<RunPlan>>,
) -> Option<PathBuf> {
    let jobs = jobs_of(opts);
    let budget = opts.shrink_budget.unwrap_or(super::shrink::DEFAULT_BUDGET);
    eprintln!("shrinking seed {seed} ({jobs} job(s), at most {budget} runs)...");
    let runner = |reqs: &[Request]| run_all(file, base, reqs, jobs, &|_, _| {});
    let shrunk = match super::shrink::shrink_from(seed, &runner, budget, start, jobs) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("olang sim: cannot shrink seed {seed}: {e}");
            return None;
        }
    };
    println!("{}", shrunk.summary());
    for line in shrunk.listing() {
        println!("  {line}");
    }
    let absolute = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(file)
    };
    let plan = PlanFile {
        olang_sim_plan: 1,
        file: absolute.display().to_string(),
        seed,
        run_for: base.run_for,
        max_steps: base.max_steps,
        runs: shrunk.plans.clone(),
        outcome: shrunk.result.outcome.clone(),
        signature: shrunk.result.signature.clone(),
        failure: shrunk.result.failure.clone().unwrap_or_default(),
        digest: shrunk.result.digest.clone(),
    };
    let dir = PathBuf::from(".sim");
    let path = dir.join(format!("{seed}.simplan"));
    let saved = std::fs::create_dir_all(&dir).and_then(|_| {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&plan).unwrap_or_default() + "\n",
        )
    });
    match saved {
        Ok(()) => {
            println!(
                "plan: {}   (olang sim {} --replay {})",
                path.display(),
                file.display(),
                path.display()
            );
            Some(path)
        }
        Err(e) => {
            eprintln!("olang sim: could not save {}: {e}", path.display());
            None
        }
    }
}

/// `--replay PLAN`: the plan's run again, checked against its digest.
fn replay(plan_path: &Path, all_events: bool) -> i32 {
    let plan = match load_plan(plan_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("olang sim: {e}");
            return 2;
        }
    };
    let file = PathBuf::from(&plan.file);
    let cfg = Config {
        seed: plan.seed,
        run_for: plan.run_for,
        max_steps: plan.max_steps,
        ..Config::default()
    };
    let started = crate::clock::Instant::now();
    let report = match run_any(&file, cfg, Some(plan.runs.clone())) {
        Ok((r, _)) => r,
        Err(e) => {
            eprintln!("olang sim: {e}");
            return 2;
        }
    };
    let real = started.elapsed().as_secs_f64();
    let code = show_run(&report, plan.seed, real, all_events);
    let digest = format!("{:016x}", report.digest());
    if digest == plan.digest {
        println!(
            "olang sim: replayed {} bit for bit (digest {digest}, {} fault(s))",
            plan_path.display(),
            report.plannable_faults()
        );
        code
    } else {
        eprintln!(
            "olang sim: the replay diverged from {}: digest {digest}, recorded {}",
            plan_path.display(),
            plan.digest
        );
        3
    }
}

/// `olang sim shrink PLAN`: shrink a saved plan further, starting from
/// it (with its settings), and save the result as `.sim/<seed>.simplan`.
pub fn shrink_plan(plan_path: &Path, opts: &Options) -> i32 {
    let plan = match load_plan(plan_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("olang sim: {e}");
            return 2;
        }
    };
    let base = Config {
        run_for: plan.run_for,
        max_steps: plan.max_steps,
        ..Config::default()
    };
    match shrink_seed(
        Path::new(&plan.file),
        plan.seed,
        &base,
        opts,
        Some(plan.runs),
    ) {
        Some(_) => 0,
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_people_write_them() {
        assert_eq!(parse_duration("6h"), Ok(21_600_000));
        assert_eq!(parse_duration("30m"), Ok(1_800_000));
        assert_eq!(parse_duration("1.5s"), Ok(1_500));
        assert_eq!(parse_duration("250ms"), Ok(250));
        assert_eq!(parse_duration("42"), Ok(42));
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn virtual_time_prints_as_people_read_it() {
        assert_eq!(fmt_virtual(250), "0.250s");
        assert_eq!(fmt_virtual(123_500), "2m 03.500s");
        assert_eq!(fmt_virtual(3_723_250), "1h 02m 03.250s");
    }

    #[test]
    fn a_file_that_calls_sim_world_builds_worlds() {
        assert!(builds_worlds("let w = sim.world(#{ \"seed\": 1 })"));
        assert!(!builds_worlds("// sim.world( in a comment\nprintln(1)"));
        assert!(!builds_worlds("spawn { chan.recv(c) }"));
    }
}
