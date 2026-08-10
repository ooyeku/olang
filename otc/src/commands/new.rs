//! `otc new` — scaffold a project the current toolchain understands: an
//! `olang.toml` in the `olang::pkg` manifest format plus working source.
//!
//! Two shapes, matching how the module resolver finds code:
//! - application (default): entry point at `src/main.ol`, run directly.
//! - library (`--lib`): public API in `index.ol` at the package root, which
//!   is where `use <name>` looks when another package depends on this one.

use anyhow::{Context, Result};
use olang::pkg::manifest::{Manifest, PackageMeta};
use semver::Version;
use std::fs;
use std::path::Path;

pub fn execute(name: String, lib: bool, verbose: bool) -> Result<()> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(anyhow::anyhow!(
            "Project name must be a plain directory name, got '{}'",
            name
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
    };
    manifest
        .save(root)
        .map_err(|e| anyhow::anyhow!("Failed to write olang.toml: {}", e))?;

    let (source_rel, source) = if lib {
        ("index.ol", lib_index_source(&name))
    } else {
        fs::create_dir(root.join("src")).context("Failed to create src directory")?;
        ("src/main.ol", app_main_source(&name))
    };

    // The scaffold's promise is that it always generates working code —
    // refuse to write a program the current parser rejects.
    olang::parser::Parser::new()
        .parse(&source)
        .map_err(|e| anyhow::anyhow!("Generated {} does not parse: {}", source_rel, e))?;
    fs::write(root.join(source_rel), source)
        .with_context(|| format!("Failed to write {}", source_rel))?;

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
        println!("   otc pkg add {} --path ../{}", name, name);
    } else {
        println!("   olang src/main.ol");
    }
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
        r#"// {name} — a shared library. `share` marks the public API;
// anything unshared stays private to the package.

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
otc pkg add somelib --path ../somelib          # local path
otc pkg add somelib --git URL --tag v1.0.0     # git
otc pkg install                                # fetch, honoring olang.lock
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

An olang library. The public API lives in `index.ol` — everything marked
`share` is importable by packages that depend on this one.

```bash
olang test             # run the library's tests
olang fmt              # format the source
```

## Using this library

From the depending package:

```bash
otc pkg add {name} --path ../{name}     # or --git URL --tag v1.0.0
otc pkg install
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
