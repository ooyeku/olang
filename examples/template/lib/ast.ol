// The two ADTs the engine passes between its stages. Keeping them in one
// module lets the lexer, parser, and renderer share the same constructors —
// olang carries an enum's variant constructors across `use`, so `TVar("x")`
// and `NEach(path, body)` are constructible wherever the type is imported.

// A flat token, produced by the lexer from the raw template text.
share type Token = enum {
    TText(String),        // literal text between tags
    TVar(String),         // {{ path }}
    TOpenEach(String),    // {{#each path}}
    TOpenIf(String),      // {{#if path}}
    TClose(String)        // {{/each}} or {{/if}}
}

// A node in the parsed template tree. Sections nest a body of child nodes.
share type Node = enum {
    NText(String),
    NVar(String),
    NEach(String, List),  // path to a list, body rendered once per item
    NIf(String, List)     // path to a flag, body rendered when truthy
}
