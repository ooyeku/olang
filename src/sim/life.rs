//! A node's process inside a world (S3): its environment, its working
//! directory, its stdin, its shutdown handlers and its exit — each the
//! node's own, never the real process's.
//!
//! - **The environment** starts as the node was given (`#{ env }`);
//!   `os.set_env` and `os.remove_env` change it for this process only (a
//!   restart starts from what was given). `os.get_env`, `os.has_env`,
//!   `os.env` and `os.list_env` read it.
//! - **`os.cwd()`** is the node's working directory (`#{ cwd }`, `/`);
//!   `os.pid()` a number per node and boot, `os.hostname()` the node's
//!   name, `os.temp_dir()` its `/tmp`.
//! - **stdin**: `os.read_line()` reads the node's stdin, which holds
//!   `#{ stdin }` and what `w.write_stdin(n, text)` adds, waiting (on
//!   virtual time) for a line while it is open; `w.close_stdin(n)` ends
//!   it, and then `os.read_line()` answers `Err("eof")`. A node given no
//!   stdin reads an empty, closed one.
//! - **`os.on_shutdown(f)`** keeps `f` for `w.stop(n)`; a crash or a kill
//!   never runs it (node.rs).
//! - **`os.exit(code)`** ends the node's process: its tasks stop, its
//!   servers close, its disk keeps what was written. A code other than 0
//!   fails the run.
//!
//! Also here: `sim.maybe(point)` (buggify points) and the random crash
//! points a node's `#{ crash }` plan places at effect boundaries.

use super::node::Halt;
use super::sched::{TaskId, Wait};
use super::{Ctx, Ended, World, misuse};
use crate::ast::{Value, ValueMap};
use crate::interpreter::{Interpreter, InterpreterError};
use rand::{RngExt, SeedableRng};
use std::sync::Arc;

fn s(text: impl Into<String>) -> Value {
    Value::String(Arc::new(text.into()))
}

fn one_string<'a>(name: &str, args: &'a [Value]) -> Result<&'a str, InterpreterError> {
    match args {
        [Value::String(k)] => Ok(k.as_str()),
        [_] => Err(misuse(format!("os.{name}: the name must be a string"))),
        _ => Err(misuse(format!(
            "os.{name} expects 1 argument, got {}",
            args.len()
        ))),
    }
}

