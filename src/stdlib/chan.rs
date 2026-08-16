//! `chan` — typed message passing between `spawn`ed tasks. Channels are
//! multi-producer multi-consumer queues of olang values: `chan.new()`
//! makes an unbounded channel, `chan.bounded(n)` one that holds at most
//! `n` in-flight messages (senders block when it is full; `0` makes a
//! rendezvous channel where send waits for a receiver). Closing is
//! cooperative and drains: after `chan.close(c)` sends fail, but queued
//! messages are still received before `recv` reports the close.
//! Everything fallible speaks Result, so `?` and `match` drive the
//! control flow.
//!
//! A handle is a `Value::Native` that *owns* the channel through an `Arc`,
//! so the channel lives exactly as long as some olang value references it
//! and is freed when the last handle drops — a program that opens a
//! channel per tick does not accumulate them. Cloning a handle (including
//! across the `spawn` boundary) shares the one underlying channel by
//! reference count; no process-wide registry, so nothing to leak.

use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};

/// One channel's two ends. The sender is cloned out of its lock before
/// use so a blocking bounded send never holds it; the receiver stays
/// locked across a blocking recv, which is what makes concurrent
/// consumers take turns (each message goes to exactly one).
struct Chan {
    tx: Mutex<Option<Tx>>,
    rx: Mutex<Receiver<Value>>,
}

#[derive(Clone)]
enum Tx {
    Unbounded(Sender<Value>),
    Bounded(SyncSender<Value>),
}

impl Tx {
    fn send(&self, v: Value) -> Result<(), ()> {
        match self {
            Tx::Unbounded(t) => t.send(v).map_err(|_| ()),
            Tx::Bounded(t) => t.send(v).map_err(|_| ()),
        }
    }
}

/// The native handle wrapping a channel. Owning the `Arc<Chan>` here — in
/// the olang value itself — is what makes channel lifetime reference-counted
/// rather than registry-pinned.
struct ChanObject(Arc<Chan>);

impl std::fmt::Debug for ChanObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<channel>")
    }
}

impl NativeObject for ChanObject {
    fn module(&self) -> &'static str {
        "chan"
    }
    fn type_name(&self) -> &'static str {
        "Channel"
    }
    fn display(&self) -> String {
        "<channel>".to_string()
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        // Identity: two handles are equal iff they carry the same channel.
        other
            .as_any()
            .downcast_ref::<ChanObject>()
            .map(|o| Arc::ptr_eq(&self.0, &o.0))
            .unwrap_or(false)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn create_chan_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [
        ("new", 0),
        ("bounded", 1),
        ("send", 2),
        ("recv", 1),
        ("try_recv", 1),
        ("recv_timeout", 2),
        ("close", 1),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("chan.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

pub fn call_chan_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "new" => chan_new(args),
        "bounded" => chan_bounded(args),
        "send" => chan_send(args),
        "recv" => chan_recv(args),
        "try_recv" => chan_try_recv(args),
        "recv_timeout" => chan_recv_timeout(args),
        "close" => chan_close(args),
        _ => Err(format!("Unknown chan function: {}", name).into()),
    }
}

fn handle(chan: Arc<Chan>) -> Value {
    Value::Native(NativeHandle::new(ChanObject(chan)))
}

fn err(msg: impl Into<String>) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg.into()))))
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

/// Pull the shared `Arc<Chan>` back out of a channel handle.
fn chan_of(value: &Value) -> Result<Arc<Chan>, Box<dyn std::error::Error>> {
    match value {
        Value::Native(h) => match h.0.as_any().downcast_ref::<ChanObject>() {
            Some(obj) => Ok(obj.0.clone()),
            None => Err(format!(
                "chan: expected a channel (from chan.new), got a {} handle",
                h.0.type_name()
            )
            .into()),
        },
        other => Err(format!(
            "chan: expected a channel (from chan.new), got {}",
            other.type_name()
        )
        .into()),
    }
}

fn register(tx: Tx, rx: Receiver<Value>) -> Value {
    handle(Arc::new(Chan {
        tx: Mutex::new(Some(tx)),
        rx: Mutex::new(rx),
    }))
}

fn chan_new(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err("chan.new takes no arguments".into());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    Ok(register(Tx::Unbounded(tx), rx))
}

fn chan_bounded(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let cap = match args.as_slice() {
        [Value::Integer(n)] if *n >= 0 => *n as usize,
        _ => return Err("chan.bounded expects a non-negative Int capacity".into()),
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(cap);
    Ok(register(Tx::Bounded(tx), rx))
}

fn chan_send(mut args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err("chan.send expects a channel and a value".into());
    }
    let value = args.pop().expect("len checked");
    // A channel is the one thread crossing where the value being sent is in
    // hand, so the cell-confinement rule is enforced here — at the send,
    // where the mistake is — instead of on the receiver's first access.
    if crate::stdlib::cell::contains_cell(&value) {
        return Err(
            "chan.send: a cell cannot be sent through a channel — a cell belongs \
                    to the thread that created it. Send its contents instead \
                    (`chan.send(ch, cell.get(c))`)"
                .into(),
        );
    }
    let chan = chan_of(&args[0])?;
    // Clone the sender out of its lock: a bounded send may block until a
    // receiver drains, and holding the lock would stall chan.close.
    let tx = chan.tx.lock().unwrap().clone();
    Ok(match tx {
        Some(tx) => match tx.send(value) {
            Ok(()) => ok(Value::Unit),
            Err(()) => err("channel is closed"),
        },
        None => err("channel is closed"),
    })
}

fn chan_recv(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err("chan.recv expects a channel".into());
    }
    let chan = chan_of(&args[0])?;
    let rx = chan.rx.lock().unwrap();
    Ok(match rx.recv() {
        Ok(v) => ok(v),
        Err(_) => err("channel is closed"),
    })
}

fn chan_try_recv(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err("chan.try_recv expects a channel".into());
    }
    let chan = chan_of(&args[0])?;
    let rx = chan.rx.lock().unwrap();
    Ok(match rx.try_recv() {
        Ok(v) => ok(v),
        Err(TryRecvError::Empty) => err("channel is empty"),
        Err(TryRecvError::Disconnected) => err("channel is closed"),
    })
}

fn chan_recv_timeout(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let ms = match args.as_slice() {
        [_, Value::Integer(ms)] if *ms >= 0 => *ms as u64,
        _ => return Err("chan.recv_timeout expects a channel and a non-negative Int ms".into()),
    };
    let chan = chan_of(&args[0])?;
    let rx = chan.rx.lock().unwrap();
    Ok(
        match rx.recv_timeout(std::time::Duration::from_millis(ms)) {
            Ok(v) => ok(v),
            Err(RecvTimeoutError::Timeout) => err("timed out"),
            Err(RecvTimeoutError::Disconnected) => err("channel is closed"),
        },
    )
}

/// Close the sending side. Idempotent; queued messages still drain.
fn chan_close(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err("chan.close expects a channel".into());
    }
    let chan = chan_of(&args[0])?;
    *chan.tx.lock().unwrap() = None;
    Ok(Value::Unit)
}
