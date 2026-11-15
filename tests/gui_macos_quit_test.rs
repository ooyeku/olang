//! A quit from outside reaches the program (macOS): the Dock's Quit,
//! AppleScript's `quit app`, a logout or a shutdown is AppKit's
//! `applicationShouldTerminate:`, which winit answered by ending the
//! process with nothing of the program's quit run. A program that holds
//! quits (`gui.hold_quit(true)`, a UI framework's run loop) hears `quit` and the
//! application waits for it: the program's quit finishes (here: a write
//! half a second later) before the process ends, with its status.
//!
//! The terminate request is sent as the system sends it, through
//! `NSRunningApplication.terminate` (osascript's JavaScript bridge), to a
//! real `olang run` with a real window.

#[cfg(all(feature = "gui", target_os = "macos"))]
#[test]
fn an_external_quit_is_the_program_s_to_finish() {
    use std::process::Command;
    use std::time::{Duration, Instant};

    let dir = std::env::temp_dir().join(format!("olang_quit_hold_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mark = dir.join("mark.txt");
    let ready = dir.join("ready.txt");
    std::fs::write(
        dir.join("main.ol"),
        r##"let ev = gui.events()
let w = unwrap(gui.open(#{ "title": "olang quit hold", "size": (240, 120) }))
gui.hold_quit(true)
let mark = unwrap(os.get_env("QUIT_MARK"))
let _r = fs.write_file(unwrap(os.get_env("QUIT_READY")), "up")
let mut going = true
while going {
    match chan.recv_timeout(ev, #{ "timeout_ms": 100 }) {
        Ok(e) => if e.kind == "quit" => {
            let _a = fs.write_file(mark, "quit:" + e.why)
            // the program's quit takes a while (a session written)
            time.sleep(500)
            let _b = fs.write_file(mark, "quit:" + e.why + " done")
            going = false
        },
        Err(_) => ()
    }
}
gui.close(w)
"##,
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["run", "main.ol"])
        .current_dir(&dir)
        .env("QUIT_MARK", &mark)
        .env("QUIT_READY", &ready)
        .spawn()
        .expect("olang run");
    let t0 = Instant::now();
    while !ready.exists() {
        assert!(
            t0.elapsed() < Duration::from_secs(20),
            "the program never opened its window"
        );
        if let Ok(Some(st)) = child.try_wait() {
            panic!("the program ended before the quit: {st:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(500));
    let pid = child.id();
    let script = format!(
        "ObjC.import('AppKit'); $.NSRunningApplication.runningApplicationWithProcessIdentifier({pid}).terminate"
    );
    let asked = Instant::now();
    let st = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", &script])
        .status()
        .expect("osascript");
    assert!(st.success(), "osascript: {st:?}");
    // when the program finished its quit (its mark says done), as this
    // side sees it
    let mut done: Option<Instant> = None;
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        if done.is_none()
            && std::fs::read_to_string(&mark).is_ok_and(|m| m.ends_with(" done"))
        {
            done = Some(Instant::now());
        }
        if asked.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            panic!("the process did not end after the quit");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let took = done.map_or(Duration::ZERO, |d| d.elapsed());
    let said = std::fs::read_to_string(&mark).unwrap_or_default();
    assert_eq!(
        said, "quit:quit done",
        "the program's quit did not finish before the exit"
    );
    assert_eq!(status.code(), Some(0), "{status:?}");
    // the reply ended it, not the fallback (3 s after the program):
    // timed from the program's finish, since osascript and the quit's
    // way to the program took 10 s on a loaded machine (OL-544)
    assert!(
        took < Duration::from_millis(2500),
        "ended {took:?} after the program finished its quit"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
