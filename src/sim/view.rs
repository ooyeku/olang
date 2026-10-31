//! S5, seeing a run: its events as an `ods` Frame (`run.events()`), and
//! `olang sim view`, a swimlane timeline of a run replayed from a saved
//! plan or from a file and a seed.
//!
//! **One model, drawn anywhere.** `model` turns a report into plain data:
//! the lanes (one per node), every event placed on its lane, each message
//! as the two ends of an arrow, the faults, the stretches a node was down
//! and the partitions. The view is that model drawn by a self-contained
//! page (`view.html`: one file, no network, a canvas so a run of a
//! million events scrolls), written with `--html` and opened in the
//! system's browser otherwise; `--json` writes the model itself, for
//! another renderer (a Loom panel, Studio) to draw.
//!
//! A run is replayed to be seen: a plan replays bit for bit (its digest is
//! checked, and a view of a run that diverged says so on stderr), and a
//! file and a seed is the same run every time.

use super::cli::{self, PlanFile};
use super::{Config, Report};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ─── run.events() ────────────────────────────────────────────────────

/// Every event of a run as an `ods` Frame, one row an event, in the order
/// they happened: `seq`, `t` (virtual ms), `node`, `task` (null for what
/// the network did on its own), `kind`, `detail`, `peer` and `sent` (a
/// message's other end and when it was sent; null for the rest),
/// `inbound` (an arrival, not a loss on leaving; null when no message)
/// and `fault` (the world injected it).
pub fn events_frame(r: &Report) -> Result<crate::ast::Value, String> {
    use olang_ods::Series;
    let n = r.events.len();
    let mut seq = Vec::with_capacity(n);
    let mut t = Vec::with_capacity(n);
    let mut node = Vec::with_capacity(n);
    let mut task = Vec::with_capacity(n);
    let mut kind = Vec::with_capacity(n);
    let mut detail = Vec::with_capacity(n);
    let mut peer = Vec::with_capacity(n);
    let mut sent = Vec::with_capacity(n);
    let mut inbound = Vec::with_capacity(n);
    let mut fault = Vec::with_capacity(n);
    for (i, e) in r.events.iter().enumerate() {
        seq.push(i as i64);
        t.push(e.t as i64);
        node.push(e.node.to_string());
        task.push(e.task.map(|t| t as i64));
        kind.push(e.kind.to_string());
        detail.push(e.detail.clone());
        peer.push(e.link.as_ref().map(|l| l.peer.to_string()));
        sent.push(e.link.as_ref().map(|l| l.sent as i64));
        inbound.push(e.link.as_ref().map(|l| l.inbound));
        fault.push(e.fault);
    }
    crate::ods::OdsFrame::from_series(vec![
        ("seq".into(), Series::from_i64(seq)),
        ("t".into(), Series::from_i64(t)),
        ("node".into(), Series::from_str_values(node)),
        ("task".into(), Series::from_i64_options(task)),
        ("kind".into(), Series::from_str_values(kind)),
        ("detail".into(), Series::from_str_values(detail)),
        ("peer".into(), Series::from_str_options(peer)),
        ("sent".into(), Series::from_i64_options(sent)),
        ("inbound".into(), Series::from_bool_options(inbound)),
        ("fault".into(), Series::from_bool(fault)),
    ])
}

// ─── the model ───────────────────────────────────────────────────────

/// How an event is drawn: a message's end, a fault, a node going down or
/// coming back, the run's end, or a plain tick on its lane.
fn class_of(e: &super::Event) -> &'static str {
    if e.fault {
        return "fault";
    }
    match e.kind {
        "deliver" => "msg",
        "drop" | "late" | "refused" | "dup" if e.link.is_some() => "lost",
        "exit" | "restart" => "life",
        "fail" | "failed" | "deadlock" | "step_limit" | "violated" => "error",
        "start" | "done" | "deadline" => "edge",
        _ => "tick",
    }
}

