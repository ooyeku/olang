//! The heart of a world: a virtual clock and a seeded cooperative
//! scheduler. Pure bookkeeping, with no threads in it — `World` (mod.rs)
//! drives it under its lock and hands the baton to whichever task this
//! picks — so every rule here is unit-tested on its own.
//!
//! The rules, which are the whole of S1's determinism:
//!
//! - **Time only moves when nothing can run.** Running code costs no
//!   virtual time. While some task is runnable the clock stands still;
//!   when none is, the clock jumps to the earliest timer and every timer
//!   due then fires, in the order they were set.
//! - **The seed picks who runs next.** At every yield point the runnable
//!   tasks are taken in id order and one is drawn from the scheduler's
//!   own stream (split from the world's seed). The same seed and the same
//!   program make the same draws, so the same interleaving.
//! - **The end is decided here too:** a deadline passed (the next timer
//!   lies past it), every task blocked with no timer left (a deadlock,
//!   with each task's site), or the step budget spent (a livelock guard).

use rand::SeedableRng;
use rand::prelude::*;
use rand::rngs::StdRng;
use std::collections::{BTreeSet, VecDeque};

/// A task in a world: 0 is the world's main program, then each `spawn`
/// in the order it happened.
pub type TaskId = usize;

/// What a blocked task waits for. A wake is a hint, never a promise: the
/// task retries its operation when it runs again and blocks again if it
/// still cannot go on, so a spurious wake costs a step, never a wrong
/// answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// A channel (the world's own number for it) changed: a message
    /// arrived, one was taken, or it closed.
    Chan(u64),
    /// A task ended.
    Task(TaskId),
    /// Only its timer can wake it (`time.sleep`).
    Sleep,
    /// A mailbox of the simulated network (the world's number for it): a
    /// message was delivered to it, or it closed.
    Net(u64),
}

/// What a timer does when it fires: wake a task, or deliver a message
/// the network carried (the world's number for it). Deliveries due at
/// one instant fire in the order they were sent, among the task timers
/// in the order those were set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fire {
    Task(TaskId),
    Deliver(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Runnable,
    Blocked {
        wait: Wait,
        site: String,
        timer: Option<(u64, u64)>,
    },
    Done,
}

/// Why a world stopped before its main program returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// The main program returned (or raised): the world is over.
    MainReturned,
    /// The next thing to happen lies past the run's deadline.
    Deadline,
    /// Every live task is blocked and no timer is set: (task, site).
    Deadlock(Vec<(TaskId, String)>),
    /// More yield points than the budget allows without the run ending.
    StepLimit,
}

impl End {
    /// The outcome's name, as a run reports it.
    pub fn name(&self) -> &'static str {
        match self {
            End::MainReturned => "done",
            End::Deadline => "deadline",
            End::Deadlock(_) => "deadlock",
            End::StepLimit => "step_limit",
        }
    }
}

/// How a blocked task came back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Woke {
    /// Something it waits on changed (or it was simply scheduled).
    Ready,
    /// Its timer fired first.
    TimedOut,
}

pub struct Sched {
    /// Virtual milliseconds since the world began.
    pub now: u64,
    /// Virtual time at which the run stops, if any.
    pub deadline: Option<u64>,
    /// Yield points allowed before the run is called a livelock.
    pub max_steps: u64,
    /// Yield points so far.
    pub steps: u64,
    status: Vec<Status>,
    timed_out: Vec<bool>,
    /// (due, order set, what fires): a BTreeSet is the timer queue,
    /// ordered by due time and then by the order the timers were set.
    timers: BTreeSet<(u64, u64, Fire)>,
    timer_seq: u64,
    /// Deliveries whose time has come, not yet handed to the world.
    due: VecDeque<u64>,
    rng: StdRng,
}

impl Sched {
    /// A scheduler whose draws come from `seed` (already split for this
    /// purpose by the world). Task 0 exists and is runnable.
    pub fn new(seed: u64) -> Sched {
        Sched {
            now: 0,
            deadline: None,
            max_steps: u64::MAX,
            steps: 0,
            status: vec![Status::Runnable],
            timed_out: vec![false],
            timers: BTreeSet::new(),
            timer_seq: 0,
            due: VecDeque::new(),
            rng: StdRng::seed_from_u64(seed),
        }
    }

