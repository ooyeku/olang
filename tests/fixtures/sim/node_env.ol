// What a node sees of itself: its argv, its environment, its clock.
let role = match os.get_env("ROLE") { Ok(r) => r, Err(e) => "none" }
println(to_string(os.args()) + " " + role + " " + to_string(time.now_ms()))
