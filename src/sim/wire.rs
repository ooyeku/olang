//! The world's network (S2): mailboxes, listeners, messages in flight,
//! and what happens when one arrives. `net.rs` decides what a link does
//! to a message; this moves it: a send asks the link, schedules each copy
//! as a delivery on the scheduler's timer queue, and the delivery puts it
//! in a mailbox and wakes whoever waits there.
//!
//! **Mailboxes** are the one waiting point: a client waiting for its
//! reply, a server's workers waiting for requests, `http.serve` itself
//! waiting to be shut down. A task blocks on `Wait::Net(box)`; a delivery
//! (or a close) wakes it, and it retries, as every S1 wait does.
//!
//! **A request** is a message from the client's node to (server, port)
//! carrying the reply mailbox. At the server it lands in the listener's
//! queue (or is answered 503 when the queue is full, 413 when the body is
//! too large, or refused when nothing listens there). The response is a
//! message back. A reply arriving after the client stopped waiting is
//! `late` and dropped, as a socket closed by a timed-out client drops it.

use super::net::{NodeId, Transmit};
use super::sched::{TaskId, Wait};
use super::{Ended, State, World};
use std::collections::{BTreeMap, VecDeque};

/// The coordinator's mailbox: woken whenever a task ends.
pub(super) const COORD_BOX: u64 = 0;

/// An HTTP request in the world.
#[derive(Debug, Clone)]
pub(crate) struct Req {
    /// The world's number for it: 1, 2, ... in the order sent.
    pub id: u64,
    pub from: NodeId,
    pub port: u16,
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The client's mailbox for the answer.
    pub reply: u64,
    /// `node:port` of the client, as `req.remote_addr` reads it.
    pub remote: String,
}

/// An HTTP response in the world.
#[derive(Debug, Clone)]
pub(crate) struct Resp {
    pub status: i64,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) enum Body {
    Request(Req),
    Response {
        req: u64,
        resp: Resp,
    },
    /// Nothing listened at the port (or the node was down): the
    /// connection was refused.
    Refused {
        req: u64,
        why: String,
    },
}

#[derive(Debug, Clone)]
struct Msg {
    from: NodeId,
    to: NodeId,
    /// The mailbox it goes to, for answers (a request goes to a port).
    reply: u64,
    body: Body,
    /// Virtual ms it was handed to the network (S5: the view's arrows).
    sent: u64,
}

struct Listener {
    queue: u64,
    /// What `http.serve` itself waits on until the server shuts down.
    ctl: u64,
    capacity: usize,
    max_body: usize,
}

/// A request a handler deferred (`http.defer`), waiting for
/// `http.respond`.
pub(crate) struct Parked {
    pub node: NodeId,
    pub req: Req,
}

/// What the network did, in counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetStats {
    /// Messages handed to the network (each copy of a duplicate once).
    pub sent: u64,
    pub delivered: u64,
    /// Lost by a link.
    pub lost: u64,
    /// Cut by a partition (when sent, or while in flight).
    pub cut: u64,
    pub duplicated: u64,
    /// Replies that arrived after their client stopped waiting.
    pub late: u64,
    /// Requests to a port nothing listened on, or to a node that was down.
    pub refused: u64,
    /// Requests that waited out their timeout.
    pub timeouts: u64,
}

/// One request a node made, from send to answer: what S4's
/// linearizability checker reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    pub id: u64,
    pub node: String,
    pub method: String,
    pub url: String,
    pub body: String,
    /// Virtual ms it was sent, and answered (or given up on).
    pub sent: u64,
    pub done: Option<u64>,
    /// "pending", "ok", "timeout", "refused".
    pub outcome: &'static str,
    pub status: Option<i64>,
    pub response: Option<String>,
}