    /// A new runnable task; its id is the next in spawn order.
    pub fn add_task(&mut self) -> TaskId {
        self.status.push(Status::Runnable);
        self.timed_out.push(false);
        self.status.len() - 1
    }

    pub fn task_count(&self) -> usize {
        self.status.len()
    }

    pub fn is_done(&self, t: TaskId) -> bool {
        matches!(self.status.get(t), Some(Status::Done))
    }

    pub fn is_runnable(&self, t: TaskId) -> bool {
        matches!(self.status.get(t), Some(Status::Runnable))
    }

    /// Block `t` on `wait`, optionally until virtual time `until`.
    pub fn block(&mut self, t: TaskId, wait: Wait, site: String, until: Option<u64>) {
        let timer = until.map(|at| {
            self.timer_seq += 1;
            let key = (at.max(self.now), self.timer_seq);
            self.timers.insert((key.0, key.1, Fire::Task(t)));
            key
        });
        self.timed_out[t] = false;
        self.status[t] = Status::Blocked { wait, site, timer };
    }

    /// Make every task waiting on `wait` runnable again (its timer, if
    /// any, is cancelled).
    pub fn wake(&mut self, wait: Wait) {
        for t in 0..self.status.len() {
            if let Status::Blocked { wait: w, timer, .. } = &self.status[t]
                && *w == wait
            {
                if let Some((at, seq)) = timer {
                    self.timers.remove(&(*at, *seq, Fire::Task(t)));
                }
                self.status[t] = Status::Runnable;
            }
        }
    }

    /// Make `t` runnable whatever it waits on (its timer cancelled): a
    /// crash reaching a node's tasks, so each unwinds at its yield point.
    pub fn kick(&mut self, t: TaskId) {
        if let Some(Status::Blocked { timer, .. }) = self.status.get(t) {
            if let Some((at, seq)) = *timer {
                self.timers.remove(&(at, seq, Fire::Task(t)));
            }
            self.status[t] = Status::Runnable;
        }
    }

    /// A message the network carries arrives at virtual time `at`: when
    /// the clock reaches it, `pick_with`'s `deliver` is called with `id`.
    pub fn deliver_at(&mut self, at: u64, id: u64) {
        self.timer_seq += 1;
        self.timers
            .insert((at.max(self.now), self.timer_seq, Fire::Deliver(id)));
    }

    /// Messages in flight (deliveries not yet due).
    pub fn in_flight(&self) -> usize {
        self.timers
            .iter()
            .filter(|(_, _, f)| matches!(f, Fire::Deliver(_)))
            .count()
            + self.due.len()
    }

    /// `t` has ended; anyone joining it may go on.
    pub fn finish(&mut self, t: TaskId) {
        if let Status::Blocked {
            timer: Some((at, seq)),
            ..
        } = &self.status[t]
        {
            self.timers.remove(&(*at, *seq, Fire::Task(t)));
        }
        self.status[t] = Status::Done;
        self.wake(Wait::Task(t));
    }

    /// How `t` came back from its last block (read once it runs again).
    pub fn woke(&mut self, t: TaskId) -> Woke {
        if std::mem::take(&mut self.timed_out[t]) {
            Woke::TimedOut
        } else {
            Woke::Ready
        }
    }

    /// The blocked tasks and what each waits on, in id order.
    pub fn blocked_sites(&self) -> Vec<(TaskId, String)> {
        self.status
            .iter()
            .enumerate()
            .filter_map(|(t, s)| match s {
                Status::Blocked { site, .. } => Some((t, site.clone())),
                _ => None,
            })
            .collect()
    }

    /// One yield point: choose the task that runs next. When nothing is
    /// runnable, time jumps to the next timer; when nothing ever can be,
    /// the run ends.
    pub fn pick(&mut self) -> Result<TaskId, End> {
        self.pick_with(&mut |_, _| {})
    }

    /// `pick`, with the network: each delivery that falls due is handed
    /// to `deliver(self, id)`, which puts the message where it goes and
    /// wakes whoever waits for it (and may send more: a refusal). The
    /// world drives `next` itself, since its `deliver` needs the whole
    /// world; this is the same loop for tests.
    pub fn pick_with(&mut self, deliver: &mut dyn FnMut(&mut Sched, u64)) -> Result<TaskId, End> {
        let mut counted = true;
        loop {
            match self.next(counted)? {
                Pick::Run(t) => return Ok(t),
                Pick::Deliver(id) => {
                    counted = false;
                    deliver(self, id);
                }
            }
        }
    }

