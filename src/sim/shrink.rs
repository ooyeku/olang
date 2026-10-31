//! The shrinker (S4): a failing run, made as small as it will go while it
//! still fails the same way.
//!
//! A run's faults are a plan (`plan.rs`): every crash, lying sync,
//! buggify point, lost or duplicated message, kill, stop, partition and
//! slow disk, keyed by the visit it fired on. Forced by that plan the run
//! comes back exactly; forced by less of it, it is another run of the
//! same world, and the question is only whether it still fails *the same
//! way* (`Report::signature`: the same property, or the same outcome and
//! error once its numbers are set aside).
//!
//! In order, each step keeping a change only when the failure keeps its
//! signature:
//!
//! 1. **Faults**, by delta debugging (Zeller's ddmin): try the plan with
//!    none; then drop halves, quarters, ... and finally single faults,
//!    keeping any complement that still fails. The result is 1-minimal:
//!    no single fault can be dropped. With `--jobs`, the candidates of a
//!    round run side by side and the first in order that fails is kept,
//!    so the result is the same for any number of jobs.
//! 2. **Nodes**: each node the world-builder marked `optional` is left
//!    out, one at a time.
//! 3. **Delays**: each crash's, kill's or stop's restart delay is tried
//!    at 0, a quarter and a half of itself.
//! 4. **The schedule**: a stickier scheduler (a task that yields and
//!    could go on keeps the baton: always, then 95%, 75%, 50% of the
//!    time) means fewer preemptions; the stickiest that still fails is
//!    kept.
//!
//! Finally the plan is replaced by what the last run actually fired (an
//! entry that never matched a visit is dropped) and run once more, to
//! check that it reproduces the failure digest for digest. Each step is a
//! run of the whole world, so the work is bounded (`budget` runs).

use super::cli::{Request, SeedResult};
use super::plan::{Entry, RunPlan};

/// Runs the shrinker may spend unless told otherwise.
pub const DEFAULT_BUDGET: usize = 300;

/// What shrinking came to.
#[derive(Debug, Clone)]
pub struct Shrunk {
    pub seed: u64,
    /// Faults in the failing run before, and after.
    pub before: usize,
    pub after: usize,
    /// The plan of every world run of the file; `index` is the failing one.
    pub plans: Vec<RunPlan>,
    pub index: usize,
    /// The minimal run.
    pub result: SeedResult,
    /// Runs spent.
    pub runs: usize,
    pub preemptions_before: u64,
    /// The budget ran out before every step was done.
    pub cut_short: bool,
}

impl Shrunk {
    /// `shrunk: 11 faults → 2 · 41 runs · preemptions 1,203 → 15 · left out: c3`.
    pub fn summary(&self) -> String {
        let mut text = format!(
            "shrunk: {} fault{} → {} · {} runs",
            self.before,
            if self.before == 1 { "" } else { "s" },
            self.after,
            self.runs
        );
        let plan = &self.plans[self.index];
        if plan.stick > 0.0 {
            text.push_str(&format!(
                " · preemptions {} → {} (schedule stick {})",
                self.preemptions_before, self.result.preemptions, plan.stick
            ));
        }
        if !plan.drop.is_empty() {
            text.push_str(&format!(" · left out: {}", plan.drop.join(", ")));
        }
        if self.cut_short {
            text.push_str(" · budget spent (not minimal)");
        }
        text
    }

    /// The faults left, one line each.
    pub fn listing(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.plans[self.index]
            .faults
            .iter()
            .map(|e| {
                let delay = match e.delay {
                    Some(ms) if !e.detail.contains("restart after") => {
                        format!(" (restart after {ms} ms)")
                    }
                    _ => String::new(),
                };
                format!(
                    "{:>14}  {:<10} {:<8} {}{delay}",
                    super::cli::fmt_virtual(e.t),
                    if e.kind.is_empty() { &e.site } else { &e.kind },
                    e.at,
                    e.detail
                )
            })
            .collect();
        lines.push(format!(
            "then: {}",
            self.result.failure.clone().unwrap_or_default()
        ));
        lines
    }
}

/// Shrink `seed`'s failure. `run` runs a batch of requests (side by side
/// when it can) and answers in order; `batch` is how many to hand it at
/// once; `start` is a plan to start from (a saved one), else the seed's
/// own drawn run.
pub fn shrink(
    seed: u64,
    run: &dyn Fn(&[Request]) -> Vec<SeedResult>,
    budget: usize,
) -> Result<Shrunk, String> {
    shrink_from(seed, run, budget, None, 1)
}

