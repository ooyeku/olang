//! The data stack's parallel kernels (Campaign 5, H3) must be invisible:
//! every fan-out — chunked CSV parse, per-line JSONL parse, the parallel
//! argsort and column gather behind sort_by, the group-by key build —
//! joins in a fixed order, so the parallel result is cell-identical to
//! the sequential one, and error messages (with their line numbers) are
//! identical too. These tests run every kernel both ways over inputs
//! large enough to cross the fan-out thresholds and diff the outputs.

use olang::ast::Value;
use olang::{Interpreter, Parser};
use std::sync::{Mutex, MutexGuard};

/// The parallel switch is process-global and the test harness is
/// multi-threaded: without this lock, one test could re-enable the
/// machinery while another runs its "sequential" side, quietly turning
/// the comparison into parallel-vs-parallel.
static SWITCH: Mutex<()> = Mutex::new(());

fn hold_switch() -> MutexGuard<'static, ()> {
    SWITCH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn set_parallel(on: bool) {
    olang::parallel::set_parallel_enabled(on);
    olang_ods::set_parallel_enabled(on);
}

fn eval(source: &str) -> Result<Value, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let parser = Parser::new();
            let program = parser.parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .expect("spawn")
        .join()
        .expect("join")
}

/// Evaluate the same source with the parallel machinery on and off and
/// require identical results (or identical errors).
fn assert_parallel_transparent(source: &str) {
    let _guard = hold_switch();
    set_parallel(false);
    let sequential = eval(source);
    set_parallel(true);
    let parallel = eval(source);
    assert_eq!(
        sequential, parallel,
        "parallel and sequential runs disagree\n  source: {}",
        source
    );
}

/// 150k rows crosses every data-stack threshold (100k) with room over.
const ROWS: usize = 150_000;

fn big_csv() -> String {
    let mut text = String::from("id,name,score,flag\n");
    for i in 0..ROWS {
        text.push_str(&format!(
            "{},cat{},{}.5,{}\n",
            i,
            i % 977,
            i % 100,
            i % 3 == 0
        ));
    }
    text
}

fn big_jsonl() -> String {
    let mut text = String::new();
    for i in 0..ROWS {
        // Alternate key sets so the column union matters.
        if i % 10 == 0 {
            text.push_str(&format!("{{\"id\":{},\"extra\":\"x{}\"}}\n", i, i));
        } else {
            text.push_str(&format!(
                "{{\"id\":{},\"cat\":\"c{}\",\"v\":{}.25}}\n",
                i,
                i % 8,
                i % 1000
            ));
        }
    }
    text
}

fn csv_program(csv: &str, tail: &str) -> String {
    format!(
        "let text = {:?}\nlet df = ods.read_csv(text)\n{}",
        csv, tail
    )
}

#[test]
fn read_csv_is_parallel_transparent() {
    let csv = big_csv();
    assert_parallel_transparent(&csv_program(&csv, "ods.to_csv(df)"));
}

#[test]
fn read_csv_handles_quoted_newlines_and_commas_identically() {
    // Quoted fields with embedded newlines, commas, and doubled quotes,
    // repeated enough to cross the parse threshold — the record-boundary
    // scan must never split inside a quoted field.
    let mut csv = String::from("id,note\n");
    for i in 0..ROWS {
        csv.push_str(&format!(
            "{},\"line one\nline two, with a comma, and a \"\"quote\"\" {}\"\n",
            i, i
        ));
    }
    assert_parallel_transparent(&csv_program(
        &csv,
        "to_string(ods.n_rows(df)) + \"|\" + ods.to_csv(ods.head(df, 3)) + ods.to_csv(ods.tail(df, 3))",
    ));
}

#[test]
fn read_csv_malformed_error_is_identical() {
    // A ragged row far past the threshold: the parallel path falls back
    // to the sequential reader, so the error (with its true line number)
    // is the same either way.
    let mut csv = big_csv();
    csv.push_str("1,too,few\n");
    csv.push_str(&format!("{},cat0,0.5,true\n", ROWS + 1));
    assert_parallel_transparent(&csv_program(&csv, "ods.to_csv(df)"));
}

#[test]
fn read_jsonl_is_parallel_transparent() {
    let jsonl = big_jsonl();
    let program = format!(
        "let df = ods.read_jsonl({:?}) |> unwrap\nods.to_csv(df)",
        jsonl
    );
    assert_parallel_transparent(&program);
}

#[test]
fn read_jsonl_reports_the_lowest_bad_line_in_parallel() {
    // Two bad lines, both deep enough that different workers hit them;
    // the report must name the first, exactly as a sequential scan would.
    let mut jsonl = big_jsonl();
    let lines: Vec<&str> = jsonl.lines().collect();
    let mut rebuilt: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    rebuilt[120_000] = "[1,2,3]".to_string();
    rebuilt[140_000] = "not json".to_string();
    jsonl = rebuilt.join("\n");
    let program = format!(
        "match ods.read_jsonl({:?}) {{ Ok(df) => \"ok\", Err(msg) => msg }}",
        jsonl
    );
    let got = {
        let _guard = hold_switch();
        set_parallel(true);
        eval(&program).expect("eval")
    };
    let Value::String(msg) = got else {
        panic!("expected the error string, got {:?}", got)
    };
    assert!(
        msg.contains("line 120001"),
        "must report the lowest bad line: {}",
        msg
    );
    assert_parallel_transparent(&program);
}

#[test]
fn sort_by_and_group_by_are_parallel_transparent() {
    let csv = big_csv();
    assert_parallel_transparent(&csv_program(
        &csv,
        "let s = ods.sort_by(df, \"name\", false)\n\
         let g = ods.group_by(df, [\"name\", \"flag\"], [[\"total\", \"sum\", \"score\"], [\"n\", \"count\", \"\"]])\n\
         ods.to_csv(ods.head(s, 50)) + ods.to_csv(ods.sort_by(g, \"name\", true))",
    ));
}

#[test]
fn join_is_parallel_transparent() {
    let csv = big_csv();
    assert_parallel_transparent(&csv_program(
        &csv,
        "let dim = ods.frame_from_records(map(range(0, 977), (i) => #{ \"name\": \"cat\" + to_string(i), \"rank\": i * 7 }))\n\
         let j = ods.join_left(df, dim, \"name\")\n\
         to_string(ods.n_rows(j)) + \"|\" + ods.to_csv(ods.head(ods.sort_by(j, \"id\", false), 20))",
    ));
}
