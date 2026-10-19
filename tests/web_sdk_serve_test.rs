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
