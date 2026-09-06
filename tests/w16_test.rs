//! Roadmap W16 — the fifth reading of open-track: a drain that a long
//! poll on a kept-alive connection cannot hold open, and a package's
//! front door that carries the macros its index imports.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w16_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

fn olang(dir: &Path, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn a_drain_closes_kept_alive_connections_and_cuts_after_drain_ms() {
    let ws = workspace("drain");
    let port = 43000 + (std::process::id() % 2000) as u16;
    write(
        &ws,
        "d.ol",
        &format!(
            "unwrap(os.on_shutdown((why) => http.shutdown()))\n\
             println(\"up\")\n\
             let r = http.serve({port}, (req) => {{ if req.path == \"/wait\" => {{ time.sleep(20000) }} else => () \"ok\" }},\n\
                                #{{ \"drain_ms\": 1500 }})\n\
             println(\"served: \" + show(r))\n"
        ),
    );
    let child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["run", "d.ol"])
        .current_dir(&ws)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(700));
    // A long poll in flight on a connection that asks to stay alive.
    let mut waiter = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    waiter
        .write_all(b"GET /wait HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n")
        .unwrap();
    // A quick request on a second kept-alive connection: once the drain
    // starts, its response must say Connection: close.
    let mut quick = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let started = std::time::Instant::now();
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    std::thread::sleep(std::time::Duration::from_millis(150));
    quick
        .write_all(b"GET /quick HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n")
        .unwrap();
    quick
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    let mut head = Vec::new();
    let _ = quick.read_to_end(&mut head);
    let head = String::from_utf8_lossy(&head).to_lowercase();
    let out = child.wait_with_output().unwrap();
    let elapsed = started.elapsed();
    let text = String::from_utf8_lossy(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{text}{err}");
    // The 20-second poll did not hold the process: cut after drain_ms.
    assert!(
        elapsed < std::time::Duration::from_secs(8),
        "drain took {elapsed:?}: {text}{err}"
    );
    assert!(text.contains("served: Ok(())"), "{text}");
    assert!(err.contains("cut"), "{err}");
    // The response answered during the drain closed the connection.
    if head.contains("http/1.1 200") {
        assert!(head.contains("connection: close"), "{head}");
    }
    drop(waiter);
}

#[test]
fn a_packages_index_carries_the_macros_it_imports() {
    let ws = workspace("frontdoor");
    let pkg = ws
        .parent()
        .unwrap()
        .join(format!("shuttle_pkg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pkg);
    write(
        &pkg,
        "olang.toml",
        "[package]\nname = \"shuttle\"\nversion = \"0.1.0\"\n",
    );
    write(
        &pkg,
        "lib/decl.ol",
        "share fn declare(name, spec) = \"declared \" + name\n\
         meta fn resource(name, spec) = `println(\"resource \" + ${name})`\n",
    );
    // A plain `use`, not `share use`: the index imports the macro and
    // the package's consumers must still reach it by the package name.
    write(
        &pkg,
        "index.ol",
        "use lib.decl { resource, declare }\nshare fn hello() = \"hi\"\n",
    );
    write(
        &ws,
        "olang.toml",
        &format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nshuttle = {{ path = \"{}\" }}\n",
            pkg.display()
        ),
    );
    write(
        &ws,
        "main.ol",
        "use shuttle\n@resource(\"issue\", #{ \"a\": 1 })\nprintln(shuttle.hello())\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "main.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "resource issue\nhi\n");
    let _ = std::fs::remove_dir_all(&pkg);
}
