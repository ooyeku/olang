//! Faults as a plan (S4): every place a world decides a fault, and a way
//! to force those decisions instead of drawing them.
//!
//! **Sites.** A fault is decided at a *site*: a draw (a crash point at an
//! effect or inside SQLite, a lying sync, a buggify point that fires, a
//! message lost or duplicated) or a call in code (`w.crash`, `w.kill`,
//! `w.stop`, `w.partition`, `w.slow_disk`). A site has a place (a node,
//! or a link for the network's draws), and every visit to it is counted:
//! a visit's *key* is (site, place, virtual time, k), where k numbers the
//! visits to that site and place at that same millisecond. A run records
//! the key of every fault that fired (`Report.plan`).
//!
//! **Forcing.** A world given a [`RunPlan`] still makes every draw it
//! would make (so the streams behind them, sync latencies and tears,
//! advance as they did), but whether a fault fires is the plan's answer:
//! a visit fires when the plan holds an entry for its site and place not
//! yet used whose (t, k) is at or before the visit's. With the plan a run
//! recorded, the run comes back exactly, decision for decision. With
//! entries dropped (the shrinker's work), the run differs from the visit
//! the dropped fault would have struck on, and the remaining entries
//! still fire: at their own instant if the run reaches it, else at the
//! site's first visit after. A call in code that the plan does not list
//! does nothing.
//!
//! A plan also carries the nodes to leave out (a node the world-builder
//! marked `#{ optional: true }`) and how sticky the schedule is (`stick`:
//! the chance a task that yields and could go on keeps the baton; 1 runs
//! each task until it blocks, the fewest preemptions).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Where a fault is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Site {
    /// `#{ crash: #{ p } }`: at an effect, or a write, sync or delete SQLite makes.
    CrashPoint,
    /// `sim.disk(#{ fsync_lies })`: a sync that keeps nothing.
    FsyncLie,
    /// `sim.maybe(point)` fired.
    Maybe,
    /// A message the link lost.
    Loss,
    /// A message the link delivered twice.
    Dup,
    /// `w.crash(n)`.
    Crash,
    /// `w.kill(n)`.
    Kill,
    /// `w.stop(n)`.
    Stop,
    /// `w.partition(a, b)`.
    Partition,
    /// `w.slow_disk(n, f)`.
    SlowDisk,
}

impl Site {
    pub fn name(self) -> &'static str {
        match self {
            Site::CrashPoint => "crash_point",
            Site::FsyncLie => "fsync_lie",
            Site::Maybe => "maybe",
            Site::Loss => "loss",
            Site::Dup => "dup",
            Site::Crash => "crash",
            Site::Kill => "kill",
            Site::Stop => "stop",
            Site::Partition => "partition",
            Site::SlowDisk => "slow_disk",
        }
    }

    pub fn from_name(name: &str) -> Option<Site> {
        Some(match name {
            "crash_point" => Site::CrashPoint,
            "fsync_lie" => Site::FsyncLie,
            "maybe" => Site::Maybe,
            "loss" => Site::Loss,
            "dup" => Site::Dup,
            "crash" => Site::Crash,
            "kill" => Site::Kill,
            "stop" => Site::Stop,
            "partition" => Site::Partition,
            "slow_disk" => Site::SlowDisk,
            _ => return None,
        })
    }

    /// A draw (the seed decides it) rather than a call in code.
    pub fn is_draw(self) -> bool {
        matches!(
            self,
            Site::CrashPoint | Site::FsyncLie | Site::Maybe | Site::Loss | Site::Dup
        )
    }
}

/// A link's place: the sender and the receiver.
pub fn link_place(from: usize, to: usize) -> u64 {
    ((from as u64) << 32) | to as u64
}

/// The visit a fault fired on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub site: Site,
    /// A node's number, or `link_place(from, to)`.
    pub place: u64,
    pub t: u64,
    pub k: u64,
    /// The restart delay it carried (crashes, kills and stops).
    pub delay: Option<u64>,
}

/// One forced fault, as a plan file holds it: the place by name (a node,
/// or `from>to` for a link).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub site: String,
    pub at: String,
    pub t: u64,
    pub k: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<u64>,
    /// What the fault was (for people: `crash`, `fsync_lie`, ...).
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub detail: String,
}

/// The decisions one world run consumes instead of drawing.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RunPlan {
    pub faults: Vec<Entry>,
    /// Optional nodes left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drop: Vec<String>,
    /// The schedule's stickiness (0: as the seed draws it).
    #[serde(default)]
    pub stick: f64,
}

/// What a visit to a site came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub fire: bool,
    pub key: Key,
}

/// A forced entry for one (site, place): (t, k, delay).
type Forced = (u64, u64, Option<u64>);
/// A queue of forced entries for one (site, place).
type Queue = VecDeque<Forced>;

/// Visit counts for every (site, place), and, when the run is forced,
/// the entries still to fire.
#[derive(Debug, Default)]
pub struct Sites {
    counts: HashMap<(Site, u64), (u64, u64)>,
    forced: Option<HashMap<(Site, u64), Queue>>,
}

