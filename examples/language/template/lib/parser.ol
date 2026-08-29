// Fold the flat token stream into a nested node tree. `#each` / `#if` open a
// section whose body runs until the matching `{{/...}}`; parsing recurses on
// the body and reports back how far it consumed so the caller can resume.

use lib.ast {
    Token, TText, TVar, TOpenEach, TOpenIf, TClose,
    Node, NText, NVar, NEach, NIf
}

// Parse nodes starting at `start`, stopping at the next close tag or the end.
// Returns { nodes, pos } where `pos` indexes the close tag (or the length).
fn parse_until(tokens, start) = {
    let mut nodes = []
    let mut i = start
    let n = len(tokens)
    let mut stop = false
    while (i < n) && (!stop) {
        let tok = tokens[i]
        match tok {
            TClose(name) => { stop = true },
            TText(s) => { nodes = nodes + [NText(s)]; i = i + 1 },
            TVar(p) => { nodes = nodes + [NVar(p)]; i = i + 1 },
            TOpenEach(p) => {
                let sub = parse_until(tokens, i + 1)
                nodes = nodes + [NEach(p, sub.nodes)]
                i = sub.pos + 1
            },
            TOpenIf(p) => {
                let sub = parse_until(tokens, i + 1)
                nodes = nodes + [NIf(p, sub.nodes)]
                i = sub.pos + 1
            }
        }
    }
    { nodes: nodes, pos: i }
}

share fn parse(tokens) = parse_until(tokens, 0).nodes
