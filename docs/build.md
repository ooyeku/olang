# Olang Build System Design

## Philosophy: "It Just Works"

The Olang build system has one goal: **be invisible**. 
- No configuration files required
- No build scripts needed  
- No complex commands to remember

## Tool Separation

- **`olang`** - The language interpreter and REPL
- **`otc`** - The Olang Tool Chain (build system, package manager, etc.)

This keeps the interpreter lightweight while providing powerful development tools.

## How It Works

Just put your code in a folder and run `otc build`. That's it.

```
my-project/
├── main.ol             # Your code
└── (that's it!)
```

For bigger projects:
```
my-project/
├── main.ol             # Entry point
├── utils.ol            # Other files (auto-imported)
├── tests.ol            # Tests (auto-run with `olang test`)
└── deps.ol             # Dependencies (optional)
```

## Dependencies (Optional)

If you need external packages, just create a `deps.ol` file:

```olang
// deps.ol
use "http@1.0"     // Gets HTTP library version 1.0
use "json"         // Gets latest JSON library  
use "crypto@^2.1"  // Gets crypto 2.1.x
```

That's it. No version conflicts, no lock files, no package.json madness.

## Commands

```bash
otc build           # Builds your code
otc run             # Builds and runs  
otc test            # Runs tests
```

That's all you need to know.

## How It Actually Works

1. **Auto-discovery**: Olang finds your `main.ol` file
2. **Smart imports**: Other `.ol` files are imported automatically based on usage  
3. **Dependency resolution**: If you have `deps.ol`, packages are downloaded once
4. **Fast compilation**: Uses OVM for parallel, incremental builds
5. **Zero config**: Everything just works

## Advanced Usage (If You Really Need It)

Want a release build? `otc build --release`
Want to target WebAssembly? `otc build --wasm`  
Want to publish a package? `otc publish`

## Examples

**Simple script:**
```bash
echo 'println("Hello, World!")' > hello.ol
otc run hello.ol
```

**Web server:**
```olang
// main.ol
use "http"

http.serve(8080, fn(req) {
  http.response("Hello from Olang!")
})
```
```bash
otc run  # Server starts on port 8080
```

**Library with tests:**
```olang
// math.ol  
fn add(a, b) { a + b }

// tests.ol
assert(add(2, 3) == 5)
```
```bash
otc test  # Runs tests automatically
```

That's the entire build system. Simple, fast, and it just works. 