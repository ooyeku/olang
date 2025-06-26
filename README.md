# Olang 

A general-purpose, functional yet pragmatic programming language.  

## Features

- **REPL-First**: Instant feedback via an in-process REPL powered by rustyline
- **Functions as Values**: Arrow syntax (`=>`) for lambdas; named and anonymous functions are interchangeable
- **Pipeline Operator**: Infix operator `|>` to thread data through transformations
- **Pattern Matching**: Lightweight match expression with destructuring of enums, tuples, and lists
- **Minimal Syntax**: Optional semicolons, braces only for multi-line blocks, type annotations optional
- **Rust Ecosystem**: Uses pest for parsing, serde for AST I/O, and future JIT via cranelift

## Installation

### Prerequisites

- Rust 1.70+ and Cargo
- Git

### Building from Source

```bash
git clone https://github.com/ooyeku/olang.git
cd olang
make build
```

The binary will be available at `target/release/olang`.

### Install

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
```

### File Execution

Execute an Olang file:

```bash
olang script.rap
```

### Batch Mode

```bash
olang --batch script.rap
```

### Verbose Mode

```bash
olang --verbose
```

## Language Syntax

### Functions and Lambdas

```olang
// Named function
let add = (x, y) => x + y

// Anonymous function
nums.map((n) => n * 2)

// Multi-line function
let factorial = (n) => {
    if n <= 1 => 1
    else => n * factorial(n - 1)
}
```

### Pipeline Operator

```olang
data
  |> filter((x) => x % 2 == 0)
  |> map((x) => x * x)
  |> println
```

### Pattern Matching

```olang
match value {
  Some(x) => println("Got", x),
  None => println("None"),
}

// List patterns
match list {
  [head, ...tail] => println("Head:", head, "Tail:", tail),
  [] => println("Empty list"),
}
```

### Variables and Assignment

```olang
let x = 42
let message = "Hello, World!"
let numbers = [1, 2, 3, 4, 5]
```

### Control Flow

```olang
// If expressions
if x > 0 => println("Positive")
else => println("Non-positive")

// Blocks
let result = {
    let temp = x * 2
    temp + 1
}
```

## Built-in Functions

### I/O
- `println(value)` - Print with newline
- `print(value)` - Print without newline

### List Operations
- `map(list, function)` - Apply function to each element
- `filter(list, predicate)` - Filter elements by predicate
- `reduce(list, initial, function)` - Reduce list to single value
- `fold(list, initial, function)` - Alias for reduce
- `len(collection)` - Get length of list, string, or tuple
- `head(list)` - Get first element
- `tail(list)` - Get all but first element
- `cons(item, list)` - Prepend item to list

### Type Conversion
- `to_string(value)` - Convert to string
- `to_int(value)` - Convert to integer
- `to_float(value)` - Convert to float

### Utilities
- `range(start, end)` - Create range of integers
- `zip(list1, list2)` - Zip two lists into list of tuples

## Examples

### Functional Programming

```olang
// Fibonacci sequence
let fib = (n) => {
    if n <= 1 => n
    else => fib(n - 1) + fib(n - 2)
}

// List processing
let numbers = range(1, 11)
let result = numbers
  |> filter((n) => n % 2 == 0)
  |> map((n) => n * n)
  |> reduce(0, (acc, n) => acc + n)

println(result) // 220
```

### Pattern Matching

```olang
let process_list = (list) => {
    match list {
        [] => "Empty list",
        [x] => "Single element: " + to_string(x),
        [first, second, ...rest] => "Multiple elements starting with " + to_string(first),
    }
}
```

### Data Transformation

```olang
let data = [
    ("Alice", 25),
    ("Bob", 30),
    ("Charlie", 35)
]

let names = data
  |> map((person) => {
      match person {
          (name, age) => name + " (" + to_string(age) + ")"
      }
  })
  |> map((s) => "Person: " + s)

names |> map(println)
```

## Project Structure

```
olang/
├── Cargo.toml          # Project configuration
├── grammar.pest        # Pest grammar definitions
├── src/
│   ├── main.rs         # CLI entry point
│   ├── lib.rs          # Library root
│   ├── ast.rs          # Abstract syntax tree
│   ├── parser.rs       # Pest-based parser
│   ├── interpreter.rs  # AST evaluator
│   ├── repl.rs         # Interactive REPL
│   ├── builtin.rs      # Built-in functions
│   ├── analyze.rs      # Static analysis
│   └── jit.rs          # JIT compilation (future)
├── examples/           # Example programs
└── tests/              # Test files
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

## Roadmap


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
- [Cranelift](https://github.com/bytecodealliance/wasmtime/tree/main/cranelift) for future JIT compilation

## Examples Directory

Check out the `examples/` directory for more sample programs demonstrating Olang's features. 