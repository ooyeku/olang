//! `asset` — the files a package ships beside its code (fonts, images,
//! data), read by the package's name: from its directory when the program
//! runs from source, and from the application itself when it was built
//! with `olang build --app` (the package's `olang.toml` names them under
//! `assets`).

use crate::ast::Value;
use std::sync::Arc;

pub fn create_asset_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
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
        type_name: "Module".into(),
        fields: Arc::new(module).into(),
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
    let at = root.join(&path);
    // On disk (a program run from source) the file is where the system
    // takes it, links followed, and that must be inside the package too:
    // a package can ship a link (`leak -> ~/.ssh/id_ed25519`), and
    // `asset.read` answered the file it led to. A built application's
    // files are in its own section and have no links.
    if let (Ok(real_root), Ok(real)) = (std::fs::canonicalize(&root), std::fs::canonicalize(&at))
        && !real.starts_with(&real_root)
    {
        return Err(format!(
            "asset.{what}: \"{path}\" leads outside the package (to {})",
            real.display()
        ));
    }
    // Only what the package declares under `assets`, from source as in a
    // built application (which carries nothing else): without this a
    // dependency held to `fs = false` read the app's `olang.toml`, `.env`
    // or any other package's files (OL-619).
    let declared = crate::pkg::manifest::Manifest::load(&root)
        .map(|m| m.package.assets)
        .unwrap_or_default();
    let clean = |p: &str| -> std::path::PathBuf {
        p.split('/')
            .filter(|c| !c.is_empty() && *c != ".")
            .collect()
    };
    let wanted = clean(&path);
    let Some(entry) = declared
        .iter()
        .map(|a| clean(a))
        .find(|a| !a.as_os_str().is_empty() && wanted.starts_with(a))
    else {
        return Err(format!(
            "asset.{what}: \"{path}\" is not one of package \"{package}\"'s assets (its olang.toml declares assets = {declared:?})"
        ));
    };
    // ...and where a link inside an asset folder leads, it stays there.
    if let (Ok(real_entry), Ok(real)) = (
        std::fs::canonicalize(root.join(&entry)),
        std::fs::canonicalize(&at),
    ) && !real.starts_with(&real_entry)
    {
        return Err(format!(
            "asset.{what}: \"{path}\" leads outside the package's assets (to {})",
            real.display()
        ));
    }
    Ok(at)
}

pub fn call_asset_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    // a call made wrongly raises: two strings, a package and a path
    if matches!(name, "read" | "exists" | "path") {
        if args.len() != 2 {
            return Err(crate::stdlib::misuse::arity(
                &format!("asset.{name}"),
                "2",
                args.len(),
            ));
        }
        for (i, which) in [(0, "package"), (1, "path")] {
            if !matches!(args[i], Value::String(_)) {
                return Err(crate::stdlib::misuse::arg_type(
                    &format!("asset.{name}"),
                    which,
                    "a string",
                    &args[i],
                ));
            }
        }
    }
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
