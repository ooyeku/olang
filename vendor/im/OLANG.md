# im 15.1.0, patched

im (https://github.com/bodil/im-rs, MPL-2.0, Bodil Stokke), vendored
from the crates.io release 15.1.0 and used through `[patch.crates-io]`
in olang's Cargo.toml.

One change, read-only: `HashMap::trace_tree` (src/hash/map.rs) walks a
map's tree and names each node with its reference count, through
`Node::trace` (src/nodes/hamt.rs) and `addr`/`strong_count` on the
fake pool's `Arc` and `Rc` (src/fakepool.rs).

olang's cycle collector (src/cycles.rs) needs it. A closure's captured
scope is an `im::HashMap` that shares nodes with the scope it was made
in, so a value's reference count counts the nodes holding it, not the
maps. Without the nodes' own counts the collector cannot tell a node
only the garbage holds from one a live scope shares, and would free a
live cell (OL-570).

im 15.1.0 is the last release. Drop the patch if olang moves to a
persistent map that exposes its nodes.
