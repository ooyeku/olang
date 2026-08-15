// capabilities — a malicious-dependency demo.
//
// The same application ships twice, differing only in one manifest. Both
// use a third-party `analytics` package whose `report` function is a
// backdoor: it reads a local secret file. In `unguarded/`, analytics
// inherits the app's full filesystem access — the default in every
// mainstream package manager, and the world of the event-stream and xz
// supply-chain attacks. In `guarded/`, the app's olang.toml grants
// analytics `fs = false`, so the same backdoor dies at the capability gate.
//
// This narrator runs both variants in a fresh `olang` subprocess, shows the
// contrast, and checks that it came out as expected. Run it from here:
//   olang main.ol

let olang = unwrap(os.exe_path())
let root = unwrap(os.cwd())

fn run_variant(name) = {
    let dir = root + "/" + name
    unwrap(os.exec(olang, ["main.ol"], #{ "cwd": dir }))
}

fn rule(title) = {
    println("")
    println("──────────────────────────────────────────────────────────")
    println("  " + title)
    println("──────────────────────────────────────────────────────────")
}

// The secret both apps sit next to. Neither app's own code hands it to
// anyone; analytics reaches for it on its own.
println("The app depends on `analytics`, a package added for one pure")
println("helper. Buried in it is a backdoor that reads secret.txt.")

rule("1. unguarded — analytics inherits full access (the status quo)")
let un = run_variant("unguarded")
print(un.stdout)
let stolen = str.contains(un.stdout, "STOLEN by analytics")
if stolen => println(">> The dependency read the secret. In Node or Python, this ships.")
else => println(">> (unexpected: the theft did not occur)")

rule("2. guarded — the manifest grants analytics fs = false")
let gu = run_variant("guarded")
print(gu.stdout)
if str.contains(gu.stdout, "loaded secret.txt") =>
    println(">> The app's OWN file read still worked — the gate keys on the")
else => println("")
println("   caller, not the call.")
print(gu.stderr)
let blocked = str.contains(gu.stderr, "capability 'fs' denied")
    && str.contains(gu.stderr, "dependency 'analytics'")
if blocked => println(">> Same backdoor, same code — refused at the gate.")
else => println(">> (unexpected: the backdoor was not blocked)")

rule("what happened")
println("Per-dependency attenuation: a dependency can be granted LESS than")
println("the application, never more (the grant can only shrink). No process")
println("flag, no sandbox wrapper — the rule lives in the app's olang.toml,")
println("and the runtime enforces it at the one boundary effects pass through.")

// Self-check so a regression fails the examples harness.
let ok = stolen && blocked && un.code == 0 && gu.code != 0
println("")
if ok => println("demo ok")
else => {
    println("demo FAILED: expected unguarded theft to succeed and guarded to be blocked")
    os.exit(1)
}
