//! `otc new` — scaffold a project the current toolchain understands: an
//! `olang.toml` in the `olang::pkg` manifest format plus working source.
//!
//! Two shapes, matching how the module resolver finds code:
//! - application (default): entry point at `src/main.ol`, run directly.
//! - library (`--lib`): public API in `index.ol` at the package root, which
//!   is where `use <name>` looks when another package depends on this one,
//!   over implementation modules in `lib/`.
//!
//! The library shape is deliberately two files rather than one. `index.ol`
//! has to sit at the root because that is where the resolver looks, but a
//! library that grows puts everything there by default and ends up with
//! one long file and no seam. Starting with `lib/` present makes the
//! second module an obvious addition rather than a refactor, and it makes
//! `index.ol` what it should be: the list of what the package exports.

use anyhow::{Context, Result};
use olang::pkg::manifest::{Manifest, PackageMeta};
use semver::Version;
use std::fs;
use std::path::Path;

pub fn execute(name: String, lib: bool, web: bool, verbose: bool) -> Result<()> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(anyhow::anyhow!(
            "Project name must be a plain directory name, got '{}'",
            name
        ));
    }
    if lib && web {
        return Err(anyhow::anyhow!(
            "--lib and --web are different shapes; pick one"
        ));
    }
    let root = Path::new(&name);
    if root.exists() {
        return Err(anyhow::anyhow!("Directory '{}' already exists", name));
    }

    fs::create_dir(root).with_context(|| format!("Failed to create directory '{}'", name))?;

    let manifest = Manifest {
        package: PackageMeta {
            name: name.clone(),
            version: Version::new(0, 1, 0),
            description: None,
            authors: Vec::new(),
            license: None,
        },
        dependencies: Default::default(),
        capabilities: None,
    };
    manifest
        .save(root)
        .map_err(|e| anyhow::anyhow!("Failed to write olang.toml: {}", e))?;

    if web {
        return scaffold_web(root, &name, verbose);
    }

    let sources: Vec<(&str, String)> = if lib {
        fs::create_dir(root.join("lib")).context("Failed to create lib directory")?;
        vec![
            ("index.ol", lib_index_source(&name)),
            ("lib/greet.ol", lib_module_source(&name)),
        ]
    } else {
        fs::create_dir(root.join("src")).context("Failed to create src directory")?;
        vec![("src/main.ol", app_main_source(&name))]
    };

    // The scaffold's promise is that it always generates working code —
    // refuse to write a program the current parser rejects.
    for (rel, source) in &sources {
        olang::parser::Parser::new()
            .parse(source)
            .map_err(|e| anyhow::anyhow!("Generated {} does not parse: {}", rel, e))?;
        fs::write(root.join(rel), source).with_context(|| format!("Failed to write {}", rel))?;
    }
    let source_rel = sources
        .iter()
        .map(|(rel, _)| *rel)
        .collect::<Vec<_>>()
        .join(", ");

    let readme = if lib {
        lib_readme_source(&name)
    } else {
        app_readme_source(&name)
    };
    fs::write(root.join("README.md"), readme).context("Failed to write README.md")?;
    fs::write(root.join(".gitignore"), GITIGNORE).context("Failed to write .gitignore")?;

    if verbose {
        println!("Created olang.toml, {}, README.md, .gitignore", source_rel);
    }

    println!(
        "Created new olang {}: {}",
        if lib { "library" } else { "project" },
        name
    );
    println!();
    println!("Get started:");
    println!("   cd {}", name);
    if lib {
        println!("   olang test           # run the library's tests");
        println!();
        println!("Use it from another package:");
        println!(
            "   otc add ../{}  # or: otc lib add ../{} then otc add by name",
            name, name
        );
    } else {
        println!("   olang src/main.ol");
    }
    Ok(())
}

