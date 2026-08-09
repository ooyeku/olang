//! `otc new` — scaffold a project the current toolchain understands: an
//! `olang.toml` in the `olang::pkg` manifest format, a `src/main.ol` entry
//! point, a README, and a `.gitignore`.

use anyhow::{Context, Result};
use olang::pkg::manifest::{Manifest, PackageMeta};
use semver::Version;
use std::fs;
use std::path::Path;

pub fn execute(name: String, verbose: bool) -> Result<()> {
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
    fs::create_dir(root.join("src")).context("Failed to create src directory")?;

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

    let main_ol = main_ol_source(&name);
    // The scaffold's promise is that it always generates working code —
    // refuse to write a program the current parser rejects.
    olang::parser::Parser::new()
        .parse(&main_ol)
        .map_err(|e| anyhow::anyhow!("Generated main.ol does not parse: {}", e))?;
    fs::write(root.join("src/main.ol"), main_ol).context("Failed to write src/main.ol")?;

    fs::write(root.join("README.md"), readme_source(&name)).context("Failed to write README.md")?;
    fs::write(root.join(".gitignore"), GITIGNORE).context("Failed to write .gitignore")?;

    if verbose {
        println!("Created olang.toml, src/main.ol, README.md, .gitignore");
    }

    println!("Created new olang project: {}", name);
    println!();
    println!("Get started:");
    println!("   cd {}", name);
    println!("   olang src/main.ol");
    Ok(())
}

fn main_ol_source(name: &str) -> String {
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

fn readme_source(name: &str) -> String {
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
otc pkg install                                # resolve, write olang.lock
```

Then in your code:

```olang
use somelib {{ some_function }}
```
"#
    )
}

const GITIGNORE: &str = "\
# OS noise
.DS_Store
";
