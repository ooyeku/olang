//! pmap, pvec and psorted (OL-424): persistent collections that behave
//! like the Map and List they stand for. Each program runs on the
//! interpreter and on the bytecode tier, which must agree.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn eval(source: &str, tier: bool) -> Result<Value, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if tier {
                interpreter.enable_bytecode_tier(1, false);
            }
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .unwrap()
        .join()
        .unwrap()
}

fn shows(source: &str) -> String {
    let interpreted = eval(source, false);
    let tiered = eval(source, true);
    assert_eq!(tiered, interpreted, "the tiers disagree on:\n{source}");
    match &interpreted {
        Ok(Value::String(s)) => s.to_string(),
        Ok(v) => v.to_string(),
        Err(e) => panic!("{source}\nraised: {e}"),
    }
}

#[test]
fn a_pmap_is_a_map_that_keeps_its_versions() {
    assert_eq!(
        shows(
            "fn bump(p, k) = pmap.set(p, k, (pmap.get(p, k) ?? 0) + 1)\n\
             let v1 = pmap.of(#{ \"a\": 1, \"b\": [2, 3] })\n\
             let mut v2 = v1\n\
             for i in 0..3 { v2 = bump(v2, \"a\") }\n\
             let v3 = pmap.remove(pmap.set(v2, 7, \"seven\"), \"b\")\n\
             [v1.a, v2.a, v2[\"b\"], len(v3), pmap.has(v3, 7), pmap.get(v3, \"b\"), pmap.keys(v3), pmap.diff(v1, v3)]"
        ),
        "[1, 4, [2, 3], 2, true, (), [7, \"a\"], #{\"added\": [7], \"removed\": [\"b\"], \"changed\": [\"a\"]}]"
    );
}

#[test]
fn equality_hash_and_json_follow_map_and_list() {
    assert_eq!(
        shows(
            "let p = pmap.of(#{ \"x\": 1, \"y\": [1, 2] })\n\
             let s = psorted.of(#{ \"y\": [1, 2], \"x\": 1 })\n\
             let v = pvec.of([1, \"two\", (3, 4)])\n\
             [p == #{ \"y\": [1, 2], \"x\": 1 }, #{ \"x\": 1, \"y\": [1, 2] } == p, p == s, p != pmap.set(p, \"x\", 2),\n\
              [p, v] == [#{ \"x\": 1, \"y\": [1, 2] }, [1, \"two\", (3, 4)]], v == [1, \"two\"], p == (), same(p, p),\n\
              hash(p) == hash(#{ \"y\": [1, 2], \"x\": 1 }), hash(v) == hash([1, \"two\", (3, 4)]),\n\
              json.stringify(p) == json.stringify(#{ \"x\": 1, \"y\": [1, 2] }), unwrap(json.stringify(pvec.of([1, 2]))),\n\
              unwrap(value.decode(value.encode([p, s, v]))) == [p, s, v], typeof(p), typeof(v), typeof(s)]"
        ),
        "[true, true, true, true, true, false, false, true, true, true, true, \"[1,2]\", true, \"PMap\", \"PVec\", \"PSorted\"]"
    );
}

#[test]
fn show_writes_the_kind_and_the_keys_in_order() {
    assert_eq!(
        shows(
            "to_string([pmap.of(#{ \"b\": \"x\", \"a\": 1 }), pvec.of([1, \"s\"]), psorted.set(psorted.of(#{ \"k\": 2 }), 3, ())])"
        ),
        "[pmap#{\"a\": 1, \"b\": \"x\"}, pvec[1, \"s\"], psorted#{3: (), \"k\": 2}]"
    );
}

#[test]
fn psorted_scans_ranges_in_order() {
    assert_eq!(
        shows(
            "let mut s = psorted.new()\n\
             for i in [5, 1, 9, 3, 7] { s = psorted.set(s, i, i * i) }\n\
             [psorted.keys(s), psorted.range(s, 3, 8), psorted.range(s, (), 2), psorted.range(s, 8, ()), psorted.range(s, 9, 3),\n\
              psorted.first(s), psorted.last(s), psorted.first(psorted.new()), psorted.diff(s, psorted.remove(psorted.set(s, 2, 4), 9))]"
        ),
        "[[1, 3, 5, 7, 9], [(3, 9), (5, 25), (7, 49)], [(1, 1)], [(9, 81)], [], (1, 1), (9, 81), (), #{\"added\": [2], \"removed\": [9], \"changed\": []}]"
    );
}

#[test]
fn a_pvec_is_a_list_that_keeps_its_versions() {
    assert_eq!(
        shows(
            "fn total(v) = {\n    let mut t = 0\n    for x in v { t = t + x }\n    t\n}\n\
             let a = pvec.of([1, 2, 3])\n\
             let b = pvec.push(pvec.set(a, 0, 10), 4)\n\
             let c = pvec.concat(pvec.slice(b, 1, 3), pvec.pop(a))\n\
             [total(a), total(b), c, a[2], pvec.last(b), len(c), pvec.to_list(c), pvec.slice(a, 2, 99), attempt(() => pvec.get(a, 3))]"
        ),
        "[6, 19, pvec[2, 3, 1, 2], 3, 4, 4, [2, 3, 1, 2], pvec[3], Err(\"pvec.get: position 3 out of bounds for a PVec of length 3\")]"
    );
}

#[test]
fn an_update_with_versions_kept_costs_log_n_not_n() {
    // The issue's probe: 200 updates of a 100k-entry map, every version
    // kept. A Map copies itself each time (about 2 ms); a pmap does not.
    let src = "let big = fold(range(0, 100000), #{}, (m, i) => map_set(m, to_string(i), i))\n\
               let p = pmap.of(big)\n\
               let mut kept = []\n\
               let mut cur = p\n\
               let t0 = time.monotonic_ms()\n\
               for i in range(0, 200) { kept = kept + [cur]; cur = pmap.set(cur, to_string(i), -i) }\n\
               let dt = time.monotonic_ms() - t0\n\
               [dt < 100, pmap.get(cur, \"5\"), pmap.get(kept[0], \"5\"), len(kept[199])]";
    assert_eq!(
        eval(src, false).map(|v| v.to_string()),
        Ok("[true, -5, 5, 100000]".to_string())
    );
}
