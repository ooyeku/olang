"""Write a crates.io copy of olang's committed tree: the `olang` package
published as `olangc` (binary and library still `olang`), `olang-ods` as
`olangc-ods`. The repository's manifests are not touched."""
import re, subprocess, sys, pathlib, shutil
src, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
if out.exists(): shutil.rmtree(out)
out.mkdir(parents=True)
subprocess.run(f"git -C {src} archive HEAD | tar -x -C {out}", shell=True, check=True)
ver = re.search(r'^version = "([^"]+)"', (out/"Cargo.toml").read_text(), re.M).group(1)

ods = out/"olang-ods/Cargo.toml"
t = ods.read_text()
t = t.replace('name = "olang-ods"', 'name = "olangc-ods"', 1)
t = re.sub(r'^version = "[^"]+"', f'version = "{ver}"', t, count=1, flags=re.M)
t = t.replace('[package]\n', '[package]\n', 1)
if "\n[lib]" not in t:
    t = t.replace("\n[dependencies]", '\n[lib]\nname = "olang_ods"\n\n[dependencies]', 1)
t = re.sub(r'^\[package\]\n', '[package]\nhomepage = "https://olang.dev"\n', t, count=1, flags=re.M)
ods.write_text(t)

m = out/"Cargo.toml"
t = m.read_text()
t = t.replace('name = "olang"\n', 'name = "olangc"\n', 1)
t = re.sub(r'^members = \[.*\]', 'members = [".", "olang-ods"]', t, count=1, flags=re.M)
t = re.sub(r'^description = .*$', 'description = "olang, a functional language for real work: pipelines, pattern matching and immutable data, with data frames, an HTTP server and SQLite in the standard library, and a bytecode VM and JIT. Installs the `olang` command."', t, count=1, flags=re.M)
t = t.replace('repository = "https://github.com/ooyeku/olang"', 'repository = "https://github.com/ooyeku/olang"\nhomepage = "https://olang.dev"\ndocumentation = "https://olang.dev/book/"\ninclude = ["src/**/*", "build.rs", "grammar.pest", "README.md", "LICENSE", "Cargo.toml"]', 1)
t = t.replace('olang-ods = { path = "olang-ods",', f'olang-ods = {{ package = "olangc-ods", version = "={ver}", path = "olang-ods",', 1)
# the binary and the library keep their names
t = t.replace("\n[features]", '\n[lib]\nname = "olang"\npath = "src/lib.rs"\n\n[[bin]]\nname = "olang"\npath = "src/main.rs"\n\n[features]', 1)
m.write_text(t)
print(ver)