    /// One turn of the loop under `pick`: a task to run, or a delivery
    /// that fell due and must be handled before anyone is chosen (it may
    /// make a task runnable). `counted` is false for the calls that
    /// continue one pick after a delivery, so a pick costs one step.
    ///
    /// Deliveries due at an instant are all handed out before a task is
    /// drawn, so the tasks they wake are in the draw.
    pub fn next(&mut self, counted: bool) -> Result<Pick, End> {
        if counted {
            self.steps += 1;
            if self.steps > self.max_steps {
                return Err(End::StepLimit);
            }
        }
        loop {
            if let Some(id) = self.due.pop_front() {
                return Ok(Pick::Deliver(id));
            }
            let runnable: Vec<TaskId> = (0..self.status.len())
                .filter(|t| self.is_runnable(*t))
                .collect();
            if !runnable.is_empty() {
                let i = if runnable.len() == 1 {
                    0
                } else {
                    self.rng.random_range(0..runnable.len())
                };
                return Ok(Pick::Run(runnable[i]));
            }
            let Some(&(at, _, _)) = self.timers.iter().next() else {
                return Err(End::Deadlock(self.blocked_sites()));
            };
            if let Some(deadline) = self.deadline
                && at > deadline
            {
                self.now = self.now.max(deadline);
                return Err(End::Deadline);
            }
            self.now = self.now.max(at);
            // Every timer due now fires, in the order it was set.
            while let Some(&(due, seq, fire)) = self.timers.iter().next() {
                if due > self.now {
                    break;
                }
                self.timers.remove(&(due, seq, fire));
                match fire {
                    Fire::Task(t) => {
                        self.status[t] = Status::Runnable;
                        self.timed_out[t] = true;
                    }
                    Fire::Deliver(id) => self.due.push_back(id),
                }
            }
        }
    }
}

/// What `Sched::next` hands back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Run(TaskId),
    Deliver(u64),
}

