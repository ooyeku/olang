//! `fs.*` inside a world (S3): the same calls, the same answers and the
//! same error wording as outside, read from and written to the node's own
//! disk (`disk.rs`). Paths are the node's: relative ones resolve against
//! its working directory, and none of them names a real file.
//!
//! **When a write counts.** A write (`write_file`, `write_bytes`,
//! `append_file`, and the calls that create, remove or rename) changes
//! what the node sees at once and what survives a crash not at all:
//! `fs.sync(path)` makes a file's bytes and its name durable, and
//! `fs.sync(dir)` a directory's entries. A sync takes one to three ms of
//! virtual time (times the disk's `slow`), and is durable only when it
//! returns: a crash during it keeps nothing of it. It may lie
//! (`fsync_lies`). Every change is a yield point, so a crash can fall
//! between any write and the sync that follows it.

use super::disk::{self, Disk};
use super::sched::Wait;
use super::{Ctx, World, misuse, vfs};
use crate::ast::{Value, ValueMap};
use crate::interpreter::InterpreterError;
use std::sync::Arc;

fn s(text: impl Into<String>) -> Value {
    Value::String(Arc::new(text.into()))
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

fn err(text: String) -> Value {
    Value::Err(Box::new(s(text)))
}

fn arity(name: &str, n: usize, args: &[Value]) -> Result<(), InterpreterError> {
    if args.len() == n {
        Ok(())
    } else {
        let what = if n == 1 { "argument" } else { "arguments" };
        Err(misuse(format!(
            "{name} expects {n} {what}, got {}",
            args.len()
        )))
    }
}

fn path_arg<'a>(
    name: &str,
    args: &'a [Value],
    i: usize,
    what: &str,
) -> Result<&'a str, InterpreterError> {
    match args.get(i) {
        Some(Value::String(p)) => Ok(p.as_str()),
        _ => Err(misuse(format!("{name}: {what} must be a string"))),
    }
}

/// `fs.<verb>` in a world. `None` for the pure helpers (`join`, ...),
/// which run as they always do.
pub(super) fn call(
    ctx: &Ctx,
    verb: &str,
    args: &[Value],
) -> Option<Result<Value, InterpreterError>> {
    Some(match verb {
        "read_file" | "read_bytes" | "exists" | "is_file" | "is_dir" | "list_dir" | "walk"
        | "glob" | "file_size" | "file_info" | "abs_path" | "real_path" => read(ctx, verb, args),
        "write_file" | "write_bytes" | "append_file" | "create_dir" | "create_dir_all"
        | "remove_dir" | "remove_dir_all" | "remove_file" | "copy_file" | "move_file" => {
            change(ctx, verb, args)
        }
        "sync" => sync(ctx, args),
        _ => return None,
    })
}

/// Run `f` on the node's disk (the state's lock held, then the disk's).
fn with_disk<R>(
    ctx: &Ctx,
    f: impl FnOnce(&mut Disk, &str, i64) -> R,
) -> Result<R, InterpreterError> {
    let w = &ctx.world;
    let st = w.lock();
    World::stopped(&st, ctx.task)?;
    let cwd = st.nodes[ctx.node].cwd.clone();
    let now = w.start_ms + st.sched.now as i64 + w.nodes[ctx.node].skew_ms;
    let mut d = vfs::lock(&w.disks[ctx.node]);
    let r = f(&mut d.disk, &cwd, now);
    drop(d);
    drop(st);
    Ok(r)
}