/// The `--web` shape: the full-stack starter (see `super::web`). Every
/// generated `.ol` file is parse-checked before writing, same promise
/// as the other shapes.
fn scaffold_web(root: &Path, name: &str, verbose: bool) -> Result<()> {
    for (rel, contents) in super::web::files(name) {
        let path = root.join(&rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        if super::web::parse_checked(&rel) {
            olang::parser::Parser::new()
                .parse(&contents)
                .map_err(|e| anyhow::anyhow!("Generated {} does not parse: {}", rel, e))?;
        }
        fs::write(&path, contents).with_context(|| format!("Failed to write {}", rel))?;
        if verbose {
            println!("  wrote {}", rel);
        }
    }
    fs::write(
        root.join(".gitignore"),
        "# OS noise
.DS_Store
# runtime artifacts
*.db
static/olang_playground.wasm
",
    )
    .context("Failed to write .gitignore")?;

    let wasm_note = match super::web::wasm_runtime() {
        Some(found) => {
            fs::copy(&found, root.join("static/olang_playground.wasm"))
                .with_context(|| format!("Failed to copy {}", found.display()))?;
            format!("   (wasm runtime copied from {})", found.display())
        }
        None => "   NOTE: no olang_playground.wasm found on this machine — the API works,
   the browser frontend needs it. README.md says how to supply one."
            .to_string(),
    };

    println!("Created new olang web app: {}", name);
    println!();
    println!("Get started:");
    println!("   cd {}", name);
    println!("   olang main.ol        # http://127.0.0.1:7500");
    println!("{}", wasm_note);
    Ok(())
}

fn app_main_source(name: &str) -> String {
    format!(
        r#"// {name}: a starting point.
println("Hello from {name}!")

let numbers = [1, 2, 3, 4, 5]
let doubled = numbers |> map((n) => n * 2)
println("Doubled: ", doubled)

test "doubling works" {{
    assert_eq(doubled, [2, 4, 6, 8, 10])
}}
"#
    )
}

fn lib_index_source(name: &str) -> String {
    format!(
        r#"// {name} — the public API. This file is what `use {name}` finds,
// so keeping it to imports and re-exports makes the package's surface
// readable in one screen. The implementation lives in lib/.

use lib.greet {{ greet }}

share fn hello(who) = greet(who)
"#
    )
}

fn lib_module_source(name: &str) -> String {
    format!(
        r#"// {name}/lib/greet.ol — an implementation module.
//
// `share` here makes a name visible to the rest of the package; only what
// index.ol re-exports becomes part of the public API.

share fn greet(who) = `Hello, ${{who}}!`

test "greet works" {{
    assert_eq(greet("olang"), "Hello, olang!")
}}
"#
    )
}

fn app_readme_source(name: &str) -> String {
    format!(
        r#"# {name}

A new olang project.

```bash
olang src/main.ol      # run it
olang test             # run the test blocks
olang fmt              # format the source
```

## Dependencies

```bash
otc add ../somelib        # by path
otc lib add ~/code/somelib   # register once on your shelf, then anywhere:
otc add somelib           # by name
```

Then in your code:

```olang
use somelib {{ some_function }}
```
"#
    )
}

fn lib_readme_source(name: &str) -> String {
    format!(
        r#"# {name}

An olang library.

```text
index.ol        the public API — what `use {name}` finds
lib/greet.ol    implementation
```

`index.ol` sits at the package root because that is where the resolver
looks; keeping it to imports and re-exports means the package's surface
reads in one screen. Add modules under `lib/` and re-export from
`index.ol` what callers should see.

```bash
olang test             # run the library's tests
olang fmt              # format the source
```

## Using this library

From the depending package:

```bash
otc add ../{name}       # by path — or register it once:
otc lib add ../{name}
otc add {name}          # then by name, from any project
```

```olang
use {name} {{ greet }}
println(greet("world"))
```
"#
    )
}

const GITIGNORE: &str = "\
# OS noise
.DS_Store
";
