//! The fused CSV reader against the general parser — cell-identical.
//!
//! `ods.read_csv` has two engines: the fused fast path (one scan per
//! chunk parsing fields into speculative typed builders, spans into
//! the file body for text) and the general csv-crate path (owned cells,
//! full quoting). A file with any quoted cell takes the general path,
//! so quoting one harmless cell forces it — giving two parses of the
//! same logical table whose `to_csv` renderings must match byte for
//! byte. Every fixture aims at a fused-reader edge: speculation
//! upgrades and demotions, validity, CRLF, blank lines, missing final
//! newline, i64 boundaries, and cross-chunk type conflicts on a
//! multi-megabyte table.

use olang::interpreter::Interpreter;
use olang::parser::Parser;

/// Render both parses with to_csv and return them.
fn both_readings(dir: &std::path::Path, name: &str, unquoted: &str) -> (String, String) {
    // Quote the first cell of the header — csv unquotes it back, so the
    // logical table is identical while the fast path is disqualified.
    let quoted = format!("\"{}", unquoted.replacen(',', "\",", 1));
    let fast_path = dir.join(format!("{name}_fast.csv"));
    let general_path = dir.join(format!("{name}_general.csv"));
    std::fs::write(&fast_path, unquoted).expect("write");
    std::fs::write(&general_path, &quoted).expect("write");

    let src = format!(
        "let a = unwrap(ods.read_csv_file({:?}))\n\
         let b = unwrap(ods.read_csv_file({:?}))\n\
         ods.to_csv(a) + \"\\u{{0}}\" + ods.to_csv(b)",
        fast_path.to_str().unwrap(),
        general_path.to_str().unwrap(),
    );
    let program = Parser::new().parse(&src).expect("parse");
    let out = Interpreter::new().eval_program(program).expect("eval");
    let s = match out {
        olang::ast::Value::String(s) => s.to_string(),
        other => panic!("expected String, got {other:?}"),
    };
    let (a, b) = s.split_once('\u{0}').expect("separator");
    (a.to_string(), b.to_string())
}

fn assert_identical(name: &str, unquoted: &str) {
    let dir = std::env::temp_dir().join("olang_csv_diff_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let (fast, general) = both_readings(&dir, name, unquoted);
    assert_eq!(fast, general, "fused reader diverged on fixture {name}");
}

#[test]
fn plain_typed_columns() {
    assert_identical(
        "plain",
        "id,price,name,flag\n1,1.5,ada,true\n2,2.5,grace,false\n3,3.0,alan,true\n",
    );
}

#[test]
fn speculation_upgrades_int_to_float_mid_column() {
    // The first cells look like ints; a later decimal upgrades the
    // column in place. Both readings must be Float end to end.
    assert_identical(
        "upgrade",
        "n,k\n1,x\n2,y\n3,z\n2.5,w\n9007199254740993,v\n4,u\n",
    );
}

#[test]
fn speculation_demotes_numerics_and_bools_to_text() {
    // Ints then a word; bools then a word — both demote to text and
    // re-scan their spans.
    assert_identical("demote", "a,b\n1,true\n2,false\n3,maybe\nfour,true\n");
}

#[test]
fn empties_are_nulls_at_every_type() {
    assert_identical(
        "nulls",
        "i,f,s,bl,allnull\n1,,ada,,\n,2.5,,true,\n3,3.5,alan,false,\n",
    );
}

#[test]
fn crlf_blank_lines_and_missing_final_newline() {
    assert_identical("crlf", "x,y\r\n1,a\r\n\r\n2,b\r\n3,c");
    assert_identical("blank", "x,y\n1,a\n\n\n2,b\n");
}

#[test]
fn i64_boundaries_and_signed_forms() {
    // i64::MAX and MIN stay integers; a 19+-digit overflow makes the
    // column float; '+' signs parse; floats in exponent form parse.
    assert_identical(
        "bounds",
        "big,over,signed,exp\n9223372036854775807,99999999999999999999,+5,1e5\n-9223372036854775808,1,-6,2.5e-3\n",
    );
}

#[test]
fn cross_chunk_conflicts_on_a_multimegabyte_table() {
    // Big enough for the parallel path: most chunks see pure ints in
    // column `v`, but a lone word near the end forces the whole column
    // to text — chunks that parsed ints must re-derive spans. Column
    // `d` upgrades to float across chunks the same way.
    let mut body = String::with_capacity(6 << 20);
    body.push_str("v,d,tag\n");
    let rows = 300_000;
    for i in 0..rows {
        if i == rows - 7 {
            body.push_str(&format!("word,{i}.5,t{}\n", i % 10));
        } else if i == rows - 3 {
            body.push_str(&format!("{i},{i}.25,t{}\n", i % 10));
        } else {
            body.push_str(&format!("{i},{i},t{}\n", i % 10));
        }
    }
    assert!(
        body.len() > 4 * 1024 * 1024,
        "fixture must engage the parallel path"
    );
    assert_identical("chunked", &body);
}