/// splitmix64: one well-mixed 64-bit value from another. Seeds for the
/// world's separate streams (the scheduler's, each task's) are split
/// from the world's seed with it, so the streams are independent and a
/// change to one (a task drawing one more number) never moves another.
pub fn split(seed: u64, stream: u64) -> u64 {
    let mut z = seed ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_runnable_task_runs_without_a_draw() {
        let mut s = Sched::new(1);
        assert_eq!(s.pick(), Ok(0));
        assert_eq!(s.now, 0);
    }

    #[test]
    fn sleeping_advances_the_clock_instantly_and_in_order() {
        let mut s = Sched::new(7);
        let a = s.add_task();
        let b = s.add_task();
        s.block(0, Wait::Sleep, "sleep".into(), Some(3_600_000));
        s.block(a, Wait::Sleep, "sleep".into(), Some(500));
        s.block(b, Wait::Sleep, "sleep".into(), Some(500));
        // both due at 500: the clock jumps there, and both fire
        let first = s.pick().unwrap();
        assert_eq!(s.now, 500);
        assert!(first == a || first == b);
        assert!(s.is_runnable(a) && s.is_runnable(b));
        assert_eq!(s.woke(a), Woke::TimedOut);
        s.finish(a);
        s.finish(b);
        // then an hour, at once
        assert_eq!(s.pick(), Ok(0));
        assert_eq!(s.now, 3_600_000);
    }

    #[test]
    fn time_stands_still_while_anything_can_run() {
        let mut s = Sched::new(3);
        let a = s.add_task();
        s.block(a, Wait::Sleep, "sleep".into(), Some(10));
        for _ in 0..100 {
            assert_eq!(s.pick(), Ok(0));
        }
        assert_eq!(s.now, 0);
    }

    #[test]
    fn a_wake_cancels_the_timer() {
        let mut s = Sched::new(3);
        let a = s.add_task();
        s.block(a, Wait::Chan(1), "recv".into(), Some(100));
        s.wake(Wait::Chan(1));
        assert_eq!(s.woke(a), Woke::Ready);
        s.finish(0);
        assert_eq!(s.pick(), Ok(a));
        s.finish(a);
        // the cancelled timer does not move the clock
        assert!(matches!(s.pick(), Err(End::Deadlock(_))));
        assert_eq!(s.now, 0);
    }

    #[test]
    fn everyone_blocked_without_a_timer_is_a_deadlock_naming_each_site() {
        let mut s = Sched::new(9);
        let a = s.add_task();
        s.block(0, Wait::Task(a), "task.join on task 1".into(), None);
        s.block(a, Wait::Chan(2), "chan.recv on channel #2".into(), None);
        match s.pick() {
            Err(End::Deadlock(sites)) => assert_eq!(
                sites,
                vec![
                    (0, "task.join on task 1".to_string()),
                    (1, "chan.recv on channel #2".to_string())
                ]
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_deadline_stops_the_clock_at_the_deadline() {
        let mut s = Sched::new(2);
        s.deadline = Some(1_000);
        s.block(0, Wait::Sleep, "sleep".into(), Some(5_000));
        assert_eq!(s.pick(), Err(End::Deadline));
        assert_eq!(s.now, 1_000);
    }

    #[test]
    fn the_step_budget_ends_a_livelock() {
        let mut s = Sched::new(2);
        s.max_steps = 5;
        for _ in 0..5 {
            assert!(s.pick().is_ok());
        }
        assert_eq!(s.pick(), Err(End::StepLimit));
    }

    #[test]
    fn finishing_wakes_joiners() {
        let mut s = Sched::new(2);
        let a = s.add_task();
        s.block(0, Wait::Task(a), "join".into(), None);
        assert_eq!(s.pick(), Ok(a));
        s.finish(a);
        assert_eq!(s.pick(), Ok(0));
        assert!(s.is_done(a));
    }

    fn schedule(seed: u64) -> Vec<TaskId> {
        let mut s = Sched::new(seed);
        for _ in 0..4 {
            s.add_task();
        }
        (0..64).map(|_| s.pick().unwrap()).collect()
    }

    #[test]
    fn one_seed_one_schedule_and_seeds_differ() {
        assert_eq!(schedule(42), schedule(42));
        assert_ne!(schedule(42), schedule(43));
        // every runnable task gets picked at some point
        let picked: BTreeSet<TaskId> = schedule(5).into_iter().collect();
        assert_eq!(picked.len(), 5);
    }

    #[test]
    fn a_delivery_wakes_its_receiver_when_the_clock_reaches_it() {
        let mut s = Sched::new(1);
        s.block(0, Wait::Net(7), "recv".into(), Some(1_000));
        s.deliver_at(40, 99);
        let mut seen = Vec::new();
        let picked = s.pick_with(&mut |sched, id| {
            seen.push((sched.now, id));
            sched.wake(Wait::Net(7));
        });
        assert_eq!(picked, Ok(0));
        assert_eq!(seen, vec![(40, 99)]);
        assert_eq!(s.now, 40);
        assert_eq!(s.woke(0), Woke::Ready, "the delivery came first");
        assert_eq!(s.in_flight(), 0);
    }

    #[test]
    fn deliveries_alone_keep_a_world_from_deadlocking_and_fire_in_send_order() {
        let mut s = Sched::new(1);
        s.block(0, Wait::Net(1), "recv".into(), None);
        s.deliver_at(10, 1);
        s.deliver_at(10, 2);
        s.deliver_at(5, 3);
        let mut seen = Vec::new();
        // nothing wakes task 0 until the third delivery
        let picked = s.pick_with(&mut |sched, id| {
            seen.push(id);
            if seen.len() == 3 {
                sched.wake(Wait::Net(1));
            }
        });
        assert_eq!(picked, Ok(0));
        assert_eq!(seen, vec![3, 1, 2]);
        assert_eq!(s.now, 10);
    }

    #[test]
    fn a_kick_makes_a_blocked_task_runnable_and_cancels_its_timer() {
        let mut s = Sched::new(1);
        let a = s.add_task();
        s.block(a, Wait::Sleep, "sleep".into(), Some(500));
        s.block(0, Wait::Task(a), "join".into(), None);
        s.kick(a);
        assert_eq!(s.pick(), Ok(a));
        assert_eq!(s.woke(a), Woke::Ready);
        s.finish(a);
        assert_eq!(s.pick(), Ok(0));
        assert_eq!(s.now, 0, "the kicked task's timer is gone");
    }

    #[test]
    fn split_streams_are_distinct_and_stable() {
        assert_eq!(split(1, 0), split(1, 0));
        assert_ne!(split(1, 0), split(1, 1));
        assert_ne!(split(1, 0), split(2, 0));
    }
}
