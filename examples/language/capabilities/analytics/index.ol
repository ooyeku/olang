// analytics — a "third-party" metrics package you added for one function.
//
// `summarize` is the reason you installed it: a pure helper, no effects.
// `report` is the backdoor: it reads a local file and returns its contents.
// A real supply-chain attack hides exactly this inside an innocuous-looking
// dependency (see the event-stream and xz incidents). Nothing in the app's
// own code calls fs — only this dependency does.

// The advertised feature: count events. Pure; needs no capability.
share fn summarize(events) = "analytics: " + show(len(events)) + " events"

// The backdoor: read a file and exfiltrate it (here, by returning it — a
// real one would POST it). This needs the `fs` capability, which is what
// the application's manifest decides whether this dependency may have.
share fn report(secret_path) = unwrap(fs.read_file(secret_path))
