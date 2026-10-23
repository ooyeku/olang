//! The flat layout's engine calls (`gui.flatten`, `gui.flat_emit`,
//! `gui.flat_join`, src/stdlib/gui/flat.rs) on both tiers: the VM answers
//! them in its own form (the placements read natively, never converted)
//! and the tree-walker in the interpreter's, and the two must say the
//! same — frame after frame, by index and by key.

#![cfg(feature = "gui")]

use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

/// Three frames of a column of keyed cards: laid out afresh; the first
/// card gone (another shape: what is kept found by engine key); the same
/// view again (last frame's shape: kept by index). Each box is its key's,
/// wherever the node is, so what has its key keeps its box. Last frame's
/// placements are marked before they are handed back, so what is kept is
/// counted.
const PROGRAM: &str = r##"
let KEYS = ["root"] + flatten(map(range(0, 8), (i) => ["card:" + to_string(i), "h:" + to_string(i), "p:" + to_string(i)]))
fn leaf(k) = #{ "role": "text", "key": k, "props": #{ "text": "t " + k }, "children": [] }
fn card(i) = #{ "role": "group", "key": "card:" + to_string(i), "props": #{ "gap": 2 }, "children": [leaf("h:" + to_string(i)), leaf("p:" + to_string(i))] }
fn view(ids) = #{ "role": "group", "key": "root", "props": #{ "gap": 4, "msg": "go" }, "children": map(ids, card) }
let SPEC = #{ "fields": (14, 22), "theme": #{ "accent": "#0a84ff", "hairline": "#dddddd" }, "lifted": [], "fills": [], "ctx": [],
              "shrinks": [], "placed": ["text", "heading", "button", "separator"], "leaves": #{ "text": #{ "size": (40, 16) } },
              "heading": [20, 17, 15, 13], "label": #{}, "looks": #{} }
fn at(k) = { let mut i = 0
    while KEYS[i] != k { i = i + 1 }
    i }