#[derive(Default)]
pub(super) struct Wire {
    boxes: BTreeMap<u64, VecDeque<Body>>,
    listeners: BTreeMap<(NodeId, u16), Listener>,
    in_flight: BTreeMap<u64, Msg>,
    next_box: u64,
    next_msg: u64,
    next_req: u64,
    pub(super) parked: BTreeMap<u64, Parked>,
    pub(super) held: BTreeMap<(NodeId, String), Vec<u64>>,
    pub(super) stats: NetStats,
    pub(super) history: Vec<History>,
    /// Requests by id → their index in `history`.
    open_history: BTreeMap<u64, usize>,
}

fn short(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.chars().count() > 80 {
        let cut: String = text.chars().take(77).collect();
        format!("{cut}...")
    } else {
        text.into_owned()
    }
}

impl World {
    pub(super) fn node_name(&self, n: NodeId) -> &str {
        &self.nodes[n].name
    }

    /// The node a host name names, if any.
    pub(super) fn node_named(&self, name: &str) -> Option<NodeId> {
        self.nodes
            .iter()
            .position(|n| n.name.eq_ignore_ascii_case(name))
            .filter(|&n| n > 0 || self.nodes.len() == 1)
    }

    /// The world's wall clock as `node` reads it (its skew included).
    pub(super) fn wall_ms(&self, node: NodeId) -> i64 {
        self.start_ms + self.now() as i64 + self.nodes[node].skew_ms
    }

    /// A new mailbox.
    pub(super) fn open_box(&self) -> u64 {
        let mut st = self.lock();
        Self::open_box_in(&mut st)
    }

    fn open_box_in(st: &mut State) -> u64 {
        st.wire.next_box += 1;
        let id = st.wire.next_box;
        st.wire.boxes.insert(id, VecDeque::new());
        id
    }

    /// Close a mailbox: what still arrives for it is late.
    pub(super) fn close_box(&self, id: u64) {
        let mut st = self.lock();
        st.wire.boxes.remove(&id);
    }

    /// Hand `body` to the network, from `from` to `to`.
    fn transmit(&self, st: &mut State, from: NodeId, to: NodeId, reply: u64, body: Body) {
        let what = describe(&body);
        // Loss and duplication are fault sites (`plan.rs`): what the link
        // drew, or what a forced plan says.
        let now = st.sched.now;
        let place = super::plan::link_place(from, to);
        let mut keys = (None, None);
        let sites = &mut st.sites;
        let sent = st.net.transmit_with(from, to, &mut |_link, lost, dup| {
            let l = sites.visit(super::plan::Site::Loss, place, now, lost, None);
            keys.0 = l.fire.then_some(l.key);
            let d = sites.visit(super::plan::Site::Dup, place, now, dup, None);
            keys.1 = (d.fire && !l.fire).then_some(d.key);
            (l.fire, d.fire)
        });
        let route = || format!("{what} {} → {}", self.node_name(from), self.node_name(to));
        if let Some(key) = keys.0 {
            self.record_fault(st, from, "loss", route(), Some(key));
        }
        if let Some(key) = keys.1
            && matches!(sent, Transmit::Deliver(_))
        {
            self.record_fault(st, from, "dup", route(), Some(key));
        }
        match sent {
            Transmit::Dropped(why) => {
                match why {
                    super::net::Drop::Loss => st.wire.stats.lost += 1,
                    super::net::Drop::Partition => st.wire.stats.cut += 1,
                }
                let detail = format!(
                    "{what} {} → {}: {}",
                    self.node_name(from),
                    self.node_name(to),
                    why.name()
                );
                self.link_event(st, from, "drop", detail, Some((to, now, false)));
                if keys.0.is_some() {
                    Self::last_is_fault(st);
                }
            }
            Transmit::Deliver(delays) => {
                for (i, delay) in delays.iter().enumerate() {
                    if i > 0 {
                        st.wire.stats.duplicated += 1;
                        let detail =
                            format!("{what} {} → {}", self.node_name(from), self.node_name(to));
                        self.link_event(st, from, "dup", detail, Some((to, now, false)));
                        if keys.1.is_some() {
                            Self::last_is_fault(st);
                        }
                    }
                    st.wire.stats.sent += 1;
                    st.wire.next_msg += 1;
                    let id = st.wire.next_msg;
                    st.wire.in_flight.insert(
                        id,
                        Msg {
                            from,
                            to,
                            reply,
                            body: body.clone(),
                            sent: now,
                        },
                    );
                    let at = st.sched.now + delay;
                    st.sched.deliver_at(at, id);
                }
            }
        }
    }