/// The run as data a renderer draws (`view.html`, or another).
///
/// `events` are rows `[t, lane, task, kind, detail, peer, sent, class,
/// inbound]` (`task` and `peer` −1 when none, `sent` −1 when no message;
/// `kind` an index into `kinds`), in the order they happened. `down` is
/// each stretch a node was down (`[lane, from, to, how]`, `to` the run's
/// end when it never came back), and `cuts` each partition
/// (`[from, to, left lanes, right lanes]`).
pub fn model(r: &Report, title: &str) -> Json {
    // lanes: the nodes in the world's order, then any node only events name
    let mut lanes: Vec<String> = r.nodes.clone();
    for e in &r.events {
        if !lanes.iter().any(|l| **l == *e.node) {
            lanes.push(e.node.to_string());
        }
    }
    // a lane nothing happened on is left out (the world's own lane of a
    // run_fn world that only started and ended stays: it has events)
    let used: std::collections::BTreeSet<&str> = r.events.iter().map(|e| &*e.node).collect();
    lanes.retain(|l| used.contains(l.as_str()));
    let lane = |name: &str| lanes.iter().position(|l| l == name).map(|i| i as i64);

    let mut kinds: Vec<&'static str> = Vec::new();
    let mut kind_ix: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut rows = Vec::with_capacity(r.events.len());
    let mut down: Vec<Json> = Vec::new();
    let mut down_since: BTreeMap<i64, (u64, &'static str)> = BTreeMap::new();
    let mut cuts: Vec<Json> = Vec::new();
    let mut cut_open: Vec<(u64, Vec<i64>, Vec<i64>)> = Vec::new();
    for e in &r.events {
        let k = *kind_ix.entry(e.kind).or_insert_with(|| {
            kinds.push(e.kind);
            kinds.len() - 1
        });
        let l = lane(&e.node).unwrap_or(0);
        let (peer, sent, inbound) = match &e.link {
            Some(link) => (
                lane(&link.peer).unwrap_or(-1),
                link.sent as i64,
                link.inbound,
            ),
            None => (-1, -1, false),
        };
        rows.push(json!([
            e.t,
            l,
            e.task.map(|t| t as i64).unwrap_or(-1),
            k,
            e.detail,
            peer,
            sent,
            class_of(e),
            inbound as u8
        ]));
        match e.kind {
            "crash" | "kill" | "stop" | "exit" if e.fault || e.kind == "exit" => {
                down_since.entry(l).or_insert((e.t, e.kind));
            }
            // the restarted node is named by the detail (the event is
            // the restarter's)
            "restart" => {
                let n = lane(e.detail.trim()).unwrap_or(l);
                if let Some((from, how)) = down_since.remove(&n) {
                    down.push(json!([n, from, e.t, how]));
                }
            }
            "partition" if e.fault => {
                if let Some((left, right)) = e.detail.split_once(" | ") {
                    let names = |s: &str| -> Vec<i64> {
                        s.trim_matches(|c| c == '[' || c == ']')
                            .split(", ")
                            .filter_map(|n| lane(n.trim()))
                            .collect()
                    };
                    cut_open.push((e.t, names(left), names(right)));
                }
            }
            "heal" => {
                for (from, left, right) in cut_open.drain(..) {
                    cuts.push(json!([from, e.t, left, right]));
                }
            }
            _ => {}
        }
    }
    for (l, (from, how)) in down_since {
        down.push(json!([l, from, r.now, how]));
    }
    for (from, left, right) in cut_open {
        cuts.push(json!([from, r.now, left, right]));
    }
    let violation = r
        .violation
        .as_ref()
        .map(|v| json!({ "property": v.property, "kind": v.kind, "t": v.t, "detail": v.detail }));
    json!({
        "olang_sim_view": 1,
        "title": title,
        "seed": r.seed,
        "outcome": r.outcome,
        "failed": r.failed(),
        "error": r.error,
        "violation": violation,
        "now": r.now,
        "lanes": lanes,
        "kinds": kinds,
        "events": rows,
        "down": down,
        "cuts": cuts,
        "digest": format!("{:016x}", r.digest()),
    })
}

const PAGE: &str = include_str!("view.html");

/// The model as a page: `view.html` with the model in it.
pub fn page(model: &Json) -> String {
    let data = serde_json::to_string(model)
        .unwrap_or_else(|_| "{}".into())
        // the data sits in a <script>: nothing in it may close the tag
        .replace("</", "<\\/")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let title = model["title"].as_str().unwrap_or("sim view");
    PAGE.replacen("__TITLE__", &html_escape(title), 1)
        .replacen("__DATA__", &data, 1)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ─── olang sim view ──────────────────────────────────────────────────

/// What `olang sim view` was asked.
#[derive(Debug, Clone, Default)]
pub struct ViewOptions {
    /// Write the page here (and open nothing).
    pub html: Option<PathBuf>,
    /// Write the model as JSON here.
    pub json: Option<PathBuf>,
    /// Do not open the page in the browser.
    pub no_open: bool,
}

/// The run `target` names: a saved plan (`.simplan`, replayed and its
/// digest checked) or a file run with `seed` (and `run_for`). The report,
/// its title, and whether a plan's replay diverged.
pub fn replay_for_view(
    target: &Path,
    seed: Option<u64>,
    run_for: Option<u64>,
    max_steps: Option<u64>,
) -> Result<(Report, String, bool), String> {
    let is_plan = target.extension().is_some_and(|e| e == "simplan")
        || std::fs::read_to_string(target)
            .map(|t| t.trim_start().starts_with('{') && t.contains("\"olang_sim_plan\""))
            .unwrap_or(false);
    #[cfg(feature = "native")]
    crate::output::discard_output();
    if is_plan {
        let plan: PlanFile = cli::load_plan(target)?;
        let file = PathBuf::from(&plan.file);
        let cfg = Config {
            seed: plan.seed,
            run_for: plan.run_for,
            max_steps: plan.max_steps,
            ..Config::default()
        };
        let (report, _) = cli::run_any(&file, cfg, Some(plan.runs.clone()))?;
        let diverged = format!("{:016x}", report.digest()) != plan.digest;
        let title = format!("{} · seed {}", file_name(&file), plan.seed);
        Ok((report, title, diverged))
    } else {
        let seed = seed.unwrap_or(1);
        let cfg = Config {
            seed,
            run_for,
            max_steps: max_steps.unwrap_or(super::DEFAULT_MAX_STEPS),
            ..Config::default()
        };
        let (report, _) = cli::run_any(target, cfg, None)?;
        let title = format!("{} · seed {}", file_name(target), seed);
        Ok((report, title, false))
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

/// `olang sim view TARGET`: the exit code.
pub fn main(
    target: &Path,
    seed: Option<u64>,
    run_for: Option<u64>,
    max_steps: Option<u64>,
    opts: &ViewOptions,
) -> i32 {
    let (report, title, diverged) = match replay_for_view(target, seed, run_for, max_steps) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("olang sim view: {e}");
            return 2;
        }
    };
    if diverged {
        eprintln!(
            "olang sim view: the replay diverged from {} (this olang runs it differently); \
             showing the run as it ran now",
            target.display()
        );
    }
    let m = model(&report, &title);
    if let Some(path) = &opts.json {
        let text = serde_json::to_string(&m).unwrap_or_default();
        if let Err(e) = std::fs::write(path, text + "\n") {
            eprintln!("olang sim view: {}: {e}", path.display());
            return 2;
        }
        println!("olang sim view: wrote {}", path.display());
    }
    let html_path = match (&opts.html, &opts.json) {
        (Some(p), _) => Some(p.clone()),
        (None, Some(_)) => None,
        (None, None) => Some(PathBuf::from(".sim").join(format!("{}.html", report.seed))),
    };
    if let Some(path) = html_path {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            eprintln!("olang sim view: {}: {e}", dir.display());
            return 2;
        }
        if let Err(e) = std::fs::write(&path, page(&m)) {
            eprintln!("olang sim view: {}: {e}", path.display());
            return 2;
        }
        println!("olang sim view: wrote {}", path.display());
        if opts.html.is_none() && !opts.no_open {
            open_in_browser(&path);
        }
    }
    0
}

/// Show the page in the system's browser (nothing where there is none).
fn open_in_browser(path: &Path) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
