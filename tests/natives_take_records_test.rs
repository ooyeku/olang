//! OL-594: a native that takes a map takes a record too. A record
//! (`{ k: v }`, an anonymous object, or a parsed JSON object) is what
//! `map_get` reads as a map, so a native's map argument, and a map inside
//! its options, read it the same: Loom's views, themes and effects may be
//! written as records, and `os.exec` takes `{ env: { NAME: "v" } }`.

use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn run(flags: &[&str], name: &str, program: &str) -> String {
    let dir = std::env::temp_dir().join(format!("records-{}-{name}-{}", std::process::id(), flags.join("")));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("p.ol");
    std::fs::write(&file, program).unwrap();
    let out = Command::new(olang())
        .args(flags)
        .arg("run")
        .arg(&file)
        .output()
        .expect("olang runs");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "{name} {flags:?} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// The same frame laid out from maps and from records: the view's nodes,
/// their props and style, the spec's theme, leaves and label.
#[cfg(feature = "gui")]
const FLAT: &str = r##"
fn leaf_m(k) = #{ "role": "text", "key": k, "props": #{ "text": "t " + k, "style": #{ "border": true, "border_width": 2 } }, "children": [] }
fn view_m(n) = #{ "role": "group", "key": "root", "props": #{ "gap": 4 }, "children": map(range(0, n), (i) => leaf_m("k" + to_string(i))) }
fn leaf_r(k) = { role: "text", key: k, props: { text: "t " + k, style: { border: true, border_width: 2 } }, children: [] }
fn view_r(n) = { role: "group", key: "root", props: { gap: 4 }, children: map(range(0, n), (i) => leaf_r("k" + to_string(i))) }
let SPEC_M = #{ "fields": (14, 22), "theme": #{ "accent": "#0a84ff" }, "lifted": [], "fills": [], "ctx": [], "shrinks": [],
                "placed": ["text"], "leaves": #{ "text": #{ "size": (40, 16) } }, "heading": [20, 17, 15, 13], "label": #{}, "looks": #{} }
let SPEC_R = { fields: (14, 22), theme: { accent: "#0a84ff" }, lifted: [], fills: [], ctx: [], shrinks: [],
               placed: ["text"], leaves: { text: { size: (40, 16) } }, heading: [20, 17, 15, 13], label: {}, looks: {} }
fn said(f) = {
    let (i, fl, k) = gui.flat_arrays(f.id)
    show((f.n, f.gkeys, f.paths, take(i, f.n * 14), take(fl, f.n * 22), take(k, f.n)))
}
let a = gui.flatten(view_m(5), SPEC_M, ())
let b = gui.flatten(view_r(5), SPEC_R, ())
let c = gui.flatten(view_r(5), SPEC_M, ())
println("records lay out as maps " + show(said(a) == said(b) && said(a) == said(c)))
println("a record frame keeps the last " + show(gui.flatten(view_r(5), SPEC_R, b).n == 6))
println(said(b))
// a frame of another shape kept by key from last frame's, `prev` given
// as a map and as a record (OL-596)
fn boxes(f) = flatten(map(range(0, f.n), (i) => [0.0, to_float(i) * 20.0, 40.0, 16.0]))
gui.flat_keep(b.id, boxes(b))
let e1 = gui.flat_emit(b, (), SPEC_R, (), ())
let next = gui.flatten(view_r(4), SPEC_R, b)
gui.flat_keep(next.id, boxes(next))
// last frame's placements marked, so what is kept is counted
let e1m = #{ ...e1, "recs": map(e1.recs, (rs) => map(rs, (r) => map_set(r, "mark", true))) }
fn kept(e) = fold(e.recs, 0, (a, rs) => a + len(filter(rs, (r) => r.mark == true)))
// in a named function, called often enough that the bytecode tier runs it
fn keyed(next, prev_flat, out, as_record) = if as_record => gui.flat_emit(next, (), SPEC_R, "keyed", { flat: prev_flat, out: out })
    else => gui.flat_emit(next, (), SPEC_R, "keyed", #{ "flat": prev_flat, "out": out })
let mut by_map = ()
let mut by_rec = ()
for _ in 0..200 {
    by_map = keyed(next, b, e1m, false)
    by_rec = keyed(next, b, e1m, true)
}
println("keyed keep reads a record prev " + show(kept(by_map) > 0 && kept(by_map) == kept(by_rec)))
println("keyed keep keeps " + to_string(kept(by_map)) + " and " + to_string(kept(by_rec)))
"##;

#[cfg(feature = "gui")]
#[test]
fn gui_flatten_reads_records_as_maps_on_both_tiers() {
    let tree = run(&["--no-ovm"], "flat", FLAT);
    let vm = run(&["--ovm-tier=1"], "flat", FLAT);
    assert_eq!(tree, vm, "the tiers disagree");
    assert!(tree.contains("records lay out as maps true"), "{tree}");
    assert!(tree.contains("a record frame keeps the last true"), "{tree}");
    assert!(tree.contains("keyed keep reads a record prev true"), "{tree}");
}

/// Options given as records, and maps inside options given as records.
const OPTIONS: &str = r##"
let env_r = unwrap(os.exec(["sh", "-c", "printf %s \"$OL594\""], { env: { OL594: "rec" } }))
let env_m = unwrap(os.exec(["sh", "-c", "printf %s \"$OL594\""], #{ "env": #{ "OL594": "map" } }))
println("os.exec env " + env_r.stdout + " " + env_m.stdout)
let j = unwrap(json.parse("{\"env\": {\"OL594\": \"json\"}}"))
println("os.exec json env " + unwrap(os.exec(["sh", "-c", "printf %s \"$OL594\""], j)).stdout)
println("str.lines " + show(str.lines("a\nb", { keep_ends: true }) == str.lines("a\nb", #{ "keep_ends": true })))
println("md.parse " + show(md.parse("# t", { gfm: false }) == md.parse("# t", #{ "gfm": false })))
println("meta.parse " + show(meta.parse("let x = 1", { recover: true }) == meta.parse("let x = 1", #{ "recover": true })))
println("meta.eval " + show(meta.eval("1 + 1", { max_steps: 1000 }) == meta.eval("1 + 1", #{ "max_steps": 1000 })))
let s = ods.series([1.0, 2.0])
println("plot " + show(plot.line(s, s, { title: "t" }) == plot.line(s, s, #{ "title": "t" })))
println("db.open " + show(is_ok(db.open(":memory:", { readonly: false }))))
println("bytes " + show(bytes.decode(bytes.from_list([104, 105]), "utf-8", { lossy: true }) == bytes.decode(bytes.from_list([104, 105]), "utf-8", #{ "lossy": true })))
"##;

#[test]
fn options_and_maps_inside_them_read_records_on_both_tiers() {
    let tree = run(&["--no-ovm"], "opts", OPTIONS);
    let vm = run(&["--ovm-tier=1"], "opts", OPTIONS);
    assert_eq!(tree, vm, "the tiers disagree");
    for want in [
        "os.exec env rec map",
        "os.exec json env json",
        "str.lines true",
        "md.parse true",
        "meta.parse true",
        "meta.eval true",
        "plot true",
        "db.open true",
        "bytes true",
    ] {
        assert!(tree.contains(want), "no `{want}` in:\n{tree}");
    }
}