    /// A message's time has come (called from `pick`, under the lock).
    pub(super) fn deliver(&self, st: &mut State, id: u64) {
        let Some(msg) = st.wire.in_flight.remove(&id) else {
            return;
        };
        let what = describe(&msg.body);
        let route = format!("{} → {}", self.node_name(msg.from), self.node_name(msg.to));
        let link = Some((msg.from, msg.sent, true));
        if !st.net.connected(msg.from, msg.to) {
            st.wire.stats.cut += 1;
            self.link_event(
                st,
                msg.to,
                "drop",
                format!("{what} {route}: partition (in flight)"),
                link,
            );
            return;
        }
        if !st.nodes[msg.to].up {
            // A node that is down answers nothing; its peers time out.
            self.link_event(
                st,
                msg.to,
                "drop",
                format!("{what} {route}: the node is down"),
                link,
            );
            return;
        }
        match msg.body {
            Body::Request(req) => {
                let key = (msg.to, req.port);
                let Some(listener) = st.wire.listeners.get(&key) else {
                    st.wire.stats.refused += 1;
                    self.link_event(
                        st,
                        msg.to,
                        "refused",
                        format!("{what} {route}: nothing listens on port {}", req.port),
                        link,
                    );
                    let answer = Body::Refused {
                        req: req.id,
                        why: format!(
                            "connection refused ({}:{} has no server)",
                            self.node_name(msg.to),
                            req.port
                        ),
                    };
                    self.transmit(st, msg.to, msg.from, req.reply, answer);
                    return;
                };
                let (queue, capacity, max_body) =
                    (listener.queue, listener.capacity, listener.max_body);
                let waiting = st.wire.boxes.get(&queue).map(|q| q.len()).unwrap_or(0);
                let refusal = if req.body.len() > max_body {
                    Some((413, "request body too large".to_string()))
                } else if waiting >= capacity {
                    Some((503, "server overloaded".to_string()))
                } else {
                    None
                };
                st.wire.stats.delivered += 1;
                if let Some((status, text)) = refusal {
                    self.link_event(
                        st,
                        msg.to,
                        "deliver",
                        format!("{what} {route}: {status} {text}"),
                        link,
                    );
                    let answer = Body::Response {
                        req: req.id,
                        resp: Resp {
                            status,
                            headers: vec![(
                                "content-type".into(),
                                "text/plain; charset=utf-8".into(),
                            )],
                            body: text.into_bytes(),
                        },
                    };
                    self.transmit(st, msg.to, msg.from, req.reply, answer);
                    return;
                }
                self.link_event(
                    st,
                    msg.to,
                    "deliver",
                    format!("{what} {route} {} {}", req.method, req.path),
                    link,
                );
                if let Some(q) = st.wire.boxes.get_mut(&queue) {
                    q.push_back(Body::Request(req));
                }
                st.sched.wake(Wait::Net(queue));
            }
            body @ (Body::Response { .. } | Body::Refused { .. }) => {
                match st.wire.boxes.get_mut(&msg.reply) {
                    Some(q) => {
                        q.push_back(body);
                        st.wire.stats.delivered += 1;
                        self.link_event(st, msg.to, "deliver", format!("{what} {route}"), link);
                        st.sched.wake(Wait::Net(msg.reply));
                    }
                    None => {
                        st.wire.stats.late += 1;
                        self.link_event(
                            st,
                            msg.to,
                            "late",
                            format!("{what} {route}: nobody waits for it"),
                            link,
                        );
                    }
                }
            }
        }
    }

