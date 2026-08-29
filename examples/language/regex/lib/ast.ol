// The abstract syntax of a regular expression. The parser produces one of
// these trees; the matcher walks it. `Concat`, `Alt`, and the quantifiers hold
// sub-expressions, so the type is recursive — olang carries an enum's variant
// constructors across `use`, so both the parser and matcher build these.

share type Re = enum {
    Empty,             // matches the empty string
    Lit(String),       // a literal character
    Any,               // .
    Class(List, Bool), // [ ... ]: a list of [lo, hi] ranges, plus a negate flag
    Concat(Re, Re),    // one after another
    Alt(Re, Re),       // a | b
    Star(Re),          // a*
    Plus(Re),          // a+
    Opt(Re),           // a?
    AtStart,           // ^
    AtEnd              // $
}
