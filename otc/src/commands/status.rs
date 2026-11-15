//! `otc status` (and `otc` alone): one overview (OL-585). The olang in
//! use and why, the project here and its pin, its dependencies with
//! anything drifted, the shelf's size, and at most three suggested
//! commands. Reads only: no network.

use super::{ctx, emit};
use anyhow::Result;

pub fn run() -> Result<()> {
    let now = super::update::now_running();
    let default = now["default"].as_str().map(str::to_string);
    let project = super::toolchains::project_note();
    let root = std::env::current_dir()
        .ok()
        .and_then(|d| olang::pkg::manifest::Manifest::find_root(&d));
    let deps = root
        .as_deref()
        .map(super::project::dep_reports)
        .unwrap_or_default();
    let shelf = olang::pkg::shelf::Shelf::load();
    let shelf_size = super::doctor::dir_size(&olang::pkg::shelf::Shelf::home());
    let mut next: Vec<String> = Vec::new();

    // what needs doing, most pressing first
    if olang::pkg::shelf::Shelf::old_layout() {
        next.push("otc doctor --fix   (moves the shelf to this olang's layout)".to_string());
    }
    if now["olang"].is_null() {
        next.push("otc use latest   (installs olang and puts it on PATH)".to_string());
    } else if let Some(fix) = now["fix"].as_str() {
        next.push(format!("{fix}   (so `olang` runs the default)"));
    }
    if let Some(p) = &project
        && let Some(hint) = p["install"].as_str()
    {
        next.push(format!("{hint}   (this project's pin)"));
    }
    if deps.iter().any(|d| d.problem.is_some()) {
        next.push("otc install   (resolves what is missing or drifted)".to_string());
    }
    if root.is_some() && !deps.is_empty() && next.len() < 3 {
        next.push("otc outdated   (newer versions of the dependencies)".to_string());
    }
    if root.is_none() && next.len() < 3 {
        next.push("otc new <name>   (a new project)".to_string());
    }
    if next.len() < 3 {
        next.push("otc toolchain list --available   (every release)".to_string());
    }
    next.truncate(3);

    if ctx().json {
        emit(serde_json::json!({
            "format": 1,
            "kind": "status",
            "olang": now,
            "project": project.as_ref().map(|p| {
                let mut p = p.clone();
                p["dependencies"] = root.as_deref().map(super::project::deps_json)
                    .map(|d| d["dependencies"].clone()).unwrap_or_default();
                p
            }),
            "shelf": {
                "file": olang::pkg::shelf::Shelf::file().display().to_string(),
                "libraries": shelf.as_ref().map(|s| s.libraries.len()).unwrap_or(0),
                "size": shelf_size,
                "error": shelf.as_ref().err().map(|e| e.to_string()),
            },
            "next": next,
        }));
        return Ok(());
    }

    match (now["olang_version"].as_str(), now["olang"].as_str()) {
        (Some(v), Some(at)) => {
            let why = if now["shadowed"].is_null() {
                match &default {
                    Some(d) => format!("the default ({d}), through the shims"),
                    None => "the first olang on PATH".to_string(),
                }
            } else {
                "the first on PATH, ahead of the default".to_string()
            };
            println!("olang {v} runs here: {why}");
            println!("  {at}");
        }
        _ => println!("no olang is on PATH"),
    }
    if let Some(s) = now["shadowed"].as_str() {
        println!("  {s}");
    }
    match (&project, &root) {
        (Some(p), Some(r)) => {
            let name = olang::pkg::manifest::Manifest::load(r)
                .map(|m| format!("{} {}", m.package.name, m.package.version))
                .unwrap_or_default();
            println!();
            println!("project {name} ({})", r.display());
            match (p["pin"].as_str(), p["version"].as_str()) {
                (None, _) => println!("  pins no olang: it runs the one above"),
                (Some(pin), Some(v)) => println!("  pins olang {pin}: it runs {v}"),
                (Some(pin), None) => println!("  pins olang {pin}: nothing installed satisfies it"),
            }
            if deps.is_empty() {
                println!("  no dependencies");
            }
            for d in &deps {
                println!("  {} {}", d.name, d.words());
                if let Some(problem) = &d.problem {
                    println!("    ! {problem}");
                }
            }
        }
        _ => {
            println!();
            println!("no project here (no olang.toml at or above this folder)");
        }
    }
    println!();
    match &shelf {
        Ok(s) => println!(
            "shelf: {} librar{}, {} ({})",
            s.libraries.len(),
            if s.libraries.len() == 1 { "y" } else { "ies" },
            super::doctor::human(shelf_size),
            olang::pkg::shelf::Shelf::home().display()
        ),
        Err(e) => println!("shelf: {e}"),
    }
    println!();
    println!("next:");
    for n in &next {
        println!("  {n}");
    }
    Ok(())
}
