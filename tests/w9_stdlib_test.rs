//! Standard-library rows from roadmap W9, pinned as differentials
//! against `--no-ovm`: `map_path`, the `col.` mirrors of the global
//! collection helpers (`drop`, `col.index_of`, `col.slice`), `max`/`min`
//! over scalars, `dates.stamp`, and `viz`'s spec-key checking.

use std::process::Command;

fn run(source: &str, no_ovm: bool) -> (String, i32) {
    let dir = std::env::temp_dir().join("olang_w9_stdlib_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let path = dir.join(format!("t{:x}.ol", h.finish()));
    std::fs::write(&path, source).expect("write");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    if no_ovm {
        cmd.arg("--no-ovm");
    }
    let out = cmd.arg("run").arg(&path).output().expect("run");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

fn agree(source: &str) -> (String, i32) {
    let (tiered, rc_t) = run(source, false);
    let (oracle, rc_o) = run(source, true);
    assert_eq!(tiered, oracle, "tiers diverged on:\n{source}");
    assert_eq!(rc_t, rc_o);
    (tiered, rc_t)
}

#[test]
fn map_path_reads_nested_values_and_stops_at_the_first_missing_hop() {
    let (out, rc) = agree(
        "let e = #{ \"issue\": #{ \"fields\": #{ \"estimate\": 5 }, \"tags\": [\"a\"] } }\n\
         println(show(map_path(e, [\"issue\", \"fields\", \"estimate\"])))\n\
         println(show(map_path(e, [\"issue\", \"missing\", \"deep\"])))\n\
         println(show(map_path(e, [])))\n\
         let o = { a: { b: 7 } }\n\
         println(show(map_path(o, [\"a\", \"b\"])))\n\
         let j = unwrap(json.parse(\"{\\\"x\\\": {\\\"y\\\": [1, 2]}}\"))\n\
         println(show(map_path(j, [\"x\", \"y\"])))",
    );
    assert_eq!(rc, 0, "{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "()");
    assert!(lines[2].starts_with("#{"), "{out}");
    assert_eq!(lines[3], "7");
    assert_eq!(lines[4], "[1, 2]");
    // Reading through a non-map is an error naming the key, not Unit.
    let (out, rc) = agree("println(show(map_path(#{ \"a\": 3 }, [\"a\", \"b\"])))");
    assert_ne!(rc, 0);
    assert!(out.contains("cannot read key 'b'"), "{out}");
}

#[test]
fn collection_helpers_have_one_spelling_under_col_and_drop_exists() {
    let (out, rc) = agree(
        "let xs = [10, 20, 30, 40, 50]\n\
         println(show(drop(xs, 2)) + \" \" + show(col.drop(xs, 2)) + \" \" + show(skip(xs, 2)))\n\
         println(show(col.take(xs, 2)) + \" \" + show(col.contains(xs, 30)) + \" \" + show(col.sum(xs)))\n\
         println(show(col.index_of(xs, 30)) + \" \" + show(col.index_of(xs, 99)))\n\
         println(show(col.slice(xs, 1, 3)) + \" \" + show(col.slice(xs, -2, 99)) + \" \" + show(col.slice(xs, 4, 2)))\n\
         println(show(col.map(xs, (x) => x / 10)) + \" \" + show(col.fold(xs, 0, (a, b) => a + b)))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        out,
        "[30, 40, 50] [30, 40, 50] [30, 40, 50]\n\
         [10, 20] true 150\n\
         2 ()\n\
         [20, 30] [40, 50] []\n\
         [1, 2, 3, 4, 5] 150\n"
    );
}

#[test]
fn max_and_min_take_scalars_or_a_list() {
    let (out, rc) = agree(
        "println(show(max(0, 5)) + \" \" + show(min(3, 1, 2)) + \" \" + show(max([4, 9, 2])))\n\
         println(show(max(2.5, 1)))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "5 1 9\n2.5\n");
    let (out, rc) = agree("println(max(3))");
    assert_ne!(rc, 0);
    assert!(
        out.contains("pass a list (max([a, b])) or two or more values"),
        "{out}"
    );
}

#[test]
fn dates_stamp_is_utc_at_second_precision() {
    let (out, rc) = agree(
        "let s = dates.stamp()\n\
         println(show(str.length(s)))\n\
         println(show(str.ends_with(s, \"Z\")))\n\
         println(show(is_ok(dates.parse(s))))\n\
         println(show(str.contains(s, \".\")))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "20\ntrue\ntrue\nfalse\n");
}

#[test]
fn viz_accepts_short_size_keys_and_refuses_unknown_ones() {
    let (out, rc) = agree(
        "use viz\n\
         let data = [#{ \"k\": \"a\", \"v\": 1 }, #{ \"k\": \"b\", \"v\": 3 }]\n\
         let svg = viz.chart(#{ \"data\": data, \"mark\": \"bar\", \"x\": \"k\", \"y\": \"v\", \"w\": 300, \"h\": 200, \"font_size\": 10 })\n\
         println(show(str.contains(svg, \"width=\\\"300\\\"\")))\n\
         println(show(str.contains(svg, \"font-size=\\\"10.0\\\"\")))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "true\ntrue\n");
    let (out, rc) = agree(
        "use viz\n\
         viz.chart(#{ \"data\": [#{ \"k\": \"a\", \"v\": 1 }], \"mark\": \"bar\", \"x\": \"k\", \"y\": \"v\", \"size\": 300 })",
    );
    assert_ne!(rc, 0);
    assert!(out.contains("unknown key 'size'"), "{out}");
}

#[test]
fn a_bare_share_is_a_parse_error_that_names_the_fix() {
    let (out, rc) = agree("share meta fn f(x) = x\nprintln(1)");
    assert_ne!(rc, 0);
    assert!(
        out.contains("`share` must be followed by a declaration"),
        "{out}"
    );
    assert!(out.contains("A meta fn cannot be shared"), "{out}");
}
