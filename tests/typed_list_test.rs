//! Typed-list backing (the OVM's FloatList/IntList): homogeneous
//! scalar lists cross the tier boundary in their native layout. Every
//! case is a differential — the tiered binary and `--no-ovm` (the
//! interpreter oracle) must print the same bytes — aimed at the paths
//! the typed layout reimplements: indexing, iteration, destructuring
//! patterns, in-place set/swap, fused append, concatenation, equality,
//! and the boundary conversion cache under repeated lambda calls.

use std::process::Command;

fn run(source: &str, no_ovm: bool) -> String {
    let dir = std::env::temp_dir().join("olang_typed_list_tests");
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
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn assert_tiers_agree(source: &str) {
    let tiered = run(source, false);
    let oracle = run(source, true);
    assert_eq!(tiered, oracle, "tiers diverged on:\n{source}");
}

#[test]
fn indexing_and_iteration_over_big_scalar_lists() {
    assert_tiers_agree(
        "fn s(xs) = { let mut t = 0.0\nfor i in 0..len(xs) { t = t + xs[i] }\nt }\n\
         let big = map(0..5000, (i) => to_float(i))\n\
         println(s(big))\n\
         let mut u = 0\nfor v in map(0..5000, (i) => i * 3) { u = u + v }\nprintln(u)",
    );
}

#[test]
fn destructuring_patterns_see_typed_lists() {
    assert_tiers_agree(
        "fn sum_list(xs) = {\n\
             fn go(rest, acc) = match rest {\n\
                 [] => acc,\n\
                 [h, ...t] => go(t, acc + h)\n\
             }\n\
             go(xs, 0)\n\
         }\n\
         println(to_string(sum_list([1, 2, 3, 4])))\n\
         println(to_string(sum_list([1.5, 2.5, 3.0])))\n\
         fn first_two(xs) = match xs { [a, b, ...rest] => a + b + len(rest), [] => -1 }\n\
         println(first_two(map(0..200, (i) => i)))",
    );
}

#[test]
fn in_place_set_swap_and_fused_append() {
    assert_tiers_agree(
        "fn work() = {\n\
             let mut acc = []\n\
             for i in 0..2000 { acc = acc + [to_float(i) * 0.5] }\n\
             let mut xs = map(0..100, (i) => to_float(i))\n\
             for i in 0..100 { xs = col.set(xs, i, xs[i] * 2.0) }\n\
             let mut ys = map(0..100, (i) => i)\n\
             ys = col.swap(ys, 0, 99)\n\
             acc[1999] + xs[50] + to_float(ys[0])\n\
         }\n\
         println(work())",
    );
}

#[test]
fn a_mismatched_write_demotes_to_the_boxed_form() {
    // col.set of a String into a FloatList must produce the same list
    // the interpreter builds — the typed layout is invisible.
    assert_tiers_agree(
        "fn f() = {\n\
             let mut xs = map(0..100, (i) => to_float(i))\n\
             xs = col.set(xs, 3, \"three\")\n\
             `${xs[2]} ${xs[3]} ${xs[4]} ${len(xs)}`\n\
         }\n\
         println(f())",
    );
    assert_tiers_agree(
        "fn f() = {\n\
             let mut xs = map(0..100, (i) => i)\n\
             xs = xs + [\"end\"]\n\
             `${xs[99]} ${xs[100]} ${len(xs)}`\n\
         }\n\
         println(f())",
    );
}

#[test]
fn concat_and_equality_across_layouts() {
    assert_tiers_agree(
        "fn f() = {\n\
             let a = map(0..100, (i) => to_float(i))\n\
             let b = map(0..100, (i) => to_float(i + 100))\n\
             let c = a + b\n\
             let mixed = a + map(0..3, (i) => i)\n\
             `${len(c)} ${c[150]} ${len(mixed)} ${a == map(0..100, (i) => to_float(i))} ${[1, 2] == [1.0, 2.0]}`\n\
         }\n\
         println(f())",
    );
}

#[test]
fn captured_big_lists_stay_cheap_across_millions_of_calls() {
    // The boundary cache: a lambda capturing a big list crosses the
    // tier boundary once per element — this must complete promptly and
    // agree with the oracle. (Before the conversion cache this exact
    // shape was O(n²): a 240k-element gather took 102 seconds.)
    assert_tiers_agree(
        "let vals = map(0..50000, (i) => to_float(i) * 1.5)\n\
         fn gather(vals, idx) = map(idx, (i) => vals[i])\n\
         let idx = map(0..50000, (i) => (i * 7) % 50000)\n\
         let g = gather(vals, idx)\n\
         println(`${len(g)} ${g[0]} ${g[49999]}`)",
    );
}

#[test]
fn native_col_set_keeps_value_semantics() {
    // The write loop runs native (col.set on a raw list inside an OSR
    // region); the caller's list must stay untouched — the first write
    // copies, exactly as the VM's Arc semantics do.
    assert_tiers_agree(
        "fn double_all(xs) = {\n\
             let n = len(xs)\n\
             let mut out = xs\n\
             for i in 0..n { out = col.set(out, i, out[i] * 2.0) }\n\
             out\n\
         }\n\
         let big = map(0..50000, (i) => to_float(i))\n\
         let d = double_all(big)\n\
         println(`${d[0]} ${d[1]} ${d[49999]} ${len(d)}`)\n\
         println(`${big[49999]} ${d[49999]}`)\n\
         let mut ints = map(0..50000, (i) => i)\n\
         for i in 0..50000 { ints = col.set(ints, i, ints[i] + 1) }\n\
         println(`${ints[0]} ${ints[49999]}`)",
    );
}

#[test]
fn native_col_set_out_of_bounds_matches_the_oracle() {
    assert_tiers_agree(
        "fn f(xs) = {\n\
             let mut out = xs\n\
             for i in 0..600 { out = col.set(out, i, 1.0) }\n\
             out\n\
         }\n\
         println(f(map(0..500, (i) => to_float(i))))",
    );
}

#[test]
fn every_hot_loop_in_a_function_gets_its_own_region() {
    // The kmeans shape: one function, an outer iteration around several
    // sequential hot loops — a scan with a data-dependent conditional,
    // then an accumulation pass writing three lists at data-dependent
    // indexes (a multi-list live-out marshaled back as a tuple). Each
    // loop must reach native independently and re-enter on every outer
    // iteration, and the whole run must print what the interpreter
    // prints.
    assert_tiers_agree(
        "fn cluster(xs, ys, k, iters) = {\n\
             let n = len(xs)\n\
             let mut cx = map(0..k, (c) => xs[c * (n / k)])\n\
             let mut assign = map(0..n, (i) => 0)\n\
             let mut it = 0\n\
             while it < iters {\n\
                 for i in 0..n {\n\
                     let mut best = 0\n\
                     let mut bd = 100000000.0\n\
                     for c in 0..k {\n\
                         let d = (xs[i] - cx[c]) * (xs[i] - cx[c])\n\
                         if d < bd => { bd = d; best = c }\n\
                     }\n\
                     assign = col.set(assign, i, best)\n\
                 }\n\
                 let mut sx = map(0..k, (c) => 0.0)\n\
                 let mut sy = map(0..k, (c) => 0.0)\n\
                 let mut ct = map(0..k, (c) => 0)\n\
                 for i in 0..n {\n\
                     let c = assign[i]\n\
                     sx = col.set(sx, c, sx[c] + xs[i])\n\
                     sy = col.set(sy, c, sy[c] + ys[i])\n\
                     ct = col.set(ct, c, ct[c] + 1)\n\
                 }\n\
                 for c in 0..k {\n\
                     if ct[c] > 0 => { cx = col.set(cx, c, sx[c] / to_float(ct[c])) }\n\
                 }\n\
                 it = it + 1\n\
             }\n\
             let mut sizes = map(0..k, (c) => 0)\n\
             for i in 0..n { sizes = col.set(sizes, assign[i], sizes[assign[i]] + 1) }\n\
             sizes\n\
         }\n\
         let n = 20000\n\
         let xs = map(0..n, (i) => to_float(i % 100) / 10.0)\n\
         let ys = map(0..n, (i) => to_float((i * 7) % 100) / 10.0)\n\
         println(cluster(xs, ys, 5, 4))",
    );
}

#[test]
fn native_fused_append_keeps_value_semantics() {
    // The accumulate idiom `acc = acc + [v]` runs native (the fused
    // append with the ownership families). The demotion cases and the
    // aliasing case must land exactly where the interpreter does: a
    // shared accumulator's other name is untouched, and a mismatched
    // element rebuilds the boxed layout mid-loop.
    assert_tiers_agree(
        "fn build(xs, n) = {\n\
             let mut acc = []\n\
             for i in 0..n { acc = acc + [xs[i] * 2.0] }\n\
             acc\n\
         }\n\
         let xs = map(0..50000, (i) => to_float(i))\n\
         let a = build(xs, 50000)\n\
         println(`${a[0]} ${a[49999]} ${len(a)}`)\n\
         let mut shared = map(0..100, (i) => to_float(i))\n\
         let keep = shared\n\
         for i in 0..100 { shared = shared + [to_float(i)] }\n\
         println(`${len(keep)} ${len(shared)} ${keep[99]} ${shared[199]}`)\n\
         let mut mixed = []\n\
         for i in 0..50 { mixed = mixed + [i] }\n\
         mixed = mixed + [\"end\"]\n\
         println(`${mixed[49]} ${mixed[50]} ${len(mixed)}`)",
    );
}

#[test]
fn nested_list_reads_run_the_forward_pass_natively() {
    // The feature-matrix shape: cols[j][i] inside a loop that also
    // calls a compiled function and appends its result. The whole
    // logistic forward pass, against the interpreter oracle.
    assert_tiers_agree(
        "fn act(z) = if z > 1.0 => 1.0 else => z\n\
         fn forward(cols, weights, bias, n, nf) = {\n\
             let mut preds = []\n\
             for i in 0..n {\n\
                 let mut z = bias\n\
                 for j in 0..nf { z = z + weights[j] * cols[j][i] }\n\
                 preds = preds + [act(z)]\n\
             }\n\
             preds\n\
         }\n\
         let n = 30000\n\
         let cols = map(0..6, (j) => map(0..n, (i) => to_float((i * (j + 2)) % 89) / 89.0))\n\
         let weights = map(0..6, (j) => 0.2 * to_float(j) - 0.3)\n\
         let p = forward(cols, weights, 0.1, n, 6)\n\
         println(`${p[0]} ${p[1]} ${p[29999]} ${len(p)}`)\n\
         println(`${cols[0][0]} ${cols[5][29999]}`)",
    );
}