    /// Send a request from `me` (on node `from`) to `to`, and note it in
    /// the history. The caller then waits on `req.reply`.
    pub(super) fn send_request(
        &self,
        me: TaskId,
        from: NodeId,
        to: NodeId,
        mut req: Req,
        url: &str,
    ) -> Result<u64, Ended> {
        let mut st = self.lock();
        Self::stopped(&st, me)?;
        st.wire.next_req += 1;
        req.id = st.wire.next_req;
        st.nodes[from].next_port = st.nodes[from].next_port.wrapping_add(1).max(49_152);
        req.remote = format!("{}:{}", self.node_name(from), st.nodes[from].next_port);
        let detail = format!("#{} {} {url}", req.id, req.method);
        self.event(&mut st, me, "http", detail);
        let index = st.wire.history.len();
        let now = st.sched.now;
        st.wire.history.push(History {
            id: req.id,
            node: self.node_name(from).to_string(),
            method: req.method.clone(),
            url: url.to_string(),
            body: String::from_utf8_lossy(&req.body).into_owned(),
            sent: now,
            done: None,
            outcome: "pending",
            status: None,
            response: None,
        });
        st.wire.open_history.insert(req.id, index);
        let id = req.id;
        let reply = req.reply;
        self.transmit(&mut st, from, to, reply, Body::Request(req));
        Ok(id)
    }

    /// The request `id` came to an end: note how.
    pub(super) fn settle_request(
        &self,
        me: TaskId,
        id: u64,
        outcome: &'static str,
        status: Option<i64>,
        response: Option<&[u8]>,
        detail: String,
    ) {
        let mut st = self.lock();
        let now = st.sched.now;
        if outcome == "timeout" {
            st.wire.stats.timeouts += 1;
        }
        if let Some(i) = st.wire.open_history.remove(&id)
            && let Some(h) = st.wire.history.get_mut(i)
        {
            h.done = Some(now);
            h.outcome = outcome;
            h.status = status;
            h.response = response.map(|b| String::from_utf8_lossy(b).into_owned());
        }
        let kind = match outcome {
            "ok" => "reply",
            "timeout" => "timeout",
            _ => "refused",
        };
        self.event(&mut st, me, kind, format!("#{id} {detail}"));
    }

    /// Wait on mailbox `id` for one message, until virtual time `until`
    /// (none: forever). `None` back means the time ran out.
    pub(super) fn await_box(
        &self,
        me: TaskId,
        id: u64,
        until: Option<u64>,
        site: &str,
    ) -> Result<Option<Body>, Ended> {
        loop {
            {
                let mut st = self.lock();
                Self::stopped(&st, me)?;
                if let Some(body) = st.wire.boxes.get_mut(&id).and_then(|q| q.pop_front()) {
                    return Ok(Some(body));
                }
                if let Some(u) = until
                    && st.sched.now >= u
                {
                    return Ok(None);
                }
            }
            self.block(me, Wait::Net(id), site.to_string(), until)?;
        }
    }

    /// Listen on (node, port): the listener's queue and control mailboxes,
    /// and the port (a port 0 is given one).
    pub(super) fn listen(
        &self,
        me: TaskId,
        node: NodeId,
        port: u16,
        capacity: usize,
        max_body: usize,
    ) -> Result<Result<(u16, u64, u64), String>, Ended> {
        let mut st = self.lock();
        Self::stopped(&st, me)?;
        let port = if port == 0 {
            let mut p = 40_000u16;
            while st.wire.listeners.contains_key(&(node, p)) {
                p += 1;
            }
            p
        } else {
            port
        };
        if st.wire.listeners.contains_key(&(node, port)) {
            return Ok(Err(format!(
                "serve: could not bind {}:{port}: address already in use",
                self.node_name(node)
            )));
        }
        let queue = Self::open_box_in(&mut st);
        let ctl = Self::open_box_in(&mut st);
        st.wire.listeners.insert(
            (node, port),
            Listener {
                queue,
                ctl,
                capacity,
                max_body,
            },
        );
        let detail = format!("http://{}:{port}", self.node_name(node));
        self.event(&mut st, me, "listen", detail);
        Ok(Ok((port, queue, ctl)))
    }