pub fn shrink_from(
    seed: u64,
    run: &dyn Fn(&[Request]) -> Vec<SeedResult>,
    budget: usize,
    start: Option<Vec<RunPlan>>,
    batch: usize,
) -> Result<Shrunk, String> {
    let one = |plans: Option<Vec<RunPlan>>, want_plans: bool| -> SeedResult {
        run(&[Request {
            seed,
            plans,
            want_plans,
        }])
        .pop()
        .unwrap_or_else(|| SeedResult {
            seed,
            failed: false,
            outcome: "error".into(),
            now: 0,
            steps: 0,
            failure: None,
            property: None,
            broke_at: None,
            signature: String::new(),
            digest: String::new(),
            faults: 0,
            preemptions: 0,
            run_index: 0,
            optional: Vec::new(),
            plans: None,
            error: Some("no answer".into()),
        })
    };
    let first = one(start, true);
    if let Some(e) = &first.error {
        return Err(e.clone());
    }
    if !first.failed {
        return Err(format!("seed {seed} does not fail"));
    }
    let signature = first.signature.clone();
    let index = first.run_index;
    let mut plans = first.plans.clone().unwrap_or_default();
    if plans.len() <= index {
        plans.resize(index + 1, RunPlan::default());
    }
    let before = plans[index].faults.len();
    let mut runs = 1usize;
    // The plan must bring the failure back as it was, or there is
    // nothing sound to shrink.
    let again = one(Some(plans.clone()), false);
    runs += 1;
    if !(again.failed && again.signature == signature && again.digest == first.digest) {
        return Err(format!(
            "the run does not come back from its own plan (digest {} then {}; {} then {}): \
             something in it is not deterministic",
            first.digest, again.digest, first.signature, again.signature
        ));
    }
    let cut_short = std::cell::Cell::new(false);
    // Try candidates in order (a batch at a time); the first that still
    // fails the same way.
    let trial = |cands: &[Vec<RunPlan>], runs: &mut usize| -> Option<usize> {
        for (b, chunk) in cands.chunks(batch.max(1)).enumerate() {
            if *runs + chunk.len() > budget {
                cut_short.set(true);
                return None;
            }
            let reqs: Vec<Request> = chunk
                .iter()
                .map(|p| Request {
                    seed,
                    plans: Some(p.clone()),
                    want_plans: false,
                })
                .collect();
            let results = run(&reqs);
            *runs += chunk.len();
            if let Some(i) = results
                .iter()
                .position(|r| r.failed && r.signature == signature)
            {
                return Some(b * batch.max(1) + i);
            }
        }
        None
    };
    let with_faults = |plans: &[RunPlan], faults: Vec<Entry>| -> Vec<RunPlan> {
        let mut p = plans.to_vec();
        p[index].faults = faults;
        p
    };

    // 1. faults: ddmin
    let mut faults = plans[index].faults.clone();
    if !faults.is_empty() {
        if trial(&[with_faults(&plans, Vec::new())], &mut runs).is_some() {
            faults.clear();
        } else {
            let mut n = 2usize;
            while faults.len() >= 2 {
                let size = faults.len().div_ceil(n);
                let chunks: Vec<std::ops::Range<usize>> = (0..faults.len())
                    .step_by(size)
                    .map(|a| a..(a + size).min(faults.len()))
                    .collect();
                let complements: Vec<Vec<Entry>> = chunks
                    .iter()
                    .map(|r| {
                        faults
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| !r.contains(i))
                            .map(|(_, e)| e.clone())
                            .collect()
                    })
                    .collect();
                let cands: Vec<Vec<RunPlan>> = complements
                    .iter()
                    .map(|c| with_faults(&plans, c.clone()))
                    .collect();
                match trial(&cands, &mut runs) {
                    Some(i) => {
                        faults = complements[i].clone();
                        n = (n - 1).max(2);
                    }
                    None if cut_short.get() => break,
                    None if n >= faults.len() => break,
                    None => n = (n * 2).min(faults.len()),
                }
            }
        }
        plans[index].faults = faults;
    }

    // 2. optional nodes, one at a time
    for node in first.optional.clone() {
        if cut_short.get() || plans[index].drop.contains(&node) {
            continue;
        }
        let mut p = plans.clone();
        p[index].drop.push(node);
        if trial(&[p.clone()], &mut runs).is_some() {
            plans = p;
        }
    }

    // 3. delays
    for i in 0..plans[index].faults.len() {
        let Some(d) = plans[index].faults[i].delay.filter(|d| *d > 0) else {
            continue;
        };
        let cands: Vec<Vec<RunPlan>> = [0, d / 4, d / 2]
            .into_iter()
            .filter(|x| *x < d)
            .map(|x| {
                let mut p = plans.clone();
                p[index].faults[i].delay = Some(x);
                p
            })
            .collect();
        if cut_short.get() {
            break;
        }
        if let Some(k) = trial(&cands, &mut runs) {
            plans = cands[k].clone();
        }
    }

    // 4. the schedule
    if !cut_short.get() && plans[index].stick < 1.0 {
        let cands: Vec<Vec<RunPlan>> = [1.0, 0.95, 0.75, 0.5]
            .into_iter()
            .filter(|s| *s > plans[index].stick)
            .map(|s| {
                let mut p = plans.clone();
                p[index].stick = s;
                p
            })
            .collect();
        if let Some(k) = trial(&cands, &mut runs) {
            plans = cands[k].clone();
        }
    }

    // What the last run fired is the plan; run it once more to check it
    // reproduces the failure exactly.
    let last = one(Some(plans.clone()), true);
    runs += 1;
    if !(last.failed && last.signature == signature) {
        return Err(format!(
            "the shrunk plan no longer fails the same way ({})",
            last.signature
        ));
    }
    if let Some(fired) = last.plans.as_ref().and_then(|p| p.get(index)) {
        let mut p = plans.clone();
        p[index].faults = fired.faults.clone();
        let check = one(Some(p.clone()), false);
        runs += 1;
        if check.digest == last.digest {
            plans = p;
        }
    }
    let result = one(Some(plans.clone()), false);
    runs += 1;
    Ok(Shrunk {
        seed,
        before,
        after: plans[index].faults.len(),
        plans,
        index,
        result,
        runs,
        preemptions_before: first.preemptions,
        cut_short: cut_short.get(),
    })
}
