//! `gui`'s text engine (Loom phase 3): variable fonts' optical size,
//! fallback per cluster, the shaping cache's generations, darkening that
//! matches macOS, hairlines and scrolls on whole pixels, and the text
//! system's gestures (a double click's drag by words, a drag-select past
//! a field's edge scrolling it on).

#![cfg(feature = "gui")]

use olang::stdlib::gui::text::{self, Align, Font};
use olang::stdlib::gui::{raster, rich, window};
use olang::{Interpreter, Parser, Value};

/// The engine has one event channel for the process; tests that read it
/// take turns.
static EVENTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn events_turn() -> std::sync::MutexGuard<'static, ()> {
    EVENTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn run(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parses");
    Interpreter::new().eval_program(program).expect("runs")
}

#[test]
fn the_optical_size_follows_the_text_size() {
    let small = Font { size: 13.0, ..Font::default() };
    let large = Font { size: 28.0, ..Font::default() };
    assert!(format!("{:?}", small.styles()).contains(r#"opsz\" 13"#));
    assert!(format!("{:?}", large.styles()).contains(r#"opsz\" 28"#));
    // a span's own size sets its own optical size
    let span = rich::SpanStyle { size: Some(20.0), ..Default::default() };
    assert!(format!("{:?}", span.props(1.0)).contains(r#"opsz\" 20"#));
    // the system's face (SF Pro, variable in weight and optical size) is
    // set at another instance at another size
    if cfg!(target_os = "macos") {
        let mut ts = text::system().lock().unwrap();
        let at = |ts: &mut text::TextSystem, size: f32| {
            let f = Font { family: "system-ui".into(), size, ..Font::default() };
            let sh = ts.shape("Hamburg", &f, [0, 0, 0, 255], None, Align::Start, 1.0);
            sh.runs[0].coords.to_vec()
        };
        let (a, b) = (at(&mut ts, 13.0), at(&mut ts, 28.0));
        if !a.is_empty() {
            assert_ne!(a, b, "the optical size moved the instance");
        }
    }
}

#[test]
fn every_script_falls_back_to_a_face_that_has_it() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let mut ts = text::system().lock().unwrap();
    // Latin, Chinese, Arabic, Thai, Devanagari, Hebrew, a symbol, emoji
    for t in ["a", "中文", "عربي", "ไทย", "हिन्दी", "עברית", "⌘", "😀"] {
        let sh = ts.shape(t, &Font::default(), [0, 0, 0, 255], None, Align::Start, 2.0);
        let glyphs: Vec<u16> = sh.runs.iter().flat_map(|r| r.glyphs.iter().map(|g| g.0)).collect();
        assert!(!glyphs.is_empty(), "{t}: no glyphs");
        assert!(glyphs.iter().all(|g| *g != 0), "{t}: a missing glyph ({glyphs:?})");
    }
    // one text of many scripts: a run a face, each cluster in a face that has it
    let sh = ts.shape("a中ع😀ก", &Font::default(), [0, 0, 0, 255], None, Align::Start, 2.0);
    let faces: std::collections::HashSet<_> = sh.runs.iter().map(|r| r.face).collect();
    assert!(faces.len() >= 4, "{} faces", faces.len());
    assert!(sh.runs.iter().flat_map(|r| r.glyphs.iter()).all(|g| g.0 != 0));
}

#[test]
fn the_shaping_cache_keeps_what_is_still_used_through_a_turn_over() {
    let mut ts = text::system().lock().unwrap();
    let f = Font::default();
    let black = [0, 0, 0, 255];
    let a = ts.shape("kept across generations", &f, black, None, Align::Start, 1.0);
    // more texts than a generation holds, the kept one used meanwhile
    for i in 0..20_000 {
        ts.shape(&format!("filler {i}"), &f, black, None, Align::Start, 1.0);
        if i % 4000 == 0 {
            ts.shape("kept across generations", &f, black, None, Align::Start, 1.0);
        }
    }
    let (shaped, found) = (ts.shaped, ts.found);
    let again = ts.shape("kept across generations", &f, black, None, Align::Start, 1.0);
    assert_eq!((ts.shaped, ts.found), (shaped, found + 1), "found, not shaped again");
    assert!(std::sync::Arc::ptr_eq(&a, &again));
    assert!(ts.cached() <= 2 * 8192);
    // a width or a colour is another shape
    ts.shape("kept across generations", &f, black, Some(40.0), Align::Start, 1.0);
    assert_eq!(ts.shaped, shaped + 1);
}

#[test]
fn text_darkens_as_macos_draws_it() {
    // the colour's lightness in steps
    assert_eq!(raster::tone_of([0, 0, 0, 255]), 0);
    assert_eq!(raster::tone_of([255, 255, 255, 255]), raster::TONES);
    assert!(raster::tone_of([0x86, 0x86, 0x8b, 255]) > 0);
    let t = raster::Tuning::default();
    // light text on dark is thickened more than dark text on light, small
    // text a little, never past a twentieth of the em
    let (d, l) = (raster::darken(&t, 26.0, 0), raster::darken(&t, 26.0, raster::TONES));
    assert!(l >= d && l > 0.0, "{d} {l}");
    assert!(raster::darken(&t, 8.0, raster::TONES) <= 8.0 / 20.0);
    // the coverage curve keeps empty and full pixels
    for tone in 0..=raster::TONES {
        let c = raster::coverage_curve(&t, tone);
        assert_eq!((c[0], c[255]), (0, 255));
        assert!(c.windows(2).all(|w| w[0] <= w[1]));
    }
    if !cfg!(target_os = "macos") {
        return;
    }
    // a glyph drawn for white text has more ink than for black
    let mut ts = text::system().lock().unwrap();
    let sh = ts.shape("e", &Font { size: 13.0, ..Font::default() }, [0, 0, 0, 255], None, Align::Start, 2.0);
    drop(ts);
    let r = &sh.runs[0];
    let (id, x, _) = r.glyphs[0];
    let ink = |c: [u8; 4]| {
        let k = raster::GlyphKey::new(r.face, id, r.size, x, &r.coords, false, false, c);
        let img = raster::glyph(&k, &r.coords).expect("ink");
        img.data.iter().map(|v| *v as u64).sum::<u64>()
    };
    assert!(ink([255, 255, 255, 255]) > ink([0, 0, 0, 255]));
}

#[test]
fn hairlines_and_scrolls_sit_on_whole_pixels() {
    assert_eq!(window::snap_line(10.4, 1.5), (10.0, 2.0));
    assert_eq!(window::snap_line(10.6, 0.4), (11.0, 1.0));
    assert_eq!(rich::snap_scroll(12.6, 100.0), 13.0);
    assert_eq!(rich::snap_scroll(99.7, 99.5), 99.0);
    assert_eq!(rich::snap_scroll(-3.0, 10.0), 0.0);
}

#[test]
fn a_styled_field_scrolls_by_whole_pixels_and_slow_wheels_add_up() {
    use olang::stdlib::gui::window::{Clipboard, Input, WinState};
    struct NoClip;
    impl Clipboard for NoClip {
        fn get(&mut self) -> Option<String> {
            None
        }
        fn set(&mut self, _: String) {}
    }
    let _turn = events_turn();
    run(r##"
        let w = gui.headless(#{ "size": (300, 200), "scale": 2 })
        let lines = join(map(range(0, 400), (i) => `line ${i}`), "\n")
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 200) },
          #{ "key": "wheeled", "parent": "root", "role": "textarea", "box": (0, 0, 300, 200),
             "edit": #{ "value": lines, "rich": true, "undo": false }, "style": #{ "size": 13.3, "line_height": 1.37 } }
        ])
        gui.read(w, "pixels")
    "##);
    let st = olang::stdlib::gui::windows()
        .lock()
        .unwrap()
        .values()
        .find(|w| w.lock().unwrap().editors.contains_key("wheeled"))
        .cloned()
        .expect("the window");
    let mut st = st.lock().unwrap();
    let st: &mut WinState = &mut st;
    let mut out = vec![];
    st.input(Input::Pointer { action: window::PointerAction::Move, x: 100.0, y: 100.0, button: "left".into(), clicks: 0 }, &mut NoClip, &mut out);
    let mut offs = vec![];
    for _ in 0..8 {
        st.input(Input::Wheel { dx: 0.0, dy: -0.3, mods: None }, &mut NoClip, &mut out);
        let _ = st.display_list();
        offs.push(st.editors["wheeled"].rich().unwrap().scroll_y);
    }
    // every offset a whole device pixel; eight wheels of 0.3 logical at
    // 2× added up to about 4.8 device pixels (none lost to rounding)
    assert!(offs.iter().all(|o| o.fract() == 0.0), "{offs:?}");
    assert!((offs[7] - 4.8).abs() <= 0.5, "{offs:?}");
    assert!(offs[0] <= 1.0, "{offs:?}");
}

#[test]
fn a_double_click_drags_by_words_and_a_drag_past_the_edge_scrolls_on() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (400, 120) })
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 400, 120) },
          #{ "key": "p", "parent": "root", "role": "input", "box": (0, 0, 400, 30), "edit": #{ "value": "alpha beta gamma delta" } }
        ])
        // a double click on "beta", dragged into "gamma": both words whole
        gui.input(w, #{ "kind": "pointer", "action": "down", "x": 60, "y": 15, "clicks": 1 })
        gui.input(w, #{ "kind": "pointer", "action": "up", "x": 60, "y": 15 })
        gui.input(w, #{ "kind": "pointer", "action": "down", "x": 60, "y": 15, "clicks": 2 })
        let word = gui.read(w, "selection", "p")
        gui.input(w, #{ "kind": "pointer", "action": "move", "x": 95, "y": 15 })
        let words = gui.read(w, "selection", "p")
        gui.input(w, #{ "kind": "pointer", "action": "up", "x": 95, "y": 15 })
        gui.close(w)

        let w2 = gui.headless(#{ "size": (300, 200) })
        let lines = join(map(range(0, 300), (i) => `line ${i}`), "\n")
        gui.apply(w2, [
          #{ "key": "root", "box": (0, 0, 300, 200) },
          #{ "key": "f", "parent": "root", "role": "textarea", "box": (0, 0, 300, 150),
             "edit": #{ "value": lines, "rich": true, "undo": false } }
        ])
        gui.read(w2, "pixels")
        gui.input(w2, #{ "kind": "pointer", "action": "down", "x": 20, "y": 10, "clicks": 1 })
        // the pointer held below the field: each frame scrolls and selects on
        gui.input(w2, #{ "kind": "pointer", "action": "move", "x": 20, "y": 190 })
        let sel0 = gui.read(w2, "selection", "f")
        let mut t = 0
        for i in 0..30 {
          t = t + 16
          gui.input(w2, #{ "kind": "clock", "ms": t })
          gui.read(w2, "pixels")
        }
        let sel1 = gui.read(w2, "selection", "f")
        let due = gui.read(w2, "next_frame")
        gui.input(w2, #{ "kind": "pointer", "action": "up", "x": 20, "y": 190 })
        gui.read(w2, "pixels")
        let after = gui.read(w2, "next_frame")
        [word, words, sel0[1] < sel1[1], due != (), after]
    "##);
    assert_eq!(format!("{v}"), "[(6, 10), (6, 16), true, true, ()]");
}

#[test]
fn a_fields_lines_place_each_character_for_assistive_output() {
    let _turn = events_turn();
    run(r##"
        let w = gui.headless(#{ "size": (300, 200), "scale": 2 })
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 200) },
          #{ "key": "lined", "parent": "root", "role": "textarea", "box": (10, 20, 120, 160),
             "edit": #{ "value": "one two three four five six\nseven", "multiline": true } }
        ])
        gui.read(w, "pixels")
    "##);
    let st = olang::stdlib::gui::windows()
        .lock()
        .unwrap()
        .values()
        .find(|w| w.lock().unwrap().editors.contains_key("lined"))
        .cloned()
        .expect("the window");
    let mut st = st.lock().unwrap();
    let lines = st.field_lines("lined").expect("lines");
    // wrapped at 120: the first paragraph on two lines or more, then "seven"
    assert!(lines.len() >= 3, "{lines:?}");
    let last = lines.last().unwrap();
    assert_eq!((last.0.para, last.0.start, last.0.xs.len()), (1, 0, 5));
    // each line below the last, inside the field's box (device pixels at 2×)
    for w in lines.windows(2) {
        assert!(w[1].1[1] >= w[0].1[1] + w[0].1[3] - 2.0, "{lines:?}");
    }
    assert!(lines[0].1[0] >= 20.0 && lines[0].1[1] >= 38.0, "{:?}", lines[0].1);
    // characters left to right along a line, as wide as their advances
    let first = &lines[0].0;
    assert!(first.xs.windows(2).all(|p| p[1] > p[0]));
    assert!(first.widths.iter().all(|w| *w >= 0.0));
    assert_eq!(first.start, 0);
    let second = &lines[1].0;
    assert_eq!(second.para, 0);
    assert!(second.start > 0);
}

/// A stand-in checker: "teh" is the only misspelling.
fn teh(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    for i in 0..chars.len().saturating_sub(2) {
        let start = i == 0 || !chars[i - 1].is_alphanumeric();
        let end = chars.get(i + 3).is_none_or(|c| !c.is_alphanumeric());
        if start && end && chars[i..i + 3] == ['t', 'e', 'h'] {
            out.push((i, i + 3));
        }
    }
    out
}

#[test]
fn a_spell_checked_field_underlines_its_misspelled_words_but_not_the_one_being_typed() {
    use olang::stdlib::gui::edit;
    let _turn = events_turn();
    edit::set_spell_checker(Some(teh));
    run(r##"
        let w = gui.headless(#{ "size": (300, 100) })
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 100), "style": #{ "bg": "#ffffff" } },
          #{ "key": "spelt", "parent": "root", "role": "textarea", "box": (0, 0, 300, 40),
             "edit": #{ "value": "teh cat and teh", "spellcheck": true } },
          #{ "key": "unspelt", "parent": "root", "role": "textarea", "box": (0, 50, 300, 40),
             "edit": #{ "value": "teh dog" } },
          #{ "op": "focus", "key": "spelt" }
        ])
        gui.read(w, "pixels")
    "##);
    let st = olang::stdlib::gui::windows()
        .lock()
        .unwrap()
        .values()
        .find(|w| w.lock().unwrap().editors.contains_key("spelt"))
        .cloned()
        .expect("the window");
    let mut st = st.lock().unwrap();
    let red = |st: &mut window::WinState| {
        st.display_list()
            .prims
            .iter()
            .filter(|p| matches!(p, window::Prim::Rect { fill, .. } if *fill == [255, 59, 48, 255]))
            .count()
    };
    // both words found; the caret (at the end, after focusing) is on the
    // second, still being typed: only the first is underlined
    let _ = red(&mut st);
    assert_eq!(st.misspelled("spelt"), vec![(0, 3), (12, 15)]);
    assert!(st.misspelled("unspelt").is_empty(), "a field not asked is not checked");
    let first_only = red(&mut st);
    let ed = st.editors.get_mut("spelt").unwrap();
    let mut ts = text::system().lock().unwrap();
    if let Some(e) = ed.plain_mut() {
        e.select_chars(5, 5, &mut ts);
    }
    drop(ts);
    let both = red(&mut st);
    assert!(first_only > 0 && both > first_only, "{first_only} {both}");
    edit::set_spell_checker(None);
}

#[test]
fn spelling_rechecks_only_the_paragraph_typed_in_once_typing_pauses() {
    use olang::stdlib::gui::edit;
    use olang::stdlib::gui::window::{SPELL_PAUSE_MS, Spelling};
    let _turn = events_turn();
    edit::set_spell_checker(Some(teh));
    let mut sp = Spelling::default();
    // the first time, every paragraph at once
    assert_eq!(sp.update("teh one\nand teh two", 0.0), None);
    assert_eq!(sp.found, vec![(0, 3), (12, 15)]);
    // typing in the first: the second's kept (moved along), the first's
    // waits for a pause
    let due = sp.update("teh ones\nand teh two", 10.0);
    assert_eq!(due, Some(10.0 + SPELL_PAUSE_MS));
    assert_eq!(sp.found, vec![(13, 16)]);
    assert_eq!(sp.update("teh ones\nand teh two", 10.0 + SPELL_PAUSE_MS), None);
    assert_eq!(sp.found, vec![(0, 3), (13, 16)]);
    edit::set_spell_checker(None);
}
