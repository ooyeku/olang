// Tree-sitter grammar for olang — token-level with just enough
// structure for a distinct visual identity: pipelines, arrows, map
// literals, template interpolation, qualified calls, and doc comments
// are their own nodes, so themes color what makes olang olang instead
// of dressing it like Rust. The authoritative grammar is the pest one
// in the compiler; this one segments tokens, never validates programs.
module.exports = grammar({
  name: "olang",
  extras: ($) => [/\s/, $.doc_comment, $.comment],
  word: ($) => $.identifier,
  rules: {
    source_file: ($) => repeat($._token),
    _token: ($) =>
      choice($.share, $.keyword, $.boolean, $.dotted, $.call,
             $.type_identifier, $.identifier, $.number, $.char,
             $.template, $.raw_string, $.string, $.macro_name,
             $.pipe, $.arrow, $.hash_brace, $.operator,
             $.bracket, $.delimiter),
    doc_comment: () => token(prec(2, seq(choice("///", "//!"), /.*/))),
    comment: () => token(seq("//", /.*/)),
    // `share` is the export word — its own node so it can pop.
    share: () => "share",
    keyword: () =>
      choice("fn","let","mut","type","if","else","match","for","par","while",
             "loop","break","continue","return","spawn","meta",
             "error","use","struct","enum","test","trait","impl","in"),
    boolean: () => choice("true", "false"),
    // fs.read_file(...), r.state.aliases, stats.norm.sample(...) — a
    // dotted path, any depth, optionally ending in a call. Queries
    // anchor on the member just before the open paren to color call
    // targets as functions and everything else as properties.
    dotted: ($) =>
      seq(field("module", $.identifier),
          repeat1(seq(token.immediate("."),
                      alias(token.immediate(/[a-z_][A-Za-z0-9_]*/), $.member))),
          optional(alias(token.immediate("("), $.open_call))),
    // foo( — any call site, so function names read as functions.
    call: ($) =>
      seq(field("function", $.identifier), token.immediate("(")),
    type_identifier: () => /[A-Z][A-Za-z0-9_]*/,
    identifier: () => /[a-z_][A-Za-z0-9_]*/,
    number: () =>
      /0[xX][0-9a-fA-F][0-9a-fA-F_]*|0[bB][01][01_]*|0[oO][0-7][0-7_]*|\d[\d_]*(\.\d[\d_]*)?([eE][+-]?\d[\d_]*)?/,
    char: () => /'[^']'/,
    macro_name: () => /@[a-z_][A-Za-z0-9_]*/,
    string: ($) => seq('"', repeat(choice(/[^"\\]+/, $.escape)), '"'),
    raw_string: () => seq('r"', repeat(choice(/[^"\\]+/, /\\./)), '"'),
    escape: () => token.immediate(/\\./),
    // Templates carry their interpolations as embedded code.
    template: ($) =>
      seq("`",
          repeat(choice($.template_text, "$", $.escape, $.interpolation)),
          "`"),
    template_text: () => token.immediate(prec(1, /[^`\\$]+/)),
    interpolation: ($) =>
      seq(token.immediate("${"), repeat($._interp_token), "}"),
    // Interpolation bodies are expressions without bare braces — enough
    // for `${r.ms}`, `${a + b}`, `${f(x)}`; a brace construct inside
    // degrades locally, never past the closing backtick.
    _interp_token: ($) =>
      choice($.dotted, $.call, $.boolean, $.type_identifier,
             $.identifier, $.number, $.string, $.pipe, $.arrow,
             $.operator, $.interp_delimiter, "(", ")", "[", "]"),
    interp_delimiter: () => choice(",", ":"),
    // The three marks that make olang code look like olang.
    pipe: () => "|>",
    arrow: () => choice("=>", "->"),
    hash_brace: () => "#{",
    operator: () =>
      choice("==","!=","<=",">=","&&","||","..=","..",
             "+","-","*","/","%","<",">","=","!","?","|","&","^"),
    bracket: () => choice("{","}","(",")","[","]"),
    delimiter: () => choice(",",":",";",".","#"),
  },
});
