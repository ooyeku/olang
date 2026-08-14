//! `olang doc` extraction: the doc-comment scanner that turns `//!` and
//! `///` conventions into a structured reference. Pure over source text.

use olang::tools::doc::{Item, extract};

fn item(kind: &str, name: &str, sig: &str, doc: &str, shared: bool) -> Item {
    Item {
        kind: kind.into(),
        name: name.into(),
        signature: sig.into(),
        doc: doc.into(),
        shared,
    }
}

#[test]
fn module_note_and_documented_items() {
    let src = r#"//! greet — a tiny module.
//! Two lines of module note.

/// Greet someone by name.
share fn greet(name) = "Hello, " + name

/// A record type.
type Person = struct { name: String }

// not a doc comment — this helper is invisible to the reference
fn helper(x) = x + 1

/// Loud greeting.
/// Spans two lines.
share fn shout(name) = str.to_upper(greet(name))
"#;
    let doc = extract(src);
    assert_eq!(
        doc.module_doc,
        "greet — a tiny module.\nTwo lines of module note."
    );
    assert_eq!(
        doc.items,
        vec![
            item("fn", "greet", "greet(name)", "Greet someone by name.", true),
            item("type", "Person", "type Person", "A record type.", false),
            item(
                "fn",
                "shout",
                "shout(name)",
                "Loud greeting.\nSpans two lines.",
                true
            ),
        ]
    );
}

#[test]
fn undocumented_declarations_are_omitted() {
    // Only declarations preceded by /// appear — the doc generator
    // reports what is documented, and nothing else.
    let src = r#"
share fn public_but_undocumented() = 1

/// This one is documented.
fn documented() = 2
"#;
    let doc = extract(src);
    assert_eq!(doc.items.len(), 1);
    assert_eq!(doc.items[0].name, "documented");
    assert!(doc.module_doc.is_empty());
}

#[test]
fn a_blank_line_between_doc_and_decl_is_tolerated() {
    let src = "/// doc\n\nshare fn f() = 1\n";
    let doc = extract(src);
    assert_eq!(doc.items.len(), 1);
    assert_eq!(doc.items[0].doc, "doc");
}
