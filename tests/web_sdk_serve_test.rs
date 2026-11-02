//! web-sdk's `serve` on a port it cannot take: it never says it listens
//! (it said "<title> listening on …" before the bind, which then failed),
//! says why on stderr, and the program ends with 1 (it ended 0, a clean
//! stop to a supervisor). Its default bind is this machine only.

use std::path::Path;
use std::process::Command;

#[test]
fn serve_on_a_port_held_elsewhere_says_so_and_exits_1() {
    let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = held.local_addr().unwrap().port();
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("frameworks/web-sdk");
    let dir = std::env::temp_dir().join(format!("olang_websrv_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("app.ol");
    std::fs::write(
        &script,
        "use lib.server { serve }\n\
         serve(#{ \"title\": \"held\", \"port\": to_int(os.args()[1]), \"routes\": [], \"client\": \"demo/client.ol\" })\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .current_dir(&web)
        .arg(&script)
        .arg(port.to_string())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stdout}{stderr}");
    assert!(!stdout.contains("listening"), "{stdout}");
    assert!(
        stderr.contains("could not start on 127.0.0.1:") && stderr.contains("in use"),
        "{stderr}"
    );
    drop(held);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `dispatch` run in process prints its access line at OLANG_ACCESS_LOG's
/// level, as `serve` does: a stdio child speaking a protocol on stdout
/// sets `off` (OL-254; the line was printed whatever the level).
#[test]
fn dispatch_in_process_honours_the_access_log_level() {
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("frameworks/web-sdk");
    let dir = std::env::temp_dir().join(format!("olang_webdispatch_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("app.ol");
    std::fs::write(
        &script,
        "use lib.routes { rpc }\n\
         use lib.server { dispatch }\n\
         let r = dispatch([rpc(\"x\", (req, p) => 1)], { method: \"POST\", path: \"/api/rpc/x\", query: #{}, headers: #{}, body: \"{}\" })\n\
         println(\"status \" + show(r.status))\n",
    )
    .unwrap();
    let run = |level: Option<&str>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
        cmd.current_dir(&web)
            .arg(&script)
            .env_remove("OLANG_ACCESS_LOG");
        if let Some(l) = level {
            cmd.env("OLANG_ACCESS_LOG", l);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(run(Some("off")), "status 200\n");
    assert_eq!(run(Some("errors")), "status 200\n");
    assert!(run(None).contains("POST /api/rpc/x -> 200"));
    let _ = std::fs::remove_dir_all(&dir);
}
