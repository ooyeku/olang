//! The simulated network's model (S2): which nodes can reach which, and
//! what happens to one message on a link. Pure bookkeeping, like
//! `sched.rs`: the world (mod.rs) asks it at every send and every
//! delivery, under its lock, and every rule here is unit-tested alone.
//!
//! - **A link** (an unordered pair of nodes) has a latency range, a loss
//!   probability and a duplication probability. Links not set take the
//!   world's default (`sim.world(#{ net: ... })`). A node talking to
//!   itself (`localhost`) is a loopback: no latency, no loss.
//! - **A partition** cuts every pair across two groups until `heal`. A
//!   message is checked when it is sent and again when it would arrive,
//!   so a partition that begins while a message is in flight drops it:
//!   the request that reached the server and the reply that never came
//!   back, the case a retry must be ready for.
//! - **Draws** come from the network's own stream (split from the seed,
//!   apart from the scheduler's and every task's), and every message
//!   sent over a link takes the same four draws whatever happens to it,
//!   so one more lost message never shifts another's latency.

use rand::SeedableRng;
use rand::prelude::*;
use rand::rngs::StdRng;
use std::collections::{BTreeMap, BTreeSet};

/// A node of the world: 0 is the world itself (its coordinator and its
/// hooks), then each node in the order it was added.
pub type NodeId = usize;

/// What one link does to the messages that cross it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Link {
    /// Latency in virtual ms, drawn uniformly from `min..=max`.
    pub latency: (u64, u64),
    /// The chance a message is lost.
    pub loss: f64,
    /// The chance a message arrives twice (each copy its own latency).
    pub dup: f64,
}

impl Default for Link {
    /// A quiet LAN: 1–10 ms, nothing lost or duplicated.
    fn default() -> Link {
        Link {
            latency: (1, 10),
            loss: 0.0,
            dup: 0.0,
        }
    }
}

impl Link {
    /// Check a link's numbers: a latency range in order, probabilities
    /// in 0..=1.
    pub fn validate(&self) -> Result<(), String> {
        if self.latency.0 > self.latency.1 {
            return Err(format!(
                "latency ({}, {}) runs backwards: the least first",
                self.latency.0, self.latency.1
            ));
        }
        for (name, p) in [("loss", self.loss), ("dup", self.dup)] {
            if !(0.0..=1.0).contains(&p) || p.is_nan() {
                return Err(format!("`{name}` is a probability in 0..1, got {p}"));
            }
        }
        Ok(())
    }
}

/// Why a message did not arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drop {
    /// The two ends were on different sides of a partition.
    Partition,
    /// The link lost it.
    Loss,
}

impl Drop {
    pub fn name(self) -> &'static str {
        match self {
            Drop::Partition => "partition",
            Drop::Loss => "loss",
        }
    }
}

/// What the link does with one message: lost, or delivered after each
/// of these delays (two when it was duplicated).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transmit {
    Dropped(Drop),
    Deliver(Vec<u64>),
}

#[derive(Debug)]
pub struct Net {
    pub default: Link,
    links: BTreeMap<(NodeId, NodeId), Link>,
    cut: BTreeSet<(NodeId, NodeId)>,
    rng: StdRng,
}

fn pair(a: NodeId, b: NodeId) -> (NodeId, NodeId) {
    if a <= b { (a, b) } else { (b, a) }
}

impl Net {
    /// A network whose draws come from `seed` (already split for this
    /// purpose by the world).
    pub fn new(seed: u64, default: Link) -> Net {
        Net {
            default,
            links: BTreeMap::new(),
            cut: BTreeSet::new(),
            rng: StdRng::seed_from_u64(seed),
        }
    }

    /// Set the link between `a` and `b` (both directions).
    pub fn set_link(&mut self, a: NodeId, b: NodeId, link: Link) {
        self.links.insert(pair(a, b), link);
    }

    pub fn link(&self, a: NodeId, b: NodeId) -> Link {
        self.links.get(&pair(a, b)).copied().unwrap_or(self.default)
    }

    /// Cut every pair with one end in `left` and the other in `right`.
    /// Partitions add up until `heal`.
    pub fn partition(&mut self, left: &[NodeId], right: &[NodeId]) {
        for &a in left {
            for &b in right {
                if a != b {
                    self.cut.insert(pair(a, b));
                }
            }
        }
    }

    /// Every partition is gone.
    pub fn heal(&mut self) {
        self.cut.clear();
    }

    pub fn is_partitioned(&self) -> bool {
        !self.cut.is_empty()
    }

    /// Can a message from `a` reach `b` right now?
    pub fn connected(&self, a: NodeId, b: NodeId) -> bool {
        a == b || !self.cut.contains(&pair(a, b))
    }

    /// One message from `a` to `b`: what happens to it. A loopback takes
    /// no draws; a cut link takes none either (nothing crosses it).
    pub fn transmit(&mut self, a: NodeId, b: NodeId) -> Transmit {
        self.transmit_with(a, b, &mut |_, lost, dup| (lost, dup))
    }

