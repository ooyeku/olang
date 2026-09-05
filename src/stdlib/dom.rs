//! The `dom` module: olang as a frontend language.
//!
//! Elements are opaque integer handles. Every operation crosses the wasm
//! boundary as a host import the page implements (see src/playground.rs);
//! on native builds each call errors clearly instead. Event handlers are
//! olang functions kept in the persistent browser session's registry and
//! re-entered through olang_dispatch_event.

use crate::ast::Value;
use std::collections::HashMap;

pub fn create_dom_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [
        ("query", 1),
        ("set_text", 2),
        ("get_text", 1),
        ("set_html", 2),
        ("value", 1),
        ("set_value", 2),
        ("on", 3),
        ("fetch", 4),
        ("focus", 1),
        ("set_class", 2),
        // Node-level control
        ("get_attr", 2),
        ("set_attr", 3),
        ("remove_attr", 2),
        ("class_add", 2),
        ("class_remove", 2),
        ("class_toggle", 2),
        ("set_style", 3),
        ("measure", 1),
        ("create", 1),
        ("append", 2),
        ("remove", 1),
        ("scroll_into_view", 1),
        // Time
        ("set_timeout", 2),
        ("set_interval", 2),
        ("clear_interval", 1),
        ("request_frame", 1),
        ("on_frame", 1),
        // Graphics
        ("draw", 2),
        ("draw_points", 4),
        // Structure ordering
        ("insert_before", 3),
        // Routing and storage
        ("push_state", 1),
        ("location", 0),
        ("on_route", 1),
        ("storage_get", 1),
        ("storage_set", 2),
        ("state_get", 1),
        ("state_set", 2),
        ("storage_remove", 1),
        // Workers (page side) and their in-worker mirrors
        ("worker", 1),
        ("worker_send", 2),
        ("available", 0),
        ("worker_on", 2),
        ("worker_close", 1),
        ("post", 1),
        ("on_message", 1),
        ("fetch_json", 4),
        ("request", 4),
        // The page's own state: its color scheme, what has focus, a
        // confirmation, and a picked file's contents.
        ("prefers_dark", 0),
        ("active_id", 0),
        ("confirm", 1),
        ("read_file", 2),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("dom.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn call_dom_function(
    name: &str,
    _args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    // The one dom function that exists everywhere: the honest answer
    // to "am I in a browser?", so isomorphic code can degrade
    // gracefully (render a static preview, skip event wiring) instead
    // of trapping.
    if name == "available" {
        return Ok(Value::Boolean(false));
    }
    Err(format!(
        "dom.{}: the dom module is only available in the browser (wasm build)",
        name
    )
    .into())
}

#[cfg(target_arch = "wasm32")]
pub fn call_dom_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    if name == "available" {
        return Ok(Value::Boolean(true));
    }
    crate::playground::dom_call(name, args)
}