/// `os.<verb>` inside a world. `None` lets the builtin run (it is pure,
/// or refused before it reaches here).
pub(super) fn call(
    ctx: &Ctx,
    verb: &str,
    args: &[Value],
    interpreter: &mut Interpreter,
) -> Option<Result<Value, InterpreterError>> {
    let w = &ctx.world;
    let n = ctx.node;
    Some(match verb {
        "args" if args.is_empty() => Ok(Value::List(Arc::new(
            w.nodes[n].argv.iter().map(|a| s(a.clone())).collect(),
        ))),
        "get_env" => one_string("get_env", args).map(|k| match w.lock().nodes[n].env.get(k) {
            Some(v) => Value::Ok(Box::new(s(v.clone()))),
            None => Value::Err(Box::new(s(format!("Environment variable '{k}' not found")))),
        }),
        "has_env" => one_string("has_env", args)
            .map(|k| Value::Boolean(w.lock().nodes[n].env.contains_key(k))),
        "env" => one_string("env", args).map(|k| {
            w.lock().nodes[n]
                .env
                .get(k)
                .map(|v| s(v.clone()))
                .unwrap_or(Value::Unit)
        }),
        "list_env" if args.is_empty() => {
            let st = w.lock();
            let mut m = ValueMap::default();
            for (k, v) in &st.nodes[n].env {
                m.insert(k.clone(), s(v.clone()));
            }
            Ok(Value::Map(Arc::new(m)))
        }
        "set_env" => match args {
            [Value::String(k), v] => {
                let v = match v {
                    Value::String(t) => t.to_string(),
                    other => other.to_string(),
                };
                let mut st = w.lock();
                st.nodes[n].env.insert(k.to_string(), v.clone());
                w.event(&mut st, ctx.task, "env", format!("{k}={v}"));
                Ok(Value::Unit)
            }
            _ => Err(misuse("os.set_env expects a name (a string) and a value")),
        },
        "remove_env" => one_string("remove_env", args).map(|k| {
            w.lock().nodes[n].env.remove(k);
            Value::Unit
        }),
        "cwd" if args.is_empty() => Ok(Value::Ok(Box::new(s(w.lock().nodes[n].cwd.clone())))),
        // a process id per node and boot: the same in every run of a seed
        "pid" if args.is_empty() => {
            let boot = super::vfs::lock(&w.disks[n]).disk.boot as i64;
            Ok(Value::Integer(1_000 * n as i64 + boot + 1))
        }
        // the machine a node runs on is the node: its name, its /tmp
        "hostname" if args.is_empty() => Ok(Value::Ok(Box::new(s(w.nodes[n].name.to_string())))),
        "temp_dir" if args.is_empty() => Ok(s("/tmp")),
        // a node's output is the world's events, never a terminal
        "is_tty" | "stdin_is_tty" | "color_enabled" if args.is_empty() => Ok(Value::Boolean(false)),
        "flush" => Ok(Value::Unit),
        "path_separator" if args.is_empty() => Ok(s("/")),
        // no signal reaches a world: w.stop runs the shutdown handlers
        "on_interrupt" if args.is_empty() => Ok(Value::Ok(Box::new(Value::Unit))),
        "interrupted" if args.is_empty() => Ok(Value::Boolean(false)),
        "reset_interrupt" if args.is_empty() => Ok(Value::Unit),
        "read_line" if args.is_empty() => read_line(ctx),
        "on_shutdown" => match args {
            [f @ (Value::Function(_) | Value::Builtin(_))] => {
                w.handlers
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(n)
                    .or_default()
                    .push((interpreter.thread_safe_clone(), f.clone()));
                w.note(ctx.task, "on_shutdown", "a handler kept".into());
                Ok(Value::Ok(Box::new(Value::Unit)))
            }
            _ => Err(misuse("os.on_shutdown: expected a function")),
        },
        "exit" => match args {
            [Value::Integer(code)] => exit(ctx, *code),
            _ => Err(misuse("os.exit: argument must be an integer")),
        },
        _ => return None,
    })
}

/// `os.read_line()`: the next line of the node's stdin (without its
/// newline), `Err("eof")` once it is closed and empty.
fn read_line(ctx: &Ctx) -> Result<Value, InterpreterError> {
    let w = &ctx.world;
    let n = ctx.node;
    loop {
        {
            let mut st = w.lock();
            World::stopped(&st, ctx.task)?;
            let stdin = &mut st.nodes[n].stdin;
            let line = match stdin.buf.find('\n') {
                Some(i) => {
                    let mut line: String = stdin.buf.drain(..=i).collect();
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                    Some(line)
                }
                None if !stdin.open && !stdin.buf.is_empty() => {
                    Some(std::mem::take(&mut stdin.buf))
                }
                None if !stdin.open => {
                    w.event(&mut st, ctx.task, "stdin", "eof".into());
                    return Ok(Value::Err(Box::new(s("eof"))));
                }
                None => None,
            };
            if let Some(line) = line {
                w.event(&mut st, ctx.task, "stdin", line.clone());
                return Ok(Value::Ok(Box::new(s(line))));
            }
        }
        w.block(
            ctx.task,
            Wait::Stdin(n),
            "os.read_line (stdin open, no line yet)".into(),
            None,
        )?;
    }
}

/// `os.exit(code)`: the node's process ends here.
fn exit(ctx: &Ctx, code: i64) -> Result<Value, InterpreterError> {
    let w = &ctx.world;
    let n = ctx.node;
    if n == 0 {
        return Err(misuse(format!(
            "sim: os.exit({code}) in the world's main program (it ends the run as a raise would)"
        )));
    }
    {
        let mut st = w.lock();
        World::stopped(&st, ctx.task)?;
        st.nodes[n].exit = Some(if code == 0 {
            Ok(Value::Integer(0))
        } else {
            Err(format!("exited with status {code}"))
        });
        if code != 0 && st.failure.is_none() {
            st.failure = Some(format!(
                "node {} exited with status {code}",
                w.nodes[n].name
            ));
        }
        w.halt_locked(
            &mut st,
            ctx.task,
            n,
            Halt::Exit,
            &format!("os.exit({code})"),
            None,
        );
        st.killed[ctx.task] = Some(Ended::Exit);
    }
    Err(Ended::Exit.into())
}

