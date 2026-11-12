"""Write a crates.io copy of olang's committed tree: the `olang` package
published as `olangc` (binary and library still `olang`), `olang-ods` as
`olangc-ods`, and olang's patched copy of `im` (vendor/im, which the
cycle collector needs: vendor/im/OLANG.md) as `olangc-im`, since a
`[patch]` does not reach a crate installed from crates.io. The
repository's manifests are not touched."""
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

# olang's copy of im, under a name of its own; the library is still `im`
im = out/"vendor/im/Cargo.toml"
t = im.read_text()
t = t.replace('name = "im"\n', 'name = "olangc-im"\n', 1)
t = re.sub(r'^version = "[^"]+"', f'version = "{ver}"', t, count=1, flags=re.M)
t = re.sub(r'^description = .*$', 'description = "olang\'s copy of im 15.1.0 (immutable collection datatypes by Bodil Stokke) with the tracing its cycle collector needs. Use im itself unless you are building olang."', t, count=1, flags=re.M)
t = re.sub(r'^homepage = .*$', 'homepage = "https://olang.dev"', t, count=1, flags=re.M)
t = re.sub(r'^documentation = .*\n', '', t, count=1, flags=re.M)
t = re.sub(r'^repository = .*$', 'repository = "https://github.com/ooyeku/olang"', t, count=1, flags=re.M)
assert '[lib]\npath = "./src/lib.rs"' in t
t = t.replace('[lib]\npath = "./src/lib.rs"', '[lib]\nname = "im"\npath = "./src/lib.rs"', 1)
im.write_text(t)
(out/"vendor/im/Cargo.toml.orig").unlink(missing_ok=True)
# im's build script tells im (thread-safe) from im-rc by the package's name
b = out/"vendor/im/build.rs"
t = b.read_text()
assert '        "im" => {' in t
b.write_text(t.replace('        "im" => {', '        "im" | "olangc-im" => {', 1))

m = out/"Cargo.toml"
t = m.read_text()
t = t.replace('name = "olang"\n', 'name = "olangc"\n', 1)
t = re.sub(r'^members = \[.*\]', 'members = [".", "olang-ods", "vendor/im"]', t, count=1, flags=re.M)
assert 'im = { version = "15.1", features = ["serde"] }' in t and 'im = { path = "vendor/im" }\n' in t
t = t.replace('im = { version = "15.1", features = ["serde"] }', f'im = {{ package = "olangc-im", version = "={ver}", path = "vendor/im", features = ["serde"] }}', 1)
t = t.replace('im = { path = "vendor/im" }\n', '', 1)
t = re.sub(r'^description = .*$', 'description = "olang is a programming language for small tools, data work and web apps. Installs the `olang` command."', t, count=1, flags=re.M)
t = t.replace('repository = "https://github.com/ooyeku/olang"', 'repository = "https://github.com/ooyeku/olang"\nhomepage = "https://olang.dev"\ndocumentation = "https://olang.dev/book/"\ninclude = ["src/**/*", "build.rs", "grammar.pest", "README.md", "LICENSE", "Cargo.toml"]', 1)
t = t.replace('olang-ods = { path = "olang-ods",', f'olang-ods = {{ package = "olangc-ods", version = "={ver}", path = "olang-ods",', 1)
# the binary and the library keep their names
t = t.replace("\n[features]", '\n[lib]\nname = "olang"\npath = "src/lib.rs"\n\n[[bin]]\nname = "olang"\npath = "src/main.rs"\n\n[features]', 1)
m.write_text(t)
print(ver)
