//! What the toolchain tests share: a temporary home, and a local fake of
//! GitHub — its releases API, its release downloads, and git repositories
//! (over git's plain "dumb" HTTP) — so no test reaches the network.
#![allow(dead_code)]

use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fake GitHub serving the files under `root`: `/api/...` is the API
/// (`/api/repos/ooyeku/olang/releases` the releases list), everything
/// else a file (`/ooyeku/olang/releases/download/v1.0.0/...`, and bare git
/// repositories at `/<owner>/<repo>.git/`).
pub struct FakeGitHub {
    pub root: PathBuf,
    pub port: u16,
    pub requests: Arc<AtomicUsize>,
}

impl FakeGitHub {
    pub fn start(root: &Path) -> FakeGitHub {
        std::fs::create_dir_all(root).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(AtomicUsize::new(0));
        let (root2, count) = (root.to_path_buf(), requests.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (root, count) = (root2.clone(), count.clone());
                std::thread::spawn(move || serve(stream, &root, &count));
            }
        });
        FakeGitHub {
            root: root.to_path_buf(),
            port,
            requests,
        }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn api(&self) -> String {
        format!("{}/api", self.url())
    }

    pub fn hits(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

fn serve(mut stream: std::net::TcpStream, root: &Path, count: &AtomicUsize) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
            break;
        }
    }
    count.fetch_add(1, Ordering::SeqCst);
    let target = first.split_whitespace().nth(1).unwrap_or("/").to_string();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let rel = path.trim_start_matches('/');
    let mut file = root.join(rel);
    if rel.starts_with("api/repos/") && rel.ends_with("/releases") {
        // one page: a second page is empty
        if query.contains("page=") && !query.contains("page=1") {
            return respond(&mut stream, 200, b"[]", "application/json");
        }
        file = root.join(format!("{rel}.json"));
    }
    if rel.contains("..") || !file.is_file() {
        return respond(&mut stream, 404, b"not found", "text/plain");
    }
    let mut body = Vec::new();
    std::fs::File::open(&file)
        .unwrap()
        .read_to_end(&mut body)
        .unwrap();
    respond(&mut stream, 200, &body, "application/octet-stream");
}

fn respond(stream: &mut std::net::TcpStream, status: u16, body: &[u8], kind: &str) {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Length: {}\r\nContent-Type: {kind}\r\nConnection: close\r\n\r\n",
        if status == 200 { "OK" } else { "Not Found" },
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

/// The platform part of a release tarball's name, as otc picks it.
pub fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("linux", "aarch64") => "linux-arm64",
        _ => "linux-x64",
    }
}

/// A fake olang: a script answering `--version` as `olang <version>`.
pub fn fake_olang(path: &Path, version: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\necho \"olang {version}\"\n")).unwrap();
    make_executable(path);
}

pub fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Publish olang releases on the fake: `(version, date, prerelease)`
/// each, with a tarball holding fake `olang` and `otc` and a SHA256SUMS.
pub fn publish_releases(gh: &FakeGitHub, releases: &[(&str, &str, bool)]) {
    let mut list = Vec::new();
    for (v, date, pre) in releases {
        let dir = gh.root.join(format!("ooyeku/olang/releases/download/v{v}"));
        std::fs::create_dir_all(&dir).unwrap();
        let stage = gh.root.join(format!(".stage-{v}"));
        let inner = stage.join(format!("olang-{v}-{}", platform()));
        fake_olang(&inner.join("olang"), v);
        std::fs::write(inner.join("otc"), format!("#!/bin/sh\necho \"otc {v}\"\n")).unwrap();
        make_executable(&inner.join("otc"));
        let tarball = format!("olang-{v}-{}.tar.gz", platform());
        let ok = Command::new("tar")
            .arg("-czf")
            .arg(dir.join(&tarball))
            .arg("-C")
            .arg(&stage)
            .arg(inner.file_name().unwrap())
            .status()
            .unwrap()
            .success();
        assert!(ok, "tar");
        let bytes = std::fs::read(dir.join(&tarball)).unwrap();
        let sum: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        std::fs::write(dir.join("SHA256SUMS"), format!("{sum}  {tarball}\n")).unwrap();
        let _ = std::fs::remove_dir_all(&stage);
        list.push(serde_json::json!({
            "tag_name": format!("v{v}"), "published_at": format!("{date}T12:00:00Z"),
            "prerelease": pre, "draft": false,
        }));
    }
    let api = gh.root.join("api/repos/ooyeku/olang");
    std::fs::create_dir_all(&api).unwrap();
    std::fs::write(
        api.join("releases.json"),
        serde_json::Value::Array(list).to_string(),
    )
    .unwrap();
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    git_env(dir, args, &[])
}

pub fn git_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=t",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
        ])
        .args(args)
        .envs(env.iter().copied())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A library's files at one version: `olang.toml` naming it and an