fn read(ctx: &Ctx, verb: &str, args: &[Value]) -> Result<Value, InterpreterError> {
    let name = verb;
    arity(name, 1, args)?;
    let given = path_arg(
        name,
        args,
        0,
        if verb == "glob" { "pattern" } else { "path" },
    )?
    .to_string();
    with_disk(ctx, |d, cwd, _| {
        let p = disk::normalize(cwd, &given);
        match verb {
            "read_file" => match d.read(&p) {
                Ok(bytes) => match String::from_utf8(bytes.to_vec()) {
                    Ok(text) => ok(s(text)),
                    Err(_) => err(format!(
                        "Failed to read file '{given}': stream did not contain valid UTF-8"
                    )),
                },
                Err(e) => err(format!("Failed to read file '{given}': {e}")),
            },
            "read_bytes" => match d.read(&p) {
                Ok(bytes) => ok(crate::stdlib::bytes::to_value(bytes.to_vec())),
                Err(e) => err(format!("Failed to read file '{given}': {e}")),
            },
            "exists" => Value::Boolean(d.exists(&p)),
            "is_file" => Value::Boolean(d.is_file(&p)),
            "is_dir" => Value::Boolean(d.is_dir(&p)),
            "list_dir" => match d.list(&p) {
                Ok(names) => ok(Value::List(
                    names.into_iter().map(s).collect::<Vec<_>>().into(),
                )),
                Err(e) => err(format!("Failed to read directory '{given}': {e}")),
            },
            "walk" => match d.walk(&p) {
                Ok(files) => {
                    // prefixed with the argument, as the real walk's are
                    let head = given.trim_end_matches('/');
                    let cut = if p == "/" { 0 } else { p.len() };
                    ok(Value::List(
                        files
                            .into_iter()
                            .map(|f| s(format!("{head}{}", &f[cut..])))
                            .collect::<Vec<_>>()
                            .into(),
                    ))
                }
                Err(e) => err(format!("walk: {e}")),
            },
            "glob" => {
                let absolute = given.starts_with('/');
                let base = if cwd == "/" {
                    "/".to_string()
                } else {
                    format!("{cwd}/")
                };
                let mut found: Vec<String> = d
                    .paths()
                    .filter(|(_, e)| matches!(e, disk::Entry::File(_)))
                    .filter_map(|(f, _)| {
                        if absolute {
                            Some(f.clone())
                        } else {
                            f.strip_prefix(&base).map(str::to_string)
                        }
                    })
                    .filter(|c| crate::stdlib::fs::glob_match(&given, c))
                    .collect();
                found.sort();
                ok(Value::List(
                    found.into_iter().map(s).collect::<Vec<_>>().into(),
                ))
            }
            "file_size" => match d.read(&p) {
                Ok(bytes) => ok(Value::Integer(bytes.len() as i64)),
                Err(e) => err(format!("Failed to get file size for '{given}': {e}")),
            },
            "file_info" => match d.entry(&p) {
                Some(entry) => {
                    let mut info = ValueMap::default();
                    let size = d.read(&p).map(|b| b.len() as i64).unwrap_or(0);
                    info.insert("size".into(), Value::Integer(size));
                    info.insert(
                        "is_file".into(),
                        Value::Boolean(matches!(entry, disk::Entry::File(_))),
                    );
                    info.insert("is_dir".into(), Value::Boolean(entry == disk::Entry::Dir));
                    info.insert("readonly".into(), Value::Boolean(false));
                    info.insert(
                        "modified_ms".into(),
                        d.modified(&p).map(Value::Integer).unwrap_or(Value::Unit),
                    );
                    ok(Value::Struct {
                        type_name: "Object".into(),
                        fields: Arc::new(info).into(),
                    })
                }
                None => err(format!(
                    "Failed to get file info for '{given}': {}",
                    disk::NOT_FOUND
                )),
            },
            // no links on a node's disk: the real path is the absolute one
            "abs_path" | "real_path" => ok(s(p)),
            _ => unreachable!("fs.{verb} is a read"),
        }
    })
}

