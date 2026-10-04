//! `asset` — the files a package ships beside its code (fonts, images,
//! data), read by the package's name: from its directory when the program
//! runs from source, and from the application itself when it was built
//! with `olang build --app` (the package's `olang.toml` names them under
//! `assets`).

use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub fn create_asset_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [("read", 2), ("exists", 2), ("path", 2)] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("asset.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

fn s(v: &Value, what: &str) -> Result<String, String> {
    match v {
        Value::String(t) => Ok(t.to_string()),
        other => Err(format!(
            "asset.{what}: expected a String, got {}",
            other.type_name()
        )),
    }
}

fn located(args: &[Value], what: &str) -> Result<std::path::PathBuf, String> {
    if args.len() != 2 {
        return Err(format!("asset.{what}(package, path) takes two arguments"));
    }
    let package = s(&args[0], what)?;
    let path = s(&args[1], what)?;
    if path.starts_with('/') || path.split('/').any(|p| p == "..") {
        return Err(format!(
            "asset.{what}: \"{path}\" must stay inside the package"
        ));
    }
    let root = crate::vfs::package_root(&package).ok_or_else(|| {
        format!("asset.{what}: no package \"{package}\" is known (it is neither this project nor one of its dependencies)")
    })?;
    Ok(root.join(path))
}

pub fn call_asset_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "read" => Ok(
            match located(&args, "read").and_then(|p| {
                crate::vfs::read_bytes(&p).map_err(|e| format!("asset.read: {}: {e}", p.display()))
            }) {
                Ok(bytes) => Value::Ok(Box::new(crate::stdlib::bytes::to_value(bytes))),
                Err(e) => Value::Err(Box::new(Value::String(Arc::new(e)))),
            },
        ),
        "exists" => Ok(Value::Boolean(
            located(&args, "exists")
                .map(|p| crate::vfs::is_file(&p))
                .unwrap_or(false),
        )),
        "path" => Ok(Value::String(Arc::new(
            located(&args, "path")?.to_string_lossy().to_string(),
        ))),
        other => Err(format!("asset.{other} is not a function").into()),
    }
}