    /// Is (node, port) still served by the listener whose queue is `queue`?
    fn listening(st: &State, node: NodeId, port: u16, queue: u64) -> bool {
        st.wire
            .listeners
            .get(&(node, port))
            .is_some_and(|l| l.queue == queue)
    }

    /// A worker's wait for the next request; `None` once the server has
    /// shut down and its queue is empty.
    pub(super) fn next_request(
        &self,
        me: TaskId,
        node: NodeId,
        port: u16,
        queue: u64,
    ) -> Result<Option<Req>, Ended> {
        loop {
            {
                let mut st = self.lock();
                Self::stopped(&st, me)?;
                match st.wire.boxes.get_mut(&queue).and_then(|q| q.pop_front()) {
                    Some(Body::Request(req)) => {
                        let detail = format!("#{} {} {}", req.id, req.method, req.path);
                        self.event(&mut st, me, "handle", detail);
                        return Ok(Some(req));
                    }
                    Some(_) => continue,
                    None => {}
                }
                if !Self::listening(&st, node, port, queue) {
                    st.wire.boxes.remove(&queue);
                    return Ok(None);
                }
            }
            let site = format!(
                "http.serve on {}:{port} (waiting for a request)",
                self.node_name(node)
            );
            self.block(me, Wait::Net(queue), site, None)?;
        }
    }

    /// `http.serve`'s own wait: until the server on (node, port) shuts down.
    pub(super) fn serve_wait(
        &self,
        me: TaskId,
        node: NodeId,
        port: u16,
        queue: u64,
        ctl: u64,
    ) -> Result<(), Ended> {
        loop {
            {
                let st = self.lock();
                Self::stopped(&st, me)?;
                if !Self::listening(&st, node, port, queue) {
                    return Ok(());
                }
            }
            let site = format!("http.serve on {}:{port}", self.node_name(node));
            self.block(me, Wait::Net(ctl), site, None)?;
        }
    }

    /// Answer request `req` from `node`.
    pub(super) fn respond(
        &self,
        me: TaskId,
        node: NodeId,
        req: &Req,
        resp: Resp,
    ) -> Result<(), Ended> {
        let mut st = self.lock();
        Self::stopped(&st, me)?;
        let detail = format!("#{} {}", req.id, resp.status);
        self.event(&mut st, me, "respond", detail);
        self.transmit(
            &mut st,
            node,
            req.from,
            req.reply,
            Body::Response { req: req.id, resp },
        );
        Ok(())
    }

    /// `http.shutdown()` on `node`: every server there stops taking
    /// requests; what is queued is still answered. The count stopped.
    pub(super) fn shutdown(&self, me: TaskId, node: NodeId) -> usize {
        let mut st = self.lock();
        let ports: Vec<u16> = st
            .wire
            .listeners
            .keys()
            .filter(|(n, _)| *n == node)
            .map(|(_, p)| *p)
            .collect();
        for port in &ports {
            if let Some(l) = st.wire.listeners.remove(&(node, *port)) {
                st.sched.wake(Wait::Net(l.queue));
                st.sched.wake(Wait::Net(l.ctl));
            }
            let detail = format!("http://{}:{port}", self.node_name(node));
            self.event(&mut st, me, "shutdown", detail);
        }
        ports.len()
    }