fn change(ctx: &Ctx, verb: &str, args: &[Value]) -> Result<Value, InterpreterError> {
    let two = matches!(
        verb,
        "write_file" | "write_bytes" | "append_file" | "copy_file" | "move_file"
    );
    arity(verb, if two { 2 } else { 1 }, args)?;
    let a = path_arg(
        verb,
        args,
        0,
        if two && verb.ends_with("_file") && verb != "write_file" && verb != "append_file" {
            "source path"
        } else {
            "path"
        },
    )?
    .to_string();
    let bytes: Option<Vec<u8>> = match verb {
        "write_file" | "append_file" => match &args[1] {
            Value::String(t) => Some(t.as_bytes().to_vec()),
            _ => return Err(misuse(format!("{verb}: contents must be a string"))),
        },
        "write_bytes" => Some(
            crate::stdlib::bytes::bytes_of(&args[1])
                .map_err(|e| misuse(format!("write_bytes: contents must be Bytes — {e}")))?
                .to_vec(),
        ),
        _ => None,
    };
    let b = if matches!(verb, "copy_file" | "move_file") {
        Some(path_arg(verb, args, 1, "destination path")?.to_string())
    } else {
        None
    };
    let (answer, detail) = with_disk(ctx, |d, cwd, now| {
        let p = disk::normalize(cwd, &a);
        let q = b.as_deref().map(|b| disk::normalize(cwd, b));
        let r = match verb {
            "write_file" | "write_bytes" => {
                d.write(&p, bytes.as_deref().unwrap_or(&[]), false, now)
            }
            "append_file" => d.write(&p, bytes.as_deref().unwrap_or(&[]), true, now),
            "create_dir" => d.mkdir(&p),
            "create_dir_all" => d.mkdir_all(&p),
            "remove_dir" => d.rmdir(&p),
            "remove_dir_all" => d.rmdir_all(&p),
            "remove_file" => d.unlink(&p),
            "copy_file" => d.copy(&p, q.as_deref().unwrap_or("/"), now),
            "move_file" => d.rename(&p, q.as_deref().unwrap_or("/")),
            _ => unreachable!("fs.{verb} is a change"),
        };
        let full = r.as_ref().err().is_some_and(|e| e == disk::NO_SPACE);
        let answer = match r {
            Ok(()) => ok(Value::Unit),
            Err(e) => {
                let b = b.as_deref().unwrap_or("");
                err(match verb {
                    "write_file" | "write_bytes" => format!("Failed to write file '{a}': {e}"),
                    "append_file" => format!("Failed to append to file '{a}': {e}"),
                    "create_dir" => format!("Failed to create directory '{a}': {e}"),
                    "create_dir_all" => format!("Failed to create directory tree '{a}': {e}"),
                    "remove_dir" => format!("Failed to remove directory '{a}': {e}"),
                    "remove_dir_all" => format!("Failed to remove directory tree '{a}': {e}"),
                    "remove_file" => format!("Failed to remove file '{a}': {e}"),
                    "copy_file" => format!("Failed to copy '{a}' to '{b}': {e}"),
                    _ => format!("Failed to move '{a}' to '{b}': {e}"),
                })
            }
        };
        let mut detail = match (&q, &bytes) {
            (Some(q), _) => format!("{verb} {p} -> {q}"),
            (None, Some(bytes)) => format!("{verb} {p} ({} bytes)", bytes.len()),
            (None, None) => format!("{verb} {p}"),
        };
        if full {
            detail = format!("{p}: {}", disk::NO_SPACE);
        }
        (answer, (full, detail))
    })?;
    {
        let w = &ctx.world;
        let mut st = w.lock();
        let (full, detail) = detail;
        if full {
            w.fault(&mut st, Some(ctx.task), ctx.node, "disk_full", detail);
        } else if matches!(answer, Value::Ok(_)) {
            w.event(&mut st, ctx.task, "fs", detail);
        }
    }
    // A crash may fall right after any change.
    ctx.world.yield_point(ctx.task)?;
    Ok(answer)
}

/// `fs.sync(path)`: durable when it returns (see the module doc).
fn sync(ctx: &Ctx, args: &[Value]) -> Result<Value, InterpreterError> {
    arity("sync", 1, args)?;
    let given = path_arg("sync", args, 0, "path")?.to_string();
    let w = &ctx.world;
    let found = with_disk(ctx, |d, cwd, _| {
        let p = disk::normalize(cwd, &given);
        d.exists(&p).then_some(p)
    })?;
    let Some(p) = found else {
        return Ok(err(format!(
            "Failed to sync '{given}': {}",
            disk::NOT_FOUND
        )));
    };
    let (cost_us, boot) = {
        let mut d = vfs::lock(&w.disks[ctx.node]);
        (d.sync_latency_us(), d.disk.boot)
    };
    let ms = cost_us.div_ceil(1_000);
    if ms == 0 {
        w.yield_point(ctx.task)?;
    } else {
        let until = w.now() + ms;
        w.block(
            ctx.task,
            Wait::Sleep,
            format!("fs.sync({given})"),
            Some(until),
        )?;
    }
    // Still this process, and the file still there: now it counts.
    let mut st = w.lock();
    World::stopped(&st, ctx.task)?;
    let mut d = vfs::lock(&w.disks[ctx.node]);
    if d.disk.boot != boot {
        return Err(super::Ended::Crash.into());
    }
    let lie = d.draw_lie();
    let r = d.disk.sync(&p, lie.is_some());
    drop(d);
    match r {
        Ok(()) if lie.is_some() => {
            w.fault_at(&mut st, Some(ctx.task), ctx.node, "fsync_lie", p, lie)
        }
        Ok(()) => w.event(&mut st, ctx.task, "fsync", p),
        Err(e) => return Ok(err(format!("Failed to sync '{given}': {e}"))),
    }
    Ok(ok(Value::Unit))
}
