# Olang

[![CI](https://github.com/ooyeku/olang/actions/workflows/ci.yml/badge.svg)](https://github.com/ooyeku/olang/actions/workflows/ci.yml)

Current Status: Very early/Experimental.

**Vision**: A modern, functional programming language with advanced features, high-performance execution, and comprehensive standard library.

## Features

### Core Language
- **REPL-First**: Instant feedback via an advanced interactive REPL with syntax highlighting and error suggestions
- **Functions as Values**: Arrow syntax (`=>`) for lambdas; named and anonymous functions are interchangeable
- **Pipeline Operator**: Infix operator `|>` to thread data through transformations
- **Pattern Matching**: Advanced match expressions with destructuring of enums, tuples, lists, and structs
- **Minimal Syntax**: Optional semicolons, braces only for multi-line blocks, type annotations optional
- **Template Strings**: String interpolation with `${expression}` syntax
- **Bitwise Operations**: Full support for `&`, `|`, `^`, `<<`, `>>` operations
- **Advanced Literals**: Binary (`0b1010`), octal (`0o755`), hex (`0xFF`), raw strings (`r"..."`), character literals (`'a'`)

### Execution
- **Tiered Execution**: A tree-walking interpreter plus an opt-in register-based
  bytecode VM that hot functions are promoted to (`--ovm-tier`). Promotion is
  transparent: anything the VM can't compile keeps running on the interpreter
- **Lazy Evaluation**: Lazy list operations for large datasets
- **Parallel Processing**: Multi-threaded list operations
- **Reference Counting**: Deterministic memory reclamation
- **Performance Monitoring**: Execution statistics via `--ovm-stats`

See [docs/ovm.md](docs/ovm.md) for the architecture, measured speedups, and an
explicit list of what is and isn't implemented.

### Type System
- **Union Types**: `Int | String | Bool`
- **Intersection Types**: `Int & Comparable`
- **Generic Types**: `List<T>`, `Map<K, V>`
- **Result Types**: `Result<T, E>` for error handling
- **Promise Types**: `Promise<T, E>` for async operations
- **Literal Types**: `"admin" | "user"`, `42`

### Standard Library (11 Modules)
- **fs**: File system operations (read, write, copy, move, etc.)
- **http**: HTTP client (requests, headers, JSON); `http.serve` is a
  placeholder, not a working server
- **math**: Comprehensive mathematical functions
- **random**: Random number generation and distributions
- **dates**: Date/time parsing, formatting, and arithmetic
- **json**: JSON parsing, manipulation, and serialization
- **csv**: CSV file reading, writing, and manipulation
- **base64**: Base64 encoding and decoding
- **crypto**: Cryptographic operations (hashing, encryption, etc.)
- **os**: Operating system utilities
- **testing**: Assertions for the built-in test framework

### Development Features
- **Testing Framework**: Built-in test declarations and assertions
- **Module System**: Share declarations and use imports
- **Error Handling**: Try-catch expressions and Result types
- **Async/Await**: Full async programming support
- **Help System**: Interactive help with fuzzy search and tutorials
- **REPL Shell Integration**: Run shell commands and navigate the filesystem
  without leaving the REPL, with TAB completion for paths and identifiers

## Installation

### Prerequisites

- Rust 1.70+ and Cargo
- Git

### Quick Setup (Recommended)

For the easiest installation experience, use our cross-platform setup scripts:

```bash
git clone https://github.com/ooyeku/olang.git
cd olang
./setup
```

This will:
- Install both `olang` and `otc` using `cargo install`
- Create `~/.olang/` directory with unified executables
- Copy all example files to `~/.olang/examples/`
- Configure your PATH automatically
- Test the installation

For Windows users, run `setup.bat` or `.\setup.ps1` instead.

### Building from Source

```bash
git clone https://github.com/ooyeku/olang.git
cd olang
make build
```

The binary will be available at `target/release/olang`.

### Manual Install

```bash
make install
```

## Usage

### REPL Mode

Start the interactive REPL:

```bash
olang
```

Example REPL session:

```olang
olang> let inc = (x) => x + 1
olang> inc(5)
6
olang> [1,2,3] |> map(inc) |> filter((n) => n % 2 == 1)
[3]
olang> match Ok(42) { Ok(v) => println(v), Err(e) => println(e) }
42
olang> let name = "ann"
olang> let score = 91
olang> `Hello ${name}, your score is ${score}%`
"Hello ann, your score is 91%"
olang> :help map
```

#### Shell Commands

Run shell commands without leaving the REPL. `cd` changes the REPL's own
working directory, so relative paths in `fs.` calls and later commands follow
along:

```
olang> :pwd                        # print working directory
olang> :cd src                     # change directory (supports ~)
olang> :ls -la                     # list files
olang> :sh cat data.csv | head     # any shell command; pipes and globs work
olang> !git status                 # ! is shorthand for :sh
```

#### TAB Completion

Press TAB to complete:

- REPL commands — `:p` completes to `:pwd`, `:profile`, ...
- File paths after shell commands (`!`, `:sh`, `:cd`, `:ls`, `:run`) **and
  inside string literals**, so `fs.read("src/ma` completes to `src/main.rs`
- Function and variable names, including stdlib functions and bindings you
  defined earlier in the session

#### Other REPL Commands

```
:env                 # show current environment
:type <expr>         # inspect the type of an expression
:history             # command history (:!<n> re-runs an entry)
:time <expr>         # time an expression
:help <topic>        # documentation, tutorials, and fuzzy search
:clear               # clear screen or environment
```

### File Execution

Execute an Olang file:

```bash
olang script.ol
```

### Advanced Options

```bash
# Batch mode (no REPL)
olang --batch script.ol

# Verbose output
olang --verbose

# Compile hot functions to bytecode after 50 calls (or --ovm-tier=N).
# Note the '=': a bare --ovm-tier would otherwise swallow the filename.
olang --ovm-tier script.ol
olang --ovm-tier=10 script.ol

# Disable OVM (use classic interpreter only)
olang --no-ovm script.ol

# Show execution statistics, including tier promotions
olang --ovm-stats script.ol

# Control parallelism for list operations
olang --enable-parallel script.ol
olang --ovm-parallelism 4 script.ol

# Enable tracing for debugging
olang --trace
```

## Language Syntax

### Functions and Lambdas

```olang
// Named function
fn add(x: Int, y: Int) -> Int = x + y

// Function with default parameters
fn greet(name: String = "World") = "Hello, " + name

// Anonymous function
nums |> map((n) => n * 2)

// Multi-line function
fn factorial(n: Int) -> Int = {
    if n <= 1 => 1
    else => n * factorial(n - 1)
}

// Named arguments
let result = process_data("file.txt", format: "csv", compress: true)
```

### Pipeline Operator

```olang
data
  |> filter((x) => x % 2 == 0)
  |> map((x) => x * x)
  |> sum()
  |> println
```

### Pattern Matching

```olang
// Result patterns (note: `error` is a reserved keyword, so bind another name)
match result {
  Ok(value) => println(value),
  Err(e) => println(e),
}

// List patterns with rest
match list {
  [head, ...tail] => println(head),
  [] => println("Empty list"),
}

// Struct patterns (every field must be named — there is no `..` rest form)
match user {
  User { name, age } => `${name} (${age})`,
}

// Guards and ranges
match n {
  0 => "zero",
  1..10 => "small",
  x if x > 100 => "large",
  _ => "medium",
}
```

### Advanced Literals

```olang
// Numeric literals
let binary = 0b1010      // 10
let octal = 0o755        // 493
let hex = 0xFF           // 255

// String literals
let regular = "Hello \"World\""
let raw = r"C:\Users\Name\file.txt"
let template = `Hello ${name}!`

// Character literals (exactly one character; no escape sequences)
let char = 'a'
```

### Type System

```olang
// Type annotations
let count: Int = 42
let flexible: Int | String = "hello"
let user_data: Map<String, String> = #{"name": "Alice"}

// Custom types
type User = struct {
    name: String,
    age: Int,
    email: String
}

type Color = enum {
    Red,
    Green,
    Blue,
    RGB(Int, Int, Int)
}

// Error types — a variant's payload may be () or an anonymous struct,
// but not a named type
error NetworkError {
    Timeout,
    ConnectionFailed: (),
    InvalidResponse: {
        status: Int,
        message: String
    }
}
```

### Async/Await

```olang
// Async function
async fn fetch_data(url: String) -> Promise<String, String> = {
    // Implementation
    Promise.resolve("data")
}

// Await usage
let data = await fetch_data("https://api.example.com")

// Concurrent operations
let results = await Promise.all([
    fetch_user(1),
    fetch_user(2),
    fetch_user(3)
])
```

### Testing

```olang
test "addition test" {
    let result = add(2, 3)
    assert_eq(result, 5)
}

test "string concatenation" {
    let result = "Hello" + " " + "World"
    assert_eq(result, "Hello World", "String concatenation failed")
}
```

## Standard Library Examples

> **Note:** Standard library functions return `Result` values (`Ok(...)` /
> `Err(...)`). Use `unwrap(...)`, the `?` operator, or `match` to get at the
> value — the examples below use `unwrap` for brevity.

### File System Operations

```olang
// Read and write files
let content = unwrap(fs.read_file("input.txt"))
fs.write_file("output.txt", content)

// Directory operations
let files = unwrap(fs.list_dir("."))
println(len(files))
```

### HTTP Operations

```olang
// HTTP client
let response = unwrap(http.get("https://api.example.com/data"))
let data = unwrap(json.parse(response.body))

// POST with a JSON body
let created = http.post("https://api.example.com/items", unwrap(json.stringify(data)))
```

Server support (`http.serve`) is not implemented — it currently returns a
placeholder message rather than binding a port.

### Data Processing

```olang
// CSV processing — parse_with_headers takes CSV *text*, not a path
let text = unwrap(fs.read_file("data.csv"))
let rows = unwrap(csv.parse_with_headers(text))
println(len(rows))

// JSON manipulation (note: strings use double quotes)
let user = unwrap(json.parse("{\"name\": \"Alice\", \"age\": 30}"))
let encoded = unwrap(json.stringify(user))
```

### Cryptography

```olang
// Hashing
let hash = unwrap(crypto.sha256("password"))

// Password hashing and verification
let stored = unwrap(crypto.hash_password("secret"))
let ok = unwrap(crypto.verify_password("secret", stored))

// Encryption — the key is a 32-byte hex string
let key = unwrap(crypto.random_hex(32))
let encrypted = unwrap(crypto.encrypt_aes("secret data", key))
let decrypted = unwrap(crypto.decrypt_aes(encrypted, key))

// RSA signing
let keys = unwrap(crypto.generate_key_pair())
let signature = unwrap(crypto.sign_data("message", keys.private_key))
let valid = unwrap(crypto.verify_signature("message", signature, keys.public_key))
```



## Development

### Running Tests

```bash
cargo test
cargo test -- --nocapture  # Show output
```

Two suites guard the bytecode tier specifically:

```bash
# The VM must produce identical results to the interpreter
cargo test --test bytecode_differential_test

# Whole programs must behave the same with and without promotion
cargo test --test bytecode_tier_test
```

If you extend the bytecode VM, extend the differential suite in the same
change — the interpreter defines the language, so any divergence is a VM bug.

### Benchmarks

```bash
# Interpreter benchmarks (ten representative programs)
cargo bench

# Interpreter vs. bytecode tier on the same functions
cargo run --release --example tier_compare
```

### Code Formatting

```bash
cargo fmt
```

### Linting

```bash
cargo clippy
```

### Performance Testing

```bash
# Compare execution tiers
cargo run --release --example tier_compare

# Inspect tier promotions in a real program
olang --ovm-tier --ovm-stats examples/benchmark.ol
```

## Performance

Enabling `--ovm-tier` promotes hot functions to the bytecode VM. Measured on an
Apple Silicon laptop, release build:

| Workload | Interpreter | Bytecode tier | Speedup |
|---|---|---|---|
| `fib(20)` (recursive calls) | 837 ms | 6.9 ms | ~121x |
| 100k-iteration `while` loop | 27.9 ms | 4.2 ms | ~6.6x |
| 300k-iteration loop across 4 functions | 38.9 s | 0.40 s | ~97x |

End to end through the CLI, `fib(27)` runs in **23.6 s** interpreted and
**0.20 s** with the tier enabled, producing identical output.

Reproduce with `cargo run --release --example tier_compare`. Call-heavy code
benefits most, because a promoted recursive function runs its whole call tree
inside the VM.

Not every function qualifies — the VM supports a subset of the language, and
anything outside it stays on the interpreter. See
[docs/ovm.md](docs/ovm.md#known-limitations) for the current boundaries.

## Roadmap

### Current Status (v0.23, experimental)
- Core language features implemented
- Standard library (11 modules)
- Tree-walking interpreter with an opt-in bytecode tier for hot functions,
  covered by differential tests against the interpreter
- Reference-counted value model
- Type system with unions, intersections, and generics
- Async/await runtime
- Testing framework
- REPL with help system, tutorials, shell integration, and TAB completion

### Next Phase
- **Widen the bytecode tier**: `match` and `for` support; more builtins
- **Enable the tier by default** once coverage justifies it
- **Real JIT codegen** to replace the disabled Cranelift scaffolding
- Package manager and ecosystem
- WebAssembly target

Known gaps are tracked explicitly in
[docs/ovm.md](docs/ovm.md#not-implemented) rather than implied to be finished.

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the MIT License—see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- [Pest](https://pest.rs/) for parsing
- [Rustyline](https://github.com/kkawakam/rustyline) for REPL functionality
- [Serde](https://serde.rs/) for serialization
- [Cranelift](https://github.com/bytecodealliance/wasmtime/tree/main/cranelift) for the (in-progress) JIT backend
- [Crossbeam](https://github.com/crossbeam-rs/crossbeam) for concurrent data structures

## Examples Directory

Check out the `examples/` directory for comprehensive sample programs demonstrating Olang's features:

- `union_types.ol` - Advanced pattern matching and discriminated unions
- `string_interpolation.ol` - Template strings and advanced literals
- `simple_sales.ol` - Data processing with pipelines
- `crypto_test.ol` - Cryptographic operations
- `dates.ol` - Date/time manipulation
- `loops.ol`, `fast_loops.ol` - Loop forms and performance comparison
- `benchmark.ol` - Mixed workload used for performance checks
- `base_utils.ol`, `extended_utils.ol`, `stats_module.ol` - Module system
- And more in the directory.

## Documentation

- [Syntax Reference](docs/syntax.md) - Complete language syntax
- [OVM Architecture](docs/ovm.md) - Execution tiers, the bytecode VM, measured
  performance, and current limitations

Installation instructions are in [Installation](#installation) above.
- [Standard Library](docs/stdlib.md) - API reference for all modules 