//! `db.*` inside a world (S3): SQLite as outside — the same calls, rows
//! and errors — over the node's disk, through the world's VFS (`vfs.rs`).
//!
//! - **`db.open(path)`** opens the database at `path` on the node's disk
//!   (relative to its working directory; `:memory:` stays in memory). A
//!   URI filename is refused: its parameters could name another VFS.
//! - **Every other call** first checks the connection is this node's,
//!   opened in this world by this process (a connection from outside the
//!   world, another node, or a process that crashed is refused, never
//!   used). Then it runs the ordinary `db` code, and afterwards pays
//!   what the disk cost (each sync one to three ms of virtual time,
//!   slept), records what the disk did, and crashes the node there if a
//!   crash point fell inside the call.
//! - **`db.transaction(conn, f)`** serializes transactions on a
//!   connection as outside, waiting on the world's scheduler rather than
//!   a real lock.
//!
//! Every call is a yield point before it runs, and every call that paid
//! for a sync one after: the seed decides what runs in between, and a
//! crash may fall there.

use super::sched::Wait;
use super::{Ctx, Ended, World, disk, misuse, vfs};
use crate::ast::Value;
use crate::interpreter::{Interpreter, InterpreterError};
use std::sync::atomic::{AtomicU64, Ordering};

/// Owners of connections opened in worlds (0 is the real file system's).
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

pub(super) fn call(
    ctx: &Ctx,
    verb: &str,
    args: &[Value],
    interpreter: &mut Interpreter,
) -> Option<Result<Value, InterpreterError>> {
    Some(match verb {
        "open" => open(ctx, args),
        "transaction" => transaction(ctx, args, interpreter),
        _ => run(ctx, verb, args),
    })
}

fn open(ctx: &Ctx, args: &[Value]) -> Result<Value, InterpreterError> {
    let w = &ctx.world;
    let Some(Value::String(given)) = args.first() else {
        return Err(misuse("db.open: path must be a string"));
    };
    if given.starts_with("file:") {
        return Err(misuse(
            "sim: db.open: a URI filename is refused inside a world (its parameters could \
             reach past the node's disk); give a path",
        ));
    }
    w.yield_point(ctx.task)?;
    let vfs_name = vfs::registered().map_err(misuse)?;
    let owner = NEXT_OWNER.fetch_add(1, Ordering::Relaxed);
    let (path, exists, boot) = {
        let st = w.lock();
        World::stopped(&st, ctx.task)?;
        let d = vfs::lock(&w.disks[ctx.node]);
        if given.is_empty() || given.as_str() == ":memory:" {
            // An in-memory database: no file; its temporary files are
            // the VFS's own memory.
            (given.to_string(), true, d.disk.boot)
        } else {
            let p = disk::normalize(&st.nodes[ctx.node].cwd, given);
            (vfs::sqlite_path(d.key, &p), d.disk.exists(&p), d.disk.boot)
        }
    };
    let answer = crate::stdlib::db::open_simulated(
        args.to_vec(),
        crate::stdlib::db::SimOpen {
            vfs: vfs_name,
            path,
            exists,
            owner,
        },
    )
    .map_err(|e| InterpreterError::runtime(e.to_string()))?;
    {
        let mut st = w.lock();
        st.db_owners.insert(owner, (ctx.node, boot));
        w.event(&mut st, ctx.task, "db_open", given.to_string());
    }
    settle(ctx)?;
    Ok(answer)
}

/// Is the connection (or cursor) in `args[0]` this node's, in this
/// process? `Ok(Some(answer))` when the call is answered without
/// running (a dead process's connection).
fn check(ctx: &Ctx, verb: &str, args: &[Value]) -> Result<Option<Value>, InterpreterError> {
    let Some(first) = args.first() else {
        return Ok(None);
    };
    let Some(owner) = crate::stdlib::db::owner_of(first) else {
        // not a live connection: the db code says what is wrong
        return Ok(None);
    };
    if owner == 0 {
        return Err(misuse(format!(
            "sim: db.{verb} is refused inside a world on a connection opened outside it: a \
             world never reaches the real disk (open the database inside the world)"
        )));
    }
    let w = &ctx.world;
    let st = w.lock();
    let Some((node, boot)) = st.db_owners.get(&owner).copied() else {
        return Err(misuse(format!(
            "sim: db.{verb}: this connection was opened in another world"
        )));
    };
    if node != ctx.node {
        return Err(misuse(format!(
            "sim: db.{verb}: this connection belongs to node {}, and a node reaches only its \
             own disk",
            w.nodes[node].name
        )));
    }
    if vfs::lock(&w.disks[node]).disk.boot != boot {
        return Ok(Some(Value::Err(Box::new(Value::String(
            std::sync::Arc::new(format!(
                "db.{verb}: disk I/O error (the process that opened this connection is gone)"
            )),
        )))));
    }
    Ok(None)
}

/// One `db` call: a yield point, the ordinary code, then what the disk
/// cost and did.
fn run(ctx: &Ctx, verb: &str, args: &[Value]) -> Result<Value, InterpreterError> {
    if let Some(answer) = check(ctx, verb, args)? {
        return Ok(answer);
    }
    ctx.world.yield_point(ctx.task)?;
    let answer = crate::stdlib::db::call_db_function(verb, args.to_vec())
        .map_err(|e| InterpreterError::runtime(e.to_string()));
    settle(ctx)?;
    answer
}

/// After a `db` call: the disk's events into the world's, a crash that
/// fell inside the call carried out, and the virtual time its syncs took
/// slept.
fn settle(ctx: &Ctx) -> Result<(), Ended> {
    let w = &ctx.world;
    let crash_at = {
        let mut st = w.lock();
        w.flush_disk(&mut st, ctx.task, ctx.node);
        vfs::lock(&w.disks[ctx.node]).crash_at.clone()
    };
    if let Some(point) = crash_at {
        vfs::take_debt_ms();
        let restart = w.nodes[ctx.node].crash.and_then(|c| c.restart_after);
        let _ = w.halt(
            ctx.task,
            ctx.node,
            super::node::Halt::Crash,
            restart,
            &format!("at {point}"),
        );
        return Err(Ended::Crash);
    }
    let ms = vfs::take_debt_ms();
    if ms > 0 {
        let until = w.now() + ms;
        w.block(
            ctx.task,
            Wait::Sleep,
            "db: waiting for the disk".into(),
            Some(until),
        )?;
    }
    Ok(())
}

/// Releases a connection's transaction slot however the transaction ends.
struct Slot<'a> {
    ctx: &'a Ctx,
    id: i64,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let w = &self.ctx.world;
        let mut st = w.lock();
        let free = match st.tx.get_mut(&self.id) {
            Some((_, depth)) if *depth > 1 => {
                *depth -= 1;
                false
            }
            Some(_) => true,
            None => false,
        };
        if free {
            st.tx.remove(&self.id);
            st.sched.wake(Wait::Tx(self.id));
        }
    }
}

fn enter<'a>(ctx: &'a Ctx, id: i64) -> Result<Slot<'a>, Ended> {
    let w = &ctx.world;
    loop {
        {
            let mut st = w.lock();
            World::stopped(&st, ctx.task)?;
            match st.tx.get_mut(&id) {
                None => {
                    st.tx.insert(id, (ctx.task, 1));
                    return Ok(Slot { ctx, id });
                }
                Some((t, depth)) if *t == ctx.task => {
                    *depth += 1;
                    return Ok(Slot { ctx, id });
                }
                Some(_) => {}
            }
        }
        w.block(
            ctx.task,
            Wait::Tx(id),
            format!("db.transaction on connection {id} (another task's is open)"),
            None,
        )?;
    }
}

/// `db.transaction(conn, f)`: as outside (`builtin.rs`), on the world's
/// scheduler.
fn transaction(
    ctx: &Ctx,
    args: &[Value],
    interpreter: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    let [conn, f] = args else {
        return Err(InterpreterError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    };
    if let Some(answer) = check(ctx, "transaction", args)? {
        return Ok(answer);
    }
    let Some(id) = crate::stdlib::db::connection_id_of(conn) else {
        return run(ctx, "begin", std::slice::from_ref(conn));
    };
    let _slot = enter(ctx, id)?;
    let begun = run(ctx, "begin", std::slice::from_ref(conn))?;
    if matches!(begun, Value::Err(_)) {
        return Ok(begun);
    }
    match interpreter.call_function(f.clone(), vec![conn.clone()]) {
        Ok(failed @ Value::Err(_)) => {
            let _ = run(ctx, "rollback", std::slice::from_ref(conn));
            Ok(failed)
        }
        Ok(value) => match run(ctx, "commit", std::slice::from_ref(conn))? {
            failed @ Value::Err(_) => Ok(failed),
            _ => Ok(value),
        },
        Err(raised) => {
            let _ = run(ctx, "rollback", std::slice::from_ref(conn));
            Err(raised)
        }
    }
}
