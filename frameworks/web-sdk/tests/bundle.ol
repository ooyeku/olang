// The client bundle: the exact artifact the browser loads, held to its
// contract — parses, carries no module syntax, and contains both the
// SDK's browser layer and the application.
use lib.server { bundle_client }

fn demo_client() = {
    // The runner's cwd is the test file's directory; the SDK root is
    // one up.
    let path = if fs.exists("demo/client.ol") => "demo/client.ol"
        else => "../demo/client.ol"
    unwrap(fs.read_file(path))
}

test "the demo's real bundle parses and is module-free" {
    let client = demo_client()
    let b = bundle_client(client)
    assert_eq(is_ok(meta.parse(b)), true)
    // No LOCAL module syntax survives bundling — embedded modules
    // (`use viz`) stay: the browser resolves those natively.
    let lines = str.lines(b)
    assert_eq(len(filter(lines, (l) => str.starts_with(str.trim(l), "use lib."))), 0)
    assert_eq(len(filter(lines, (l) => str.starts_with(str.trim(l), "use web"))), 0)
    assert_eq(len(filter(lines, (l) => str.starts_with(str.trim(l), "share "))), 0)
    // The SDK's browser layer and the app both arrived.
    assert_eq(str.contains(b, "fn mount("), true)
    assert_eq(str.contains(b, "fn unwrap_envelope("), true)
    assert_eq(str.contains(b, "fn view("), true)
    // Test blocks never ship to the browser.
    assert_eq(len(filter(lines, (l) => str.starts_with(l, "test "))), 0)
}

test "bundling is deterministic" {
    let client = demo_client()
    assert_eq(bundle_client(client), bundle_client(client))
}

test "the bundle gate rejects what the parser rejects" {
    // bundle_client's last step is exactly this check — a broken
    // client can never reach the browser as a blank page.
    assert_eq(is_ok(meta.parse("fn broken( = nope")), false)
    assert_eq(is_ok(meta.parse("fn fine(x) = x")), true)
}
