// Minimal tree-sitter grammar for olang — token-level, built for editor
// highlighting (Zed requires a tree-sitter grammar per language). The
// authoritative grammar is the pest one in the compiler; this one only
// needs to segment tokens correctly, never to validate programs.
module.exports = grammar({
  name: "olang",
  extras: ($) => [/\s/, $.comment],
  rules: {
    source_file: ($) => repeat($._token),
    _token: ($) =>
      choice($.keyword, $.boolean, $.type_identifier, $.identifier,
             $.number, $.string, $.operator, $.punctuation),
    comment: () => token(seq("//", /.*/)),
    keyword: () =>
      choice("fn","let","mut","type","if","else","match","for","par","while",
             "loop","break","continue","return","async","await","spawn","try",
             "catch","error","share","use","struct","enum","test","trait",
             "impl","in"),
    boolean: () => choice("true", "false"),
    type_identifier: () => /[A-Z][A-Za-z0-9_]*/,
    identifier: () => /[a-z_][A-Za-z0-9_]*/,
    number: () => /\d[\d_]*(\.\d[\d_]*)?/,
    string: ($) =>
      seq('"', repeat(choice(/[^"\\$]+/, /\\./, seq("${", /[^}]*/, "}"))), '"'),
    operator: () =>
      choice("|>","=>","->","==","!=","<=",">=","&&","||","..=","..",
             "+","-","*","/","%","<",">","=","!","?"),
    punctuation: () => choice("{","}","(",")","[","]",",",":",";",".","#"),
  },
});
