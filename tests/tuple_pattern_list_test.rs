//! A tuple pattern matches a list of its length, the way destructuring
//! reads: `((a, b)) =>` over `[[1, 2], [3, 4]]` binds as `([a, b]) =>`
//! does, in a parameter, a `let`, a `match` arm and a `for`, on every
//! tier. A list of another length does not match it.

use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

const TIERS: [&[&str]; 3] = [&["--no-ovm"], &["--ovm-tier=1"], &[]];

const PROGRAM: &str = r#"fn sum2((a, b)) = a + b
fn first2(xs) = { let (a, b) = xs; a * 10 + b }
fn arm(v) = match v { (x, y) => x - y, [x, y, z] => x + y + z, _ => -1 }
fn hot() = {
    let mut acc = 0
    for i in range(0, 300) { acc = acc + sum2([i, 1]) + first2([1, 2]) + arm([5, 3]) + arm((5, 3)) + arm([1, 2, 3]) + arm([1]) }
    acc
}
println(hot())
println(map([[1, 2], [3, 4]], ((a, b)) => a + b))
println(map([[1.5, 2.5]], ((a, b)) => a + b))
println(map([["k", [1, 2]]], ((k, (x, y))) => k + to_string(x + y)))
let (p, q) = [7, 8]
println(p * q)
for (i, x) in [[1, 2], [5, 6]] { println(i * x) }
println(attempt(() => { let (a, b) = [1, 2, 3]; a }))
println(match [1, 2, 3] { (a, b) => "pair", _ => "other" })
"#;

#[test]
fn a_tuple_pattern_matches_a_list_of_its_length_on_every_tier() {
    let dir = std::env::temp_dir().join(format!("olang_tuple_list_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("p.ol"), PROGRAM).unwrap();
    let want = "51450\n[3, 7]\n[4.0]\n[\"k3\"]\n56\n2\n30\nErr(\"Pattern match failed\")\nother";
    for flags in TIERS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push("p.ol");
        let out = Command::new(olang())
            .args(&args)
            .current_dir(&dir)
            .output()
            .expect("olang");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim_end(),
            want,
            "flags {flags:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let check = Command::new(olang())
        .args(["check", "p.ol"])
        .current_dir(&dir)
        .output()
        .expect("olang");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
