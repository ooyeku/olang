//! `olang sim FILE [--seed S] [--seeds N] [--for 6h] [--events]`: run a
//! program as the main program of a simulated world, once with one seed
//! or once per seed for many, and say which seeds fail.
//!
//! - `--seed S` runs seed S with the program's output shown, then a
//!   summary; a failure prints its error and the events that led to it.
//!   Running the seed a failure was reported with reproduces it.
//! - `--seeds N` runs N seeds in turn (from `--seed`, default 1), the
//!   program's output kept quiet, and prints each failing seed with what
//!   went wrong and the line that reproduces it. (Parallel `--jobs` and
//!   shrinking are S4.)

use super::{Config, Event, Report};
use std::path::Path;

/// What the command was asked.
#[derive(Debug, Clone)]
pub struct Options {
    pub seed: Option<u64>,
    pub seeds: Option<u64>,
    pub run_for: Option<u64>,
    pub events: bool,
    pub max_steps: Option<u64>,
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
    format!(
        "  {:>14}  task {:<3} {:<8} {}",
        fmt_virtual(e.t),
        e.task,
        e.kind,
        e.detail
    )
}

/// Run `path` as a world's main program with `cfg`.
pub fn run_file(path: &Path, cfg: Config) -> Result<Report, String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let program = crate::parser::Parser::new()
        .parse(&source)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    // The interpreter tier, as record and replay run: no bytecode tier
    // is enabled, so every builtin passes the world's dispatch hook.
    let mut interpreter = crate::interpreter::Interpreter::new();
    interpreter.set_current_file(&absolute);
    super::run(cfg, move || interpreter.eval_program(program))
}

fn failure_line(r: &Report) -> String {
    let what = r.error.clone().unwrap_or_default();
    format!(
        "{} at {} virtual: {}",
        r.outcome,
        fmt_virtual(r.now),
        what.trim()
    )
}

/// `olang sim`: the exit code.
pub fn main(file: &Path, opts: &Options) -> i32 {
    let base = Config {
        run_for: opts.run_for,
        max_steps: opts.max_steps.unwrap_or(super::DEFAULT_MAX_STEPS),
        ..Config::default()
    };
    match opts.seeds {
        None => one_seed(file, opts.seed.unwrap_or(1), base, opts.events),
        Some(n) => many_seeds(file, opts.seed.unwrap_or(1), n, base),
    }
}

fn one_seed(file: &Path, seed: u64, base: Config, all_events: bool) -> i32 {
    let started = crate::clock::Instant::now();
    let report = match run_file(file, Config { seed, ..base }) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("olang sim: {e}");
            return 2;
        }
    };
    let real = started.elapsed().as_secs_f64();
    if all_events {
        eprintln!("events:");
        for e in &report.events {
            eprintln!("{}", fmt_event(e));
        }
    }
    for e in report.events.iter().filter(|e| e.kind == "fail") {
        eprintln!("olang sim: task {} failed: {}", e.task, e.detail);
    }
    if report.failed() {
        if !all_events {
            let tail = report.events.len().saturating_sub(20);
            eprintln!(
                "the last {} of {} events:",
                report.events.len() - tail,
                report.events.len()
            );
            for e in &report.events[tail..] {
                eprintln!("{}", fmt_event(e));
            }
        }
        eprintln!("olang sim: seed {seed}: {}", failure_line(&report));
        return 1;
    }
    eprintln!(
        "olang sim: seed {seed}: {} at {} virtual · {} task(s) · {} events · {:.3} s real",
        report.outcome,
        fmt_virtual(report.now),
        report.tasks,
        report.events.len(),
        real
    );
    if report.stuck {
        eprintln!("olang sim: note: a task did not stop when the world ended");
    }
    0
}

fn many_seeds(file: &Path, from: u64, n: u64, base: Config) -> i32 {
    // The program's own output would bury the failures: keep it quiet.
    #[cfg(feature = "native")]
    crate::output::collect_output();
    let started = crate::clock::Instant::now();
    let mut failed: Vec<u64> = Vec::new();
    let mut virtual_ms: u128 = 0;
    for seed in from..from.saturating_add(n) {
        let report = run_file(
            file,
            Config {
                seed,
                ..base.clone()
            },
        );
        #[cfg(feature = "native")]
        let _ = crate::output::take_collected();
        let report = match report {
            Ok(r) => r,
            Err(e) => {
                eprintln!("olang sim: {e}");
                return 2;
            }
        };
        virtual_ms += report.now as u128;
        if report.failed() {
            eprintln!("seed {seed}: {}", failure_line(&report));
            failed.push(seed);
        }
    }
    let real = started.elapsed().as_secs_f64().max(1e-9);
    let rate = virtual_ms as f64 / 3_600_000.0 / real;
    eprintln!(
        "olang sim: {} seed(s) · {} passed · {} failed · {:.3} s real · {:.1} virtual hours per real second",
        n,
        n as usize - failed.len(),
        failed.len(),
        real,
        rate
    );
    if let Some(first) = failed.first() {
        eprintln!("reproduce: olang sim {} --seed {first}", file.display());
        return 1;
    }
    0
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
}
