# Olang

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

### High-Performance Execution
- **OVM (Olang Virtual Machine)**: Advanced bytecode VM with JIT compilation via Cranelift
- **Lazy Evaluation**: Memory-efficient lazy evaluation for large datasets
- **Parallel Processing**: Automatic parallelization for list operations
- **Garbage Collection**: Concurrent, generational garbage collection
- **Performance Monitoring**: Built-in performance metrics and profiling

### Type System
- **Union Types**: `Int | String | Bool`
- **Intersection Types**: `Int & Comparable`
- **Generic Types**: `List<T>`, `Map<K, V>`
- **Result Types**: `Result<T, E>` for error handling
- **Promise Types**: `Promise<T, E>` for async operations
- **Literal Types**: `"admin" | "user"`, `42`

### Standard Library (10 Modules)
- **fs**: File system operations (read, write, copy, move, etc.)
- **http**: HTTP client and server functionality
- **math**: Comprehensive mathematical functions
- **random**: Random number generation and distributions
- **dates**: Date/time parsing, formatting, and arithmetic
- **json**: JSON parsing, manipulation, and serialization
- **csv**: CSV file reading, writing, and manipulation
- **base64**: Base64 encoding and decoding
- **crypto**: Cryptographic operations (hashing, encryption, etc.)
- **os**: Operating system utilities

### Development Features
- **Testing Framework**: Built-in test declarations and assertions
- **Module System**: Share declarations and use imports
- **Error Handling**: Try-catch expressions and Result types
- **Async/Await**: Full async programming support
- **Help System**: Interactive help with fuzzy search and tutorials

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
[1,3]
olang> match Some(42) { Some(v) => println(v), None => println("nope") }
42
olang> let template = `Hello ${name}, your score is ${score}%`
olang> :help map
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

# Disable OVM (use classic interpreter only)
olang --no-ovm script.ol

# Show OVM performance statistics
olang --ovm-stats script.ol

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
match value {
  Some(x) => println("Got", x),
  None => println("None"),
}

// List patterns with rest
match list {
  [head, ...tail] => println("Head:", head, "Tail:", tail),
  [] => println("Empty list"),
}

// Struct patterns
match user {
  User { name: "admin", .. } => "Administrator",
  User { name, age } => `${name} (${age})`,
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

// Character literals
let char = 'a'
let newline = '\n'
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

// Error types
error NetworkError {
    Timeout,
    ConnectionFailed: String,
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

### File System Operations

```olang
// Read and write files
let content = fs.read_file("input.txt")
fs.write_file("output.txt", content)

// Directory operations
let files = fs.list_dir(".")
files |> filter((f) => fs.is_file(f)) |> map(println)
```

### HTTP Operations

```olang
// HTTP client
let response = http.get("https://api.example.com/data")
let data = json.parse(response.body)

// HTTP server
http.serve(8080, (req) => {
    http.response(200, "Hello, World!")
})
```

### Data Processing

```olang
// CSV processing
let data = csv.parse_with_headers("data.csv")
let filtered = data |> filter((row) => row.age > 25)

// JSON manipulation
let user = json.parse('{"name": "Alice", "age": 30}')
let name = json.get(user, "name")
```

### Cryptography

```olang
// Hashing
let hash = crypto.sha256("password")
let verified = crypto.verify_bcrypt("password", hash)

// Encryption
let encrypted = crypto.encrypt_aes("secret data", "key")
let decrypted = crypto.decrypt_aes(encrypted, "key")
```



## Development

### Running Tests

```bash
cargo test
cargo test -- --nocapture  # Show output
```

### Benchmarks

```bash
cargo bench
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
# Run performance benchmarks
cargo run --release --example benchmark

# Test OVM performance
olang --ovm-stats examples/benchmark.ol
```

## Performance

Olang features advanced performance optimizations:

- **OVM Bytecode VM**: 2-12x speedup for computation-intensive operations
- **Lazy Evaluation**: 85-92% memory reduction for large datasets
- **Parallel Processing**: Automatic multi-threading for list operations
- **JIT Compilation**: Native code generation for hot functions
- **Memory Management**: Concurrent garbage collection with lazy awareness

## Roadmap

### Current Status (v0.18)
- Core language features complete
- Standard library (10 modules) implemented
- OVM with bytecode VM and JIT compilation
- Advanced type system with unions and intersections
- Async/await runtime
- Testing framework
- Help system with tutorials

### Next Phase
- Advanced fusion optimization
- SIMD vectorization improvements
- Production monitoring and analytics
- WebAssembly target
- Package manager and ecosystem

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
- [Cranelift](https://github.com/bytecodealliance/wasmtime/tree/main/cranelift) for JIT compilation
- [Crossbeam](https://github.com/crossbeam-rs/crossbeam) for concurrent data structures

## Examples Directory

Check out the `examples/` directory for comprehensive sample programs demonstrating Olang's features:

- `union_types.ol` - Advanced pattern matching and discriminated unions
- `string_interpolation.ol` - Template strings and advanced literals
- `sales_analyzer.ol` - Complex data processing with pipelines
- `crypto_test.ol` - Cryptographic operations
- `dates.ol` - Date/time manipulation
- And many more...

## Documentation

- [Setup Guide](SETUP.md) - Installation and setup instructions
- [Syntax Documentation](docs/syntax.md) - Complete language syntax reference
- [OVM Documentation](docs/ovm.md) - Virtual machine architecture and features
- [Standard Library](docs/stdlib.md) - API reference for all modules 