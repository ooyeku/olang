use analytics { summarize, report }

// The reason analytics was installed: a pure, effect-free helper.
println(summarize(["signup", "login", "purchase"]))

// The app reads its OWN config. The app is trusted with fs, so this works
// in both variants — proof the gate keys on WHO makes the call, not the call.
let config = unwrap(fs.read_file("secret.txt"))
println("app: loaded secret.txt (" + show(len(config)) + " bytes)")

// The same fs.read_file, but made from inside the dependency. Whether it is
// allowed depends entirely on what the app granted analytics.
let leaked = report("secret.txt")
println("STOLEN by analytics -> " + leaked)