impl Sites {
    /// Sites that draw (or, for code, always fire).
    pub fn drawn() -> Sites {
        Sites::default()
    }

    /// Sites forced by `entries`, each already resolved to its place.
    pub fn forced(entries: impl IntoIterator<Item = (Site, u64, u64, u64, Option<u64>)>) -> Sites {
        let mut queues: HashMap<(Site, u64), Vec<Forced>> = HashMap::new();
        for (site, place, t, k, delay) in entries {
            queues.entry((site, place)).or_default().push((t, k, delay));
        }
        let forced = queues
            .into_iter()
            .map(|(key, mut v)| {
                v.sort_by_key(|(t, k, _)| (*t, *k));
                (key, v.into_iter().collect::<Queue>())
            })
            .collect();
        Sites {
            counts: HashMap::new(),
            forced: Some(forced),
        }
    }

    pub fn is_forced(&self) -> bool {
        self.forced.is_some()
    }

    /// A visit to `site` at `place` at virtual time `t`; `drawn` is what
    /// the seed said (for code, true), `delay` the restart delay the call
    /// asked. Forced, the plan's answer (and its delay) wins.
    pub fn visit(
        &mut self,
        site: Site,
        place: u64,
        t: u64,
        drawn: bool,
        delay: Option<u64>,
    ) -> Decision {
        let count = self.counts.entry((site, place)).or_insert((t, 0));
        if count.0 != t {
            *count = (t, 0);
        }
        let k = count.1;
        count.1 += 1;
        let mut key = Key {
            site,
            place,
            t,
            k,
            delay,
        };
        let fire = match &mut self.forced {
            None => drawn,
            Some(queues) => match queues.get_mut(&(site, place)) {
                Some(q) if q.front().is_some_and(|(et, ek, _)| (t, k) >= (*et, *ek)) => {
                    let (_, _, d) = q.pop_front().expect("a front");
                    if d.is_some() || delay.is_none() {
                        key.delay = d;
                    }
                    true
                }
                _ => false,
            },
        };
        Decision { fire, key }
    }
}

/// The place's name, for a plan file.
pub fn place_name(site: Site, place: u64, node_name: &dyn Fn(usize) -> String) -> String {
    match site {
        Site::Loss | Site::Dup => {
            let from = (place >> 32) as usize;
            let to = (place & 0xffff_ffff) as usize;
            format!("{}>{}", node_name(from), node_name(to))
        }
        _ => node_name(place as usize),
    }
}

/// A place by name back to its number, with `node` resolving a node's name.
pub fn place_of(site: Site, at: &str, node: &dyn Fn(&str) -> Option<usize>) -> Option<u64> {
    match site {
        Site::Loss | Site::Dup => {
            let (a, b) = at.split_once('>')?;
            Some(link_place(node(a)?, node(b)?))
        }
        _ => node(at).map(|n| n as u64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawn_sites_fire_as_drawn_and_count_visits_per_instant() {
        let mut s = Sites::drawn();
        let a = s.visit(Site::CrashPoint, 1, 10, false, None);
        let b = s.visit(Site::CrashPoint, 1, 10, true, Some(5));
        let c = s.visit(Site::CrashPoint, 1, 11, true, None);
        assert_eq!((a.fire, a.key.k), (false, 0));
        assert_eq!((b.fire, b.key.k, b.key.delay), (true, 1, Some(5)));
        assert_eq!(
            (c.key.t, c.key.k),
            (11, 0),
            "k starts again at a new instant"
        );
    }

    #[test]
    fn a_forced_site_fires_at_its_entry_or_the_first_visit_after() {
        let mut s = Sites::forced([
            (Site::Crash, 2, 100, 1, Some(0)),
            (Site::Crash, 2, 300, 0, None),
        ]);
        assert!(!s.visit(Site::Crash, 2, 100, true, Some(1000)).fire);
        let hit = s.visit(Site::Crash, 2, 100, true, Some(1000));
        assert!(hit.fire);
        assert_eq!(hit.key.delay, Some(0), "the plan's delay wins");
        assert!(
            !s.visit(Site::Crash, 2, 200, true, None).fire,
            "not listed: nothing"
        );
        // the run never visits at 300 exactly: the first visit after fires
        assert!(s.visit(Site::Crash, 2, 450, false, None).fire);
        assert!(!s.visit(Site::Crash, 2, 500, true, None).fire, "used up");
        assert!(
            !s.visit(Site::Crash, 3, 100, true, None).fire,
            "another place"
        );
    }

    #[test]
    fn places_round_trip_through_names() {
        let names = ["world", "kv", "c1"];
        let name = |n: usize| names[n].to_string();
        let node = |t: &str| names.iter().position(|n| *n == t);
        let link = link_place(2, 1);
        assert_eq!(place_name(Site::Loss, link, &name), "c1>kv");
        assert_eq!(place_of(Site::Loss, "c1>kv", &node), Some(link));
        assert_eq!(place_of(Site::Crash, "kv", &node), Some(1));
    }
}
