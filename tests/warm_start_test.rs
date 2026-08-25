//! Warm start (ovm::warm): a run of a file leaves a tier profile keyed
//! by its source hash; the next run of byte-identical source replays it,
//! compiling and specializing proven-hot functions at declaration time.
//! Profiles are hints — these tests pin that they are written, replayed,
//! keyed by content, and can never change a result.

use std::path::PathBuf;
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn run(script: &PathBuf, warm_dir: &PathBuf) -> String {
    let out = Command::new(olang())
        .arg("run")
        .arg(script)
        .env("OLANG_WARM_DIR", warm_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_run_leaves_a_profile_and_the_next_replays_it() {
    let base = std::env::temp_dir().join(format!("olang_warm_it_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let warm_dir = base.join("warm");
    let script = base.join("hot.ol");
    std::fs::write(
        &script,
        "fn hot(a, b) = {\n\
             let mut acc = 0\n\
             let mut i = 0\n\
             while i < 50000 { acc = (acc + a * i + b) % 1000003 i = i + 1 }\n\
             acc\n\
         }\n\
         let mut total = 0\n\
         for r in range(0, 40) { total = (total + hot(r, r + 1)) % 1000003 }\n\
         println(total)\n",
    )
    .unwrap();

    // Cold run computes and records.
    let cold = run(&script, &warm_dir);
    let profiles: Vec<_> = std::fs::read_dir(&warm_dir)
        .expect("warm dir written")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(profiles.len(), 1, "one profile for one program");
    let text = std::fs::read_to_string(profiles[0].path()).unwrap();
    assert!(text.contains("name = \"hot\""), "{text}");
    assert!(
        text.contains("kinds = [\"Int\", \"Int\"]"),
        "scalar kinds recorded: {text}"
    );

    // Warm run: byte-identical answer.
    let warm = run(&script, &warm_dir);
    assert_eq!(cold, warm, "warm start must never change a result");

    // An edited program is a different key: the stale profile is
    // ignored (still exactly one old file, and the run still works,
    // now writing a second).
    std::fs::write(
        &script,
        "fn hot(a, b) = {\n\
             let mut acc = 0\n\
             let mut i = 0\n\
             while i < 50000 { acc = (acc + a * i + b) % 1000003 i = i + 1 }\n\
             acc\n\
         }\n\
         let mut total = 1\n\
         for r in range(0, 40) { total = (total + hot(r, r + 1)) % 1000003 }\n\
         println(total)\n",
    )
    .unwrap();
    let edited = run(&script, &warm_dir);
    assert_ne!(edited, cold, "the edit changed the program");
    let after: Vec<_> = std::fs::read_dir(&warm_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(after.len(), 2, "a new key for the edited source");

    // A poisoned profile is a hint, not authority: corrupt every file
    // and the program still runs correctly.
    for entry in &after {
        std::fs::write(entry.path(), "functions = 12 nonsense [").unwrap();
    }
    let poisoned = run(&script, &warm_dir);
    assert_eq!(poisoned, edited, "a broken profile must cost nothing");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_built_binary_carries_and_replays_its_warm_profile() {
    // Run once (leaving a sidecar), build (embedding it), then run the
    // built binary against a fresh warm directory — the "new machine"
    // case the embedding exists for. The profile must travel in the
    // bundle meta and the output must be byte-identical.
    let base = std::env::temp_dir().join(format!("olang_warm_build_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let warm_dir = base.join("warm");
    let script = base.join("hot.ol");
    std::fs::write(
        &script,
        "fn hot(a, b) = {\n\
             let mut acc = 0\n\
             let mut i = 0\n\
             while i < 50000 { acc = (acc + a * i + b) % 1000003 i = i + 1 }\n\
             acc\n\
         }\n\
         let mut total = 0\n\
         for r in range(0, 40) { total = (total + hot(r, r + 1)) % 1000003 }\n\
         println(total)\n",
    )
    .unwrap();

    let sidecar_run = run(&script, &warm_dir);

    let bin = base.join("hotbin");
    let out = Command::new(olang())
        .arg("build")
        .arg(&script)
        .arg("-o")
        .arg(&bin)
        .env("OLANG_WARM_DIR", &warm_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let bytes = std::fs::read(&bin).unwrap();
    let hay = String::from_utf8_lossy(&bytes);
    assert!(
        hay.contains("name = \\\"hot\\\"") || hay.contains("name = \"hot\""),
        "the bundle meta must carry the warm profile"
    );

    // A machine that has never run this source: fresh warm dir.
    let fresh = base.join("fresh");
    let out = Command::new(&bin)
        .env("OLANG_WARM_DIR", &fresh)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "built binary failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        sidecar_run,
        "the embedded profile must never change a result"
    );

    let _ = std::fs::remove_dir_all(&base);
}
