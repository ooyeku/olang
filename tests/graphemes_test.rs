//! Text is made of graphemes (roadmap lane W1).
//!
//! The settled model: codepoints stay the documented indexing unit
//! (`len`, `str.length`, `str.char_at`, `str.substring` — O(1)-ish,
//! lossless), and `str.graphemes` is the visible-character view,
//! segmented per UAX #29 extended clusters. `str.reverse` operates on
//! graphemes because a reversed string is a *visual* request — tearing
//! a skin-tone modifier off its emoji was the observed corruption.

use olang::ast::Value;
use olang::interpreter::Interpreter;
use olang::parser::Parser;

fn eval(source: &str) -> Value {
    let program = Parser::new().parse(source).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

fn eval_str(source: &str) -> String {
    match eval(source) {
        Value::String(s) => s.to_string(),
        other => panic!("expected String, got {other:?}"),
    }
}

fn eval_int(source: &str) -> i64 {
    match eval(source) {
        Value::Integer(n) => n,
        other => panic!("expected Integer, got {other:?}"),
    }
}

// ── str.reverse keeps clusters whole ──────────────────────────────────

#[test]
fn reverse_keeps_skin_tone_modifiers_attached() {
    // The observed corruption: 👋🏽 is WAVING HAND + medium skin tone.
    assert_eq!(
        eval_str("str.reverse(\"\u{1F44B}\u{1F3FD}ab\")"),
        "ba\u{1F44B}\u{1F3FD}"
    );
}

#[test]
fn reverse_keeps_zwj_families_and_flags_whole() {
    // Family: man+ZWJ+woman+ZWJ+girl+ZWJ+boy is ONE visible character.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    assert_eq!(
        eval_str(&format!("str.reverse(\"{family}x\")")),
        format!("x{family}")
    );
    // Regional-indicator pairs (flags) swap as units, never re-pair.
    assert_eq!(
        eval_str("str.reverse(\"\u{1F1FA}\u{1F1F8}\u{1F1EB}\u{1F1F7}\")"),
        "\u{1F1EB}\u{1F1F7}\u{1F1FA}\u{1F1F8}"
    );
}

#[test]
fn reverse_keeps_combining_marks_on_their_base() {
    // e + COMBINING ACUTE stays "é" wherever it lands.
    assert_eq!(eval_str("str.reverse(\"e\u{0301}f\")"), "fe\u{0301}");
    // Plain ASCII behaves exactly as before.
    assert_eq!(eval_str("str.reverse(\"abc\")"), "cba");
}

// ── str.graphemes is the visible-character view ───────────────────────

#[test]
fn graphemes_segments_per_uax29() {
    // Skin-tone emoji, ZWJ family, combining mark, flag, Hangul jamo
    // sequence, CRLF — the canonical UAX #29 shapes.
    assert_eq!(eval_int("len(str.graphemes(\"\u{1F44B}\u{1F3FD}ab\"))"), 3);
    assert_eq!(
        eval_int(
            "len(str.graphemes(\"\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}\"))"
        ),
        1
    );
    assert_eq!(eval_int("len(str.graphemes(\"e\u{0301}f\"))"), 2);
    assert_eq!(eval_int("len(str.graphemes(\"\u{1F1FA}\u{1F1F8}\"))"), 1);
    // Hangul: choseong + jungseong compose into one syllable cluster.
    assert_eq!(eval_int("len(str.graphemes(\"\u{1100}\u{1161}\"))"), 1);
    // CRLF is one cluster; LF alone is one.
    assert_eq!(eval_int("len(str.graphemes(\"a\\r\\nb\"))"), 3);
}

#[test]
fn graphemes_compose_with_list_operations() {
    // The idioms the API is designed around: count, nth, slice+join.
    assert_eq!(
        eval_str("str.graphemes(\"\u{1F44B}\u{1F3FD}ab\")[0]"),
        "\u{1F44B}\u{1F3FD}"
    );
    assert_eq!(
        eval_str("str.join(take(skip(str.graphemes(\"abcde\"), 1), 3), \"\")"),
        "bcd"
    );
    // Round-trip: joining all graphemes rebuilds the string.
    assert_eq!(
        eval_str("let s = \"e\u{0301} \u{1F44B}\u{1F3FD}\"\nstr.join(str.graphemes(s), \"\")"),
        "e\u{0301} \u{1F44B}\u{1F3FD}"
    );
}

// ── codepoints stay the documented indexing unit ──────────────────────

#[test]
fn codepoint_indexing_is_unchanged() {
    // 👋🏽 is two codepoints, and the indexing functions say so — that is
    // the documented unit, not a bug. The grapheme view is the API for
    // the visible-character reading.
    assert_eq!(eval_int("len(\"\u{1F44B}\u{1F3FD}\")"), 2);
    assert_eq!(eval_int("str.length(\"\u{1F44B}\u{1F3FD}\")"), 2);
    assert_eq!(eval_str("str.char_at(\"e\u{0301}f\", 1)"), "\u{0301}");
    assert_eq!(eval_int("len(str.chars(\"\u{1F44B}\u{1F3FD}\"))"), 2);
}
