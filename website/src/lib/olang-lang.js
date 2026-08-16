// A TextMate grammar for olang, so code blocks get real token-level
// highlighting (not regex-soup guessing from a lookalike language).
// Kept in sync with grammar.pest's keyword list by hand — the keyword set
// changes rarely and deliberately (see docs/stability.md).
export const olangGrammar = {
  name: 'olang',
  scopeName: 'source.olang',
  fileTypes: ['ol'],
  patterns: [
    { include: '#comments' },
    { include: '#strings' },
    { include: '#numbers' },
    { include: '#keywords' },
    { include: '#types' },
    { include: '#functions' },
    { include: '#operators' }
  ],
  repository: {
    comments: {
      patterns: [{ name: 'comment.line.double-slash.olang', match: '//.*$' }]
    },
    strings: {
      patterns: [
        {
          name: 'string.quoted.raw.olang',
          begin: 'r"', end: '"',
          patterns: [{ name: 'constant.character.escape.olang', match: '\\\\.' }]
        },
        {
          name: 'string.quoted.double.olang',
          begin: '"', end: '"',
          patterns: [{ name: 'constant.character.escape.olang', match: '\\\\.' }]
        },
        {
          name: 'string.template.olang',
          begin: '`', end: '`',
          patterns: [
            { name: 'punctuation.definition.template-expression.olang', match: '\\$\\{[^}]*\\}' }
          ]
        }
      ]
    },
    numbers: {
      patterns: [
        { name: 'constant.numeric.olang', match: '\\b0[xX][0-9a-fA-F_]+\\b' },
        { name: 'constant.numeric.olang', match: '\\b0[bB][01_]+\\b' },
        { name: 'constant.numeric.olang', match: '\\b0[oO][0-7_]+\\b' },
        { name: 'constant.numeric.olang', match: '\\b\\d[\\d_]*(\\.\\d[\\d_]*)?([eE][+-]?\\d+)?\\b' }
      ]
    },
    keywords: {
      patterns: [
        {
          name: 'keyword.control.olang',
          match: '\\b(if|else|match|for|while|loop|break|continue|return|in|par)\\b'
        },
        {
          name: 'storage.type.olang',
          match: '\\b(fn|let|mut|type|share|use|struct|enum|error|trait|impl|test|spawn)\\b'
        },
        { name: 'constant.language.olang', match: '\\b(true|false)\\b' }
      ]
    },
    types: {
      patterns: [{ name: 'entity.name.type.olang', match: '\\b[A-Z][A-Za-z0-9_]*\\b' }]
    },
    functions: {
      patterns: [
        { name: 'entity.name.function.olang', match: '\\b[a-z_][A-Za-z0-9_]*(?=\\s*\\()' }
      ]
    },
    operators: {
      patterns: [
        { name: 'keyword.operator.pipeline.olang', match: '\\|>' },
        { name: 'keyword.operator.arrow.olang', match: '=>|->' },
        { name: 'keyword.operator.olang', match: '\\?|&&|\\|\\||==|!=|<=|>=|\\.\\.=?' }
      ]
    }
  }
};

// The brand code palette: ink surface, teal keywords, amber strings, and
// the pipeline operator in full amber — the mark's chevron, in running
// code. One object feeds two consumers: shiki's theme, which paints the
// book's code blocks at build time, and the playground editor, which
// paints itself on every keystroke. Changing a colour here changes both,
// which is the only way the two stay in agreement.
export const PALETTE = {
  bg: '#11161D',
  plain: '#D6E2EC',
  comment: '#5C7288',
  string: '#EBC776',
  escape: '#FBBF24',
  number: '#8FE8DB',
  keyword: '#2DD4BF',
  const: '#2DD4BF',
  type: '#A5E8DE',
  fn: '#E6EDF3',
  pipe: '#FBBF24',
  arrow: '#FBBF24',
  op: '#9FB6C9'
};

export const olangTheme = {
  name: 'olang-dark',
  type: 'dark',
  colors: {
    'editor.background': PALETTE.bg,
    'editor.foreground': PALETTE.plain
  },
  tokenColors: [
    { scope: ['comment'], settings: { foreground: PALETTE.comment, fontStyle: 'italic' } },
    { scope: ['string'], settings: { foreground: PALETTE.string } },
    { scope: ['constant.character.escape', 'punctuation.definition.template-expression'],
      settings: { foreground: PALETTE.escape } },
    { scope: ['constant.numeric'], settings: { foreground: PALETTE.number } },
    { scope: ['keyword.control', 'storage.type'], settings: { foreground: PALETTE.keyword } },
    { scope: ['constant.language'], settings: { foreground: PALETTE.const, fontStyle: 'bold' } },
    { scope: ['entity.name.type'], settings: { foreground: PALETTE.type } },
    { scope: ['entity.name.function'], settings: { foreground: PALETTE.fn } },
    { scope: ['keyword.operator.pipeline'], settings: { foreground: PALETTE.pipe, fontStyle: 'bold' } },
    { scope: ['keyword.operator.arrow'], settings: { foreground: PALETTE.arrow } },
    { scope: ['keyword.operator'], settings: { foreground: PALETTE.op } },
    // non-olang languages (toml, json, bash) reuse the same family
    { scope: ['support.type.property-name', 'entity.name.tag'], settings: { foreground: PALETTE.type } },
    { scope: ['punctuation'], settings: { foreground: PALETTE.op } }
  ]
};