fn boxes(f) = {
    let mut g = []
    for i in 0..f.n {
        let p = to_float(at(f.gkeys[i]))
        g = g + [p, p * 20.0, 40.0, 16.0]
    }
    g
}
fn marked(e) = #{ "recs": map(e.recs, (rs) => map(rs, (r) => map_set(r, "mark", true))), "gkeys": e.gkeys, "probs": e.probs, "lifts": e.lifts }
fn kept(e) = fold(e.recs, 0, (a, rs) => a + len(filter(rs, (r) => r.mark == true)))
fn say(name, f, e) = {
    println(name + " n " + to_string(f.n) + " changed " + show(f.changed) + " todo " + show(e.todo) + " kept " + to_string(kept(e)))
    println(name + " gkeys " + show(e.gkeys))
    println(name + " recs " + show(map(e.recs, (rs) => map(rs, (r) => (r.gkey, r.parent, r.index, r.rel, r.abs, r.path, r.gop)))))
    let j = gui.flat_join(e.pre, e.recs, e.probs, e.lifts)
    println(name + " joined " + show(map(j.nodes, (p) => p.gkey)) + " " + show(j.problems))
}
fn frames() = {
    let f1 = gui.flatten(view(range(0, 8)), SPEC, ())
    gui.flat_keep(f1.id, boxes(f1))
    let e1 = gui.flat_emit(f1, (), SPEC, (), ())
    say("one", f1, e1)
    let f2 = gui.flatten(view(range(1, 8)), SPEC, f1)
    // what was kept by key (each card's entries) is what is read afresh
    let fresh = gui.flatten(view(range(1, 8)), SPEC, ())
    let (I2, F2, K2) = gui.flat_arrays(f2.id)
    let (I0, F0, K0) = gui.flat_arrays(fresh.id)
    println("two arrays alike " + show(take(I2, f2.n * 14) == take(I0, fresh.n * 14) && take(F2, f2.n * 22) == take(F0, fresh.n * 22) && take(K2, f2.n) == take(K0, fresh.n)))
    println("two lists alike " + show(f2.gkeys == fresh.gkeys && f2.paths == fresh.paths && f2.index == fresh.index && f2.glifts == fresh.glifts))
    // a card whose own props changed is read again, not kept
    let v2b = #{ ...view(range(1, 8)), "children": map(range(1, 8), (i) => if i == 3 => #{ ...card(i), "props": #{ "gap": 9, "pad": 3 } } else => card(i)) }
    let f2b = gui.flatten(v2b, SPEC, f1)
    let freshb = gui.flatten(v2b, SPEC, ())
    let (I3, F3, _k3) = gui.flat_arrays(f2b.id)
    let (I4, F4, _k4) = gui.flat_arrays(freshb.id)
    println("two-b arrays alike " + show(take(I3, f2b.n * 14) == take(I4, freshb.n * 14) && take(F3, f2b.n * 22) == take(F4, freshb.n * 22)))
    gui.flat_keep(f2.id, boxes(f2))
    let e2 = gui.flat_emit(f2, (), SPEC, "keyed", #{ "flat": f1, "out": marked(e1) })
    say("two", f2, e2)
    let f3 = gui.flatten(view(range(1, 8)), SPEC, f2)
    gui.flat_keep(f3.id, boxes(f3))
    let keep = map(range(0, max(f3.n, 80)), (i) => if i < f3.n => 1 else => 0)
    let e3 = gui.flat_emit(f3, (), SPEC, keep, marked(e2))
    say("three", f3, e3)
    // keyed with nothing to go by: everything placed afresh
    let e4 = gui.flat_emit(f2, (), SPEC, "keyed", ())
    say("four", f2, e4)
    // the frames as the engine's patches (gui.flat_diff), each against
    // the last; what each frame's placements say (gui.flat_index)
    let pages = map([e1, e2, e3], (e) => gui.flat_join(e.pre, e.recs, e.probs, e.lifts).nodes)
    let mut ops = #{}
    for (i, nodes) in enumerate(pages) {
        let d = gui.flat_diff(ops, nodes)
        println("diff " + to_string(i) + " out " + show(map(d.out, (o) => (o.key, o.parent, o.index))) + " removes " + show(d.removes)
            + " moved " + show(d.moved) + " idx " + show(d.idx) + " next " + show(map_keys(d.next)))
        ops = d.next
        let ix = gui.flat_index(nodes, "w")
        println("index " + to_string(i) + " " + show(map_keys(ix.index)) + " " + show(map_keys(ix.present)) + " " + to_string(ix.nkeys) + " " + show(len(ix.rare)))
    }
    // a child moved among its siblings, one changed and one gone
    let a = [#{ "gkey": "r", "gop": #{ "key": "r" } }, #{ "gkey": "x", "gop": #{ "key": "x", "parent": "r", "index": 0 } },
             #{ "gkey": "y", "gop": #{ "key": "y", "parent": "r", "index": 1 } }, #{ "gkey": "z", "gop": #{ "key": "z", "parent": "r", "index": 2 } }]
    let b = [#{ "gkey": "r", "gop": #{ "key": "r" } }, #{ "gkey": "y", "gop": #{ "key": "y", "parent": "r", "index": 0 } },
             #{ "gkey": "x", "gop": #{ "key": "x", "parent": "r", "index": 1, "name": "new" } }]
    let d0 = gui.flat_diff(#{}, a)
    let d1 = gui.flat_diff(d0.next, b)
    println("moves " + show(d1))
}
frames()
"##;

fn run(flags: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!(
        "gui-flat-{}-{}",
        std::process::id(),
        flags.join("")
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("flat.ol");
    std::fs::write(&file, PROGRAM).unwrap();
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
        "{flags:?} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn the_flat_calls_answer_alike_on_both_tiers() {
    let tree = run(&["--no-ovm"]);
    let vm = run(&["--ovm-tier=1"]);
    assert_eq!(tree, vm, "the tiers disagree");
    let line = |name: &str| {
        tree.lines()
            .find(|l| l.starts_with(&format!("{name} n ")))
            .unwrap_or_else(|| panic!("no {name} line in:\n{tree}"))
            .to_string()
    };
    // laid out afresh: nothing kept, no shape to compare with
    assert!(
        line("one").contains("changed ()") && line("one").ends_with("kept 0"),
        "{}",
        line("one")
    );
    // another shape: what has its key, its box, and its index under a
    // parent of the same key keeps its placements — each card's two
    // texts (the cards themselves moved up an index, the root lost a
    // child): 7 cards, 14 nodes
    assert!(line("two").contains("changed ()"), "{}", line("two"));
    assert!(line("two").ends_with("kept 14"), "{}", line("two"));
    // the same shape: everything kept by index, nothing changed
    assert!(line("three").ends_with("kept 22"), "{}", line("three"));
    // laid out by key from last frame's arrays, a frame of another shape
    // has the arrays and lists a frame read afresh has
    assert!(tree.contains("two arrays alike true"), "{tree}");
    assert!(tree.contains("two lists alike true"), "{tree}");
    assert!(tree.contains("two-b arrays alike true"), "{tree}");
    // keyed with no last frame: placed afresh
    assert!(line("four").ends_with("kept 0"), "{}", line("four"));
    // the third frame, laid out as the second, sends nothing
    assert!(tree.contains("diff 2 out [] removes []"), "{tree}");
    // y kept its parent and moved up an index (only its index: not sent),
    // x changed, z left
    let moves = tree
        .lines()
        .find(|l| l.starts_with("moves "))
        .expect("moves");
    assert!(
        moves.contains(r#""moved": #{"r": 1}"#) && moves.contains(r#""idx": #{"y": true}"#),
        "{moves}"
    );
    assert!(moves.contains(r#""key": "z""#), "{moves}");
}