/// FNV-1a: a point's name as a number, the same on every machine.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// How often a point the seed turned on fires when reached.
const MAYBE_FIRES: f64 = 0.25;
/// The share of points a seed turns on.
const MAYBE_ON: f64 = 0.25;

impl World {
    /// `sim.maybe(point)` in this world: a point is on or off for the
    /// whole run (decided by the seed and the point's name alone, so
    /// adding a point moves no other), and one that is on fires a
    /// quarter of the times it is reached.
    pub(super) fn maybe(&self, me: TaskId, point: &str) -> bool {
        let mut st = self.lock();
        let seed = self.seed;
        let on = *st.maybe_on.entry(point.to_string()).or_insert_with(|| {
            let mut r = rand::rngs::StdRng::seed_from_u64(super::sched::split(
                super::sched::split(seed, 6),
                fnv(point),
            ));
            r.random::<f64>() < MAYBE_ON
        });
        let drawn = on && st.maybe_rng.random::<f64>() < MAYBE_FIRES;
        let node = st.task_node.get(me).copied().unwrap_or(0);
        let now = st.sched.now;
        let d = st
            .sites
            .visit(super::plan::Site::Maybe, node as u64, now, drawn, None);
        if d.fire {
            self.fault_at(
                &mut st,
                Some(me),
                node,
                "maybe",
                point.to_string(),
                Some(d.key),
            );
        }
        d.fire
    }

    /// At an effect boundary of a node with a crash plan: crash here,
    /// when the seed says so. Then the task stops.
    pub(super) fn maybe_crash(self: &Arc<World>, ctx: &Ctx, name: &str) -> Result<(), Ended> {
        let Some(plan) = self.nodes[ctx.node].crash else {
            return Ok(());
        };
        let hit = {
            let st = self.lock();
            World::stopped(&st, ctx.task)?;
            if st.nodes[ctx.node].up {
                super::vfs::lock(&self.disks[ctx.node]).draw_crash(plan.restart_after)
            } else {
                None
            }
        };
        let Some(key) = hit else {
            return Ok(());
        };
        let _ = self.halt(
            ctx.task,
            ctx.node,
            Halt::Crash,
            key.delay,
            &format!("at {name}"),
            Some(key),
        );
        Err(Ended::Crash)
    }

    /// `w.write_stdin(n, text)` / `w.close_stdin(n)`.
    pub(super) fn feed_stdin(&self, me: TaskId, n: usize, text: Option<&str>) {
        let mut st = self.lock();
        match text {
            Some(t) => {
                st.nodes[n].stdin.buf.push_str(t);
                let detail = format!(
                    "{} <- {}",
                    self.nodes[n].name,
                    super::wire::preview(t.as_bytes())
                );
                self.event(&mut st, me, "stdin_write", detail);
            }
            None => {
                st.nodes[n].stdin.open = false;
                let detail = self.nodes[n].name.to_string();
                self.event(&mut st, me, "stdin_close", detail);
            }
        }
        st.sched.wake(Wait::Stdin(n));
    }
}

impl World {
    /// `w.slow_disk(n, factor)`.
    pub(super) fn set_slow(&self, me: TaskId, n: usize, factor: f64) {
        let mut st = self.lock();
        let now = st.sched.now;
        let d = st
            .sites
            .visit(super::plan::Site::SlowDisk, n as u64, now, true, None);
        if !d.fire {
            return;
        }
        super::vfs::lock(&self.disks[n]).disk.opts.slow = factor;
        let detail = format!("{} ×{factor}", self.nodes[n].name);
        self.fault_at(&mut st, Some(me), n, "slow_disk", detail, Some(d.key));
    }

    /// A fault called in code (`w.crash`, `w.kill`, `w.stop`,
    /// `w.partition`) at `place`: whether it happens (always, unless a
    /// plan forces the run and does not list it) and its key.
    pub(super) fn code_fault(
        &self,
        site: super::plan::Site,
        place: usize,
        delay: Option<u64>,
    ) -> super::plan::Decision {
        let mut st = self.lock();
        let now = st.sched.now;
        st.sites.visit(site, place as u64, now, true, delay)
    }
}
