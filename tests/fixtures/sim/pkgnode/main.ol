// A node program in a package: `use greet` resolves through this
// package's olang.toml, as under `olang run`.
use greet { hello }
println(hello(match os.get_env("WHO") { Ok(w) => w, Err(e) => "nobody" }))