/// `index.ol` whose `version()` answers `answer`.
pub fn write_library(dir: &Path, name: &str, version: &str, answer: &str, caps: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("olang.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\ndescription = \"shapes and their areas\"\n{caps}"),
    )
    .unwrap();
    std::fs::write(
        dir.join("index.ol"),
        format!("/// {name}: shapes and their areas\nshare fn version() = \"{answer}\"\n"),
    )
    .unwrap();
}

/// Publish a GitHub repository `owner/repo` on the fake: one commit and
/// tag per `(tag, version, answer, capabilities)`, the default branch at
/// the last. `None` for the library files makes a repository that is not
/// an olang project.
pub fn publish_repo(
    gh: &FakeGitHub,
    owner_repo: &str,
    versions: &[(&str, &str, &str, &str)],
    olang: bool,
) -> PathBuf {
    let work = gh.root.join(format!(".work/{owner_repo}"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    let name = owner_repo.rsplit('/').next().unwrap();
    for (tag, version, answer, caps) in versions {
        if olang {
            write_library(&work, name, version, answer, caps);
        } else {
            std::fs::write(work.join("README.md"), format!("not olang {version}\n")).unwrap();
        }
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", version]);
        git(&work, &["tag", tag]);
    }
    let bare = gh.root.join(format!("{owner_repo}.git"));
    let _ = std::fs::remove_dir_all(&bare);
    std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
    let out = Command::new("git")
        .args(["clone", "-q", "--bare"])
        .arg(&work)
        .arg(&bare)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    git(&bare, &["repack", "-a", "-d", "-q"]);
    git(&bare, &["update-server-info"]);
    work
}

/// Re-publish after changing the work tree (a new tag, a moved tag).
pub fn republish(gh: &FakeGitHub, owner_repo: &str) {
    let work = gh.root.join(format!(".work/{owner_repo}"));
    let bare = gh.root.join(format!("{owner_repo}.git"));
    let _ = std::fs::remove_dir_all(&bare);
    let out = Command::new("git")
        .args(["clone", "-q", "--bare"])
        .arg(&work)
        .arg(&bare)
        .output()
        .unwrap();
    assert!(out.status.success());
    git(&bare, &["repack", "-a", "-d", "-q"]);
    git(&bare, &["update-server-info"]);
}

/// A temporary machine: a home (`OLANG_HOME`), a PATH holding its shims
/// and the system's tools only, and GitHub pointed at a fake.
pub struct Machine {
    pub dir: PathBuf,
    pub home: PathBuf,
    pub github: String,
    pub api: String,
}

pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn text(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(self.stdout.trim())
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text()))
    }
}

impl Machine {
    pub fn new(tag: &str, gh: Option<&FakeGitHub>) -> Machine {
        let dir = std::env::temp_dir().join(format!("otc_{tag}_{}", std::process::id()));
        let _ = make_writable_and_remove(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Machine {
            home,
            github: gh
                .map(|g| g.url())
                .unwrap_or_else(|| "http://127.0.0.1:9".to_string()),
            api: gh
                .map(|g| g.api())
                .unwrap_or_else(|| "http://127.0.0.1:9".to_string()),
            dir,
        }
    }

    pub fn path_var(&self) -> String {
        format!(
            "{}:/usr/bin:/bin:/usr/sbin:/sbin",
            self.home.join("bin").display()
        )
    }

    /// Run otc in `cwd` on this machine.
    pub fn otc(&self, cwd: &Path, args: &[&str]) -> Out {
        let out = Command::new(env!("CARGO_BIN_EXE_otc"))
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .env("HOME", &self.dir)
            .env("OLANG_HOME", &self.home)
            .env("OLANG_GITHUB", &self.github)
            .env("OLANG_GITHUB_API", &self.api)
            .env("PATH", self.path_var())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("TMPDIR", std::env::temp_dir())
            .output()
            .expect("spawn otc");
        Out {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// `otc` that must succeed.
    pub fn ok(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.otc(cwd, args);
        assert!(out.ok, "otc {args:?} failed:\n{}", out.text());
        out.text()
    }

    /// `otc` that must fail; its output.
    pub fn fails(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.otc(cwd, args);
        assert!(!out.ok, "otc {args:?} should fail:\n{}", out.text());
        out.text()
    }

    /// What `olang --version` answers on this machine's PATH.
    pub fn olang_version(&self) -> String {
        let out = Command::new("olang")
            .arg("--version")
            .env_clear()
            .env("PATH", self.path_var())
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(e) => format!("(no olang: {e})"),
        }
    }

    pub fn cleanup(&self) {
        let _ = make_writable_and_remove(&self.dir);
    }
}

/// Remove a tree even where the shelf made its copies read-only.
pub fn make_writable_and_remove(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let _ = Command::new("chmod").arg("-R").arg("u+w").arg(dir).status();
    std::fs::remove_dir_all(dir)
}