    /// A crash takes the node's servers with it, and what they queued.
    pub(super) fn drop_listeners(&self, st: &mut State, node: NodeId) {
        let keys: Vec<(NodeId, u16)> = st
            .wire
            .listeners
            .keys()
            .filter(|(n, _)| *n == node)
            .copied()
            .collect();
        for key in keys {
            if let Some(l) = st.wire.listeners.remove(&key) {
                st.wire.boxes.remove(&l.queue);
                st.sched.wake(Wait::Net(l.queue));
                st.sched.wake(Wait::Net(l.ctl));
            }
        }
        st.wire.parked.retain(|_, p| p.node != node);
        st.wire.held.retain(|(n, _), _| *n != node);
    }

    // ─── deferred answers (http.defer / respond / hold / notify) ─────

    pub(super) fn park(&self, node: NodeId, req: Req) {
        let mut st = self.lock();
        st.wire
            .parked
            .insert(req.id, super::wire::Parked { node, req });
    }

    pub(super) fn unpark(&self, ticket: u64) -> Option<Parked> {
        self.lock().wire.parked.remove(&ticket)
    }

    pub(super) fn hold(&self, node: NodeId, topic: String, ticket: u64) {
        self.lock()
            .wire
            .held
            .entry((node, topic))
            .or_default()
            .push(ticket);
    }

    pub(super) fn take_held(&self, node: NodeId, topic: Option<&str>) -> Vec<u64> {
        let mut st = self.lock();
        match topic {
            Some(t) => st
                .wire
                .held
                .remove(&(node, t.to_string()))
                .unwrap_or_default(),
            None => {
                let keys: Vec<(NodeId, String)> = st
                    .wire
                    .held
                    .keys()
                    .filter(|(n, _)| *n == node)
                    .cloned()
                    .collect();
                keys.into_iter()
                    .flat_map(|k| st.wire.held.remove(&k).unwrap_or_default())
                    .collect()
            }
        }
    }

    pub(super) fn held_count(&self, node: NodeId, topic: &str) -> usize {
        self.lock()
            .wire
            .held
            .get(&(node, topic.to_string()))
            .map(|v| v.len())
            .unwrap_or(0)
    }

    // ─── the network, changed while the world runs ───────────────────

    pub(super) fn set_partition(
        &self,
        me: TaskId,
        left: &[NodeId],
        right: &[NodeId],
        key: Option<super::plan::Key>,
    ) {
        let mut st = self.lock();
        st.net.partition(left, right);
        let names = |ns: &[NodeId]| {
            ns.iter()
                .map(|n| self.node_name(*n))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let detail = format!("[{}] | [{}]", names(left), names(right));
        let node = left.first().copied().unwrap_or(0);
        self.record_fault(&mut st, node, "partition", detail.clone(), key);
        self.event(&mut st, me, "partition", detail);
        Self::last_is_fault(&mut st);
    }

    pub(super) fn set_heal(&self, me: TaskId) {
        let mut st = self.lock();
        st.net.heal();
        self.event(&mut st, me, "heal", String::new());
    }

    pub(super) fn set_link(&self, me: TaskId, a: NodeId, b: NodeId, link: super::net::Link) {
        let mut st = self.lock();
        st.net.set_link(a, b, link);
        let detail = format!(
            "{} ↔ {}: {}–{} ms, loss {}, dup {}",
            self.node_name(a),
            self.node_name(b),
            link.latency.0,
            link.latency.1,
            link.loss,
            link.dup
        );
        self.event(&mut st, me, "link", detail);
    }
}

fn describe(body: &Body) -> String {
    match body {
        Body::Request(r) => format!("request #{}", r.id),
        Body::Response { req, resp } => format!("response #{req} {}", resp.status),
        Body::Refused { req, .. } => format!("refusal #{req}"),
    }
}

/// A response's body, short, for an event.
pub(super) fn preview(bytes: &[u8]) -> String {
    short(bytes)
}
