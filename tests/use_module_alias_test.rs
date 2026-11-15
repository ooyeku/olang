//! `use a.b as x` (OL-591): 1.0 has no alias for a whole module. It is a
//! parse error at the alias, naming the two ways that work, instead of
//! `as` failing as an undefined name when the program runs.

use olang::Parser;

#[test]
fn a_whole_module_alias_is_a_parse_error_at_the_alias() {
    let err = Parser::new()
        .parse("use core.search as s\nprintln(1)\n")
        .expect_err("refused");
    let said = err.to_string();
    assert!(said.contains("line 1, column 17"), "{said}");
    assert!(said.contains("a whole module takes no alias"), "{said}");
    assert!(
        said.contains("use core.search {}") && said.contains("search.name(...)"),
        "{said}"
    );
}

#[test]
fn the_ways_that_work_still_parse() {
    for src in [
        "use core.search {}\n",
        "use core.search { find as f }\n",
        "use core.search { find, other as o }\n",
        "use core.search\n",
    ] {
        assert!(Parser::new().parse(src).is_ok(), "{src}");
    }
}
