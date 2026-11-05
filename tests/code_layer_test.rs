//! Code layers (OL-445): an application whose Info.plist names a layers
//! folder boots the active layer when its Ed25519 signature verifies with
//! the key in the bundle and it was built for this runtime, counts each
//! try in the state file, and falls back to the bundled program once the
//! tries are used up, or when any check fails, saying why.

#![cfg(target_os = "macos")]

use base64::Engine as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

const REPORT: &str = "let s = runtime.code_layer()\nprintln(\"WHO running=\" + to_string(s.running) + \" tries=\" + to_string(s.tries) + \" fallback=\" + to_string(s.fallback))\n";

fn payload(dir: &Path, name: &str, who: &str) -> Vec<u8> {
    let src = dir.join(format!("{name}.ol"));
    fs::write(&src, REPORT.replace("WHO", who)).unwrap();
    let out_file = dir.join(format!("{name}.olb"));
    let out = Command::new(olang())
        .arg("build")
        .arg(&src)
        .arg("--payload")
        .arg(&out_file)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    fs::read(out_file).unwrap()
}

struct App {
    exe: PathBuf,
    layers: PathBuf,
    key: ed25519_dalek::SigningKey,
}

fn app(base: &Path) -> App {
    let bundle = base.join("T.app/Contents");
    fs::create_dir_all(bundle.join("MacOS")).unwrap();
    fs::create_dir_all(bundle.join("Resources")).unwrap();
    let exe = bundle.join("MacOS/T");
    fs::copy(olang(), &exe).unwrap();
    let layers = base.join("layers");
    fs::create_dir_all(&layers).unwrap();
    fs::write(
        bundle.join("Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>CFBundleExecutable</key><string>T</string>\n<key>CFBundleIdentifier</key><string>dev.olang.layertest</string>\n<key>CFBundlePackageType</key><string>APPL</string>\n<key>OlangLayersDirectory</key><string>{}</string>\n</dict></plist>\n",
            layers.display()
        ),
    )
    .unwrap();
    let bundled = payload(base, "bundled", "bundled");
    fs::write(bundle.join("Resources/app.olb"), bundled).unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    fs::write(
        bundle.join("Resources/layers.pub"),
        b64(key.verifying_key().as_bytes()),
    )
    .unwrap();
    App { exe, layers, key }
}

fn put_layer(app: &App, id: &str, program: &[u8], signed_by: &ed25519_dalek::SigningKey) {
    use ed25519_dalek::Signer as _;
    let dir = app.layers.join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("app.olb"), program).unwrap();
    fs::write(dir.join("app.olb.sig"), b64(&signed_by.sign(program).to_bytes())).unwrap();
}

fn set_state(app: &App, active: &str, tries: u32, max: u32) {
    fs::write(
        app.layers.join("state.json"),
        format!("{{\"active\": \"{active}\", \"tries\": {tries}, \"max_tries\": {max}, \"channel\": \"beta\"}}"),
    )
    .unwrap();
}

fn tries(app: &App) -> u64 {
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(app.layers.join("state.json")).unwrap()).unwrap();
    assert_eq!(v["channel"], "beta", "the library's own keys are kept");
    v["tries"].as_u64().unwrap()
}

fn boot(app: &App) -> String {
    let out = Command::new(&app.exe).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn a_signed_layer_boots_counts_its_tries_and_falls_back() {
    let base = std::env::temp_dir().join(format!("olang_layers_{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let app = app(&base);

    // no state yet: the bundled program, nothing to say
    assert_eq!(boot(&app), "bundled running=() tries=0 fallback=()");

    let layer = payload(&base, "layer", "layer");
    put_layer(&app, "v2", &layer, &app.key);
    set_state(&app, "v2", 0, 2);
    assert_eq!(boot(&app), "layer running=v2 tries=1 fallback=()");
    assert_eq!(tries(&app), 1);
    assert_eq!(boot(&app), "layer running=v2 tries=2 fallback=()");
    // its tries used up without the program marking it good
    let out = boot(&app);
    assert!(
        out.starts_with("bundled running=() tries=2 fallback=layer v2 was tried 2 times"),
        "{out}"
    );
    assert_eq!(tries(&app), 2, "a fallback counts nothing");

    // the program marks it good: tries back to 0, and it boots again
    set_state(&app, "v2", 0, 2);
    assert!(boot(&app).starts_with("layer running=v2"));

    // signed by another key
    let stranger = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
    put_layer(&app, "v3", &layer, &stranger);
    set_state(&app, "v3", 0, 3);
    let out = boot(&app);
    assert!(out.contains("bundled") && out.contains("signature does not verify"), "{out}");
    assert_eq!(tries(&app), 0);

    // a changed byte after signing
    put_layer(&app, "v4", &layer, &app.key);
    let mut bad = layer.clone();
    let at = bad.len() / 2;
    bad[at] ^= 1;
    fs::write(app.layers.join("v4/app.olb"), &bad).unwrap();
    set_state(&app, "v4", 0, 3);
    assert!(boot(&app).contains("signature does not verify"));

    // signed properly, but built by another runtime
    // the same length, byte for byte, so the footer's lengths still hold
    let from = format!("\"olang_version\":\"{}\"", env!("CARGO_PKG_VERSION"));
    let to = format!("\"olang_version\":\"9{}\"", &env!("CARGO_PKG_VERSION")[1..]);
    let at = layer
        .windows(from.len())
        .position(|w| w == from.as_bytes())
        .expect("the record names its version");
    let mut other = layer.clone();
    other[at..at + to.len()].copy_from_slice(to.as_bytes());
    put_layer(&app, "v5", &other, &app.key);
    set_state(&app, "v5", 0, 3);
    let out = boot(&app);
    assert!(out.contains("bundled") && out.contains("built by olang 9"), "{out}");

    // a name that is no folder's
    set_state(&app, "../v2", 0, 3);
    assert!(boot(&app).contains("is not a folder's name"));

    // no key in the bundle: no layer runs
    fs::remove_file(base.join("T.app/Contents/Resources/layers.pub")).unwrap();
    set_state(&app, "v2", 0, 3);
    let out = boot(&app);
    assert!(out.contains("bundled") && out.contains("public key"), "{out}");

    let _ = fs::remove_dir_all(&base);
}
