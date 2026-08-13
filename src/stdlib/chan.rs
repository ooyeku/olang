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
//! Handles are `Channel { id }` structs into a process-wide registry,
//! the same pattern as `spawn`'s task registry and `db`'s connections —
//! a handle crosses the spawn boundary as plain data.

use crate::ast::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};

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

static NEXT_ID: AtomicI64 = AtomicI64::new(1);
static CHANNELS: OnceLock<Mutex<HashMap<i64, Arc<Chan>>>> = OnceLock::new();

fn channels() -> &'static Mutex<HashMap<i64, Arc<Chan>>> {
    CHANNELS.get_or_init(|| Mutex::new(HashMap::new()))
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

fn handle(id: i64) -> Value {
    let mut fields = HashMap::new();
    fields.insert("id".to_string(), Value::Integer(id));
    Value::Struct {
        type_name: "Channel".to_string(),
        fields,
    }
}

fn err(msg: impl Into<String>) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg.into()))))
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

/// Pull the registry entry back out of a `Channel { id }` handle.
fn chan_of(value: &Value) -> Result<Arc<Chan>, Box<dyn std::error::Error>> {
    let id = match value {
        Value::Struct { type_name, fields } if type_name == "Channel" => match fields.get("id") {
            Some(Value::Integer(id)) => *id,
            _ => return Err("chan: malformed channel handle".into()),
        },
        other => {
            return Err(format!(
                "chan: expected a channel (from chan.new), got {}",
                other.type_name()
            )
            .into());
        }
    };
    channels()
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| "chan: unknown channel handle".into())
}

fn register(tx: Tx, rx: Receiver<Value>) -> Value {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    channels().lock().unwrap().insert(
        id,
        Arc::new(Chan {
            tx: Mutex::new(Some(tx)),
            rx: Mutex::new(rx),
        }),
    );
    handle(id)
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