    /// `transmit`, with what the link drew (lost, duplicated) handed to
    /// `decide`, which answers what happens (a plan's forced decisions:
    /// `plan.rs`). The draws are the same either way.
    pub fn transmit_with(
        &mut self,
        a: NodeId,
        b: NodeId,
        decide: &mut dyn FnMut(Link, bool, bool) -> (bool, bool),
    ) -> Transmit {
        if a == b {
            return Transmit::Deliver(vec![0]);
        }
        if !self.connected(a, b) {
            return Transmit::Dropped(Drop::Partition);
        }
        let link = self.link(a, b);
        // Always the same four draws, in the same order.
        let lost = self.rng.random::<f64>() < link.loss;
        let first = self.latency(link);
        let dup = self.rng.random::<f64>() < link.dup;
        let second = self.latency(link);
        let (lost, dup) = decide(link, lost, dup);
        if lost {
            return Transmit::Dropped(Drop::Loss);
        }
        if dup {
            Transmit::Deliver(vec![first, second])
        } else {
            Transmit::Deliver(vec![first])
        }
    }

    fn latency(&mut self, link: Link) -> u64 {
        let (lo, hi) = link.latency;
        if lo == hi {
            // still a draw: the stream advances the same either way
            let _ = self.rng.random::<u64>();
            lo
        } else {
            self.rng.random_range(lo..=hi)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lossy(loss: f64, dup: f64) -> Link {
        Link {
            latency: (5, 50),
            loss,
            dup,
        }
    }

    #[test]
    fn a_quiet_link_delivers_everything_within_its_latency() {
        let mut net = Net::new(1, Link::default());
        for _ in 0..1000 {
            match net.transmit(1, 2) {
                Transmit::Deliver(d) => {
                    assert_eq!(d.len(), 1);
                    assert!((1..=10).contains(&d[0]), "{d:?}");
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn loss_and_duplication_happen_about_as_often_as_asked() {
        let mut net = Net::new(7, lossy(0.1, 0.05));
        let (mut lost, mut doubled) = (0, 0);
        for _ in 0..10_000 {
            match net.transmit(1, 2) {
                Transmit::Dropped(Drop::Loss) => lost += 1,
                Transmit::Deliver(d) if d.len() == 2 => doubled += 1,
                Transmit::Deliver(_) => {}
                other => panic!("{other:?}"),
            }
        }
        assert!((800..1200).contains(&lost), "lost {lost}");
        assert!((300..650).contains(&doubled), "doubled {doubled}");
    }

    #[test]
    fn one_seed_one_network() {
        let run = |seed| {
            let mut net = Net::new(seed, lossy(0.2, 0.2));
            (0..200).map(|_| net.transmit(1, 2)).collect::<Vec<_>>()
        };
        assert_eq!(run(3), run(3));
        assert_ne!(run(3), run(4));
    }

    #[test]
    fn a_partition_cuts_across_and_heal_restores() {
        let mut net = Net::new(1, Link::default());
        net.partition(&[1], &[2, 3]);
        assert_eq!(net.transmit(1, 2), Transmit::Dropped(Drop::Partition));
        assert_eq!(net.transmit(3, 1), Transmit::Dropped(Drop::Partition));
        assert!(net.connected(2, 3), "the same side still talks");
        assert!(matches!(net.transmit(2, 3), Transmit::Deliver(_)));
        assert!(net.connected(1, 1));
        net.heal();
        assert!(!net.is_partitioned());
        assert!(matches!(net.transmit(1, 2), Transmit::Deliver(_)));
    }

    #[test]
    fn a_loopback_is_instant_and_never_lost() {
        let mut net = Net::new(1, lossy(1.0, 0.0));
        net.partition(&[1], &[2]);
        assert_eq!(net.transmit(1, 1), Transmit::Deliver(vec![0]));
    }

    #[test]
    fn a_link_overrides_the_default_in_both_directions() {
        let mut net = Net::new(1, Link::default());
        net.set_link(
            3,
            1,
            Link {
                latency: (100, 100),
                loss: 0.0,
                dup: 0.0,
            },
        );
        assert_eq!(net.transmit(1, 3), Transmit::Deliver(vec![100]));
        assert_eq!(net.transmit(3, 1), Transmit::Deliver(vec![100]));
        assert_eq!(net.link(1, 2), Link::default());
    }

    #[test]
    fn a_lost_message_takes_the_same_draws_as_a_delivered_one() {
        // Two networks, one seed: one link always loses, the other
        // never. The next message on a third, quiet link draws the
        // same latency in both.
        let mut a = Net::new(9, Link::default());
        let mut b = Net::new(9, Link::default());
        a.set_link(1, 2, lossy(1.0, 0.0));
        b.set_link(1, 2, lossy(0.0, 0.0));
        assert_eq!(a.transmit(1, 2), Transmit::Dropped(Drop::Loss));
        assert!(matches!(b.transmit(1, 2), Transmit::Deliver(_)));
        assert_eq!(a.transmit(1, 3), b.transmit(1, 3));
    }

    #[test]
    fn links_are_validated() {
        assert!(lossy(0.5, 0.5).validate().is_ok());
        assert!(lossy(1.5, 0.0).validate().is_err());
        assert!(
            Link {
                latency: (9, 3),
                ..Link::default()
            }
            .validate()
            .is_err()
        );
    }
}
