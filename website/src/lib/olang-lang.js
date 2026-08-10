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
          match: '\\b(if|else|match|for|while|loop|break|continue|return|try|catch|await|in)\\b'
        },
        {
          name: 'storage.type.olang',
          match: '\\b(fn|let|mut|type|share|use|struct|enum|error|trait|impl|test|async|spawn)\\b'
        },
        { name: 'constant.language.olang', match: '\\b(true|false)\\b' },
        { name: 'support.class.promise.olang', match: '\\bPromise\\b' }
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

// The brand code theme: ink surface, teal keywords, amber strings, and the
// pipeline operator in full amber — the mark's chevron, in running code.
export const olangTheme = {
  name: 'olang-dark',
  type: 'dark',
  colors: {
    'editor.background': '#11161D',
    'editor.foreground': '#D6E2EC'
  },
  tokenColors: [
    { scope: ['comment'], settings: { foreground: '#5C7288', fontStyle: 'italic' } },
    { scope: ['string'], settings: { foreground: '#EBC776' } },
    { scope: ['constant.character.escape', 'punctuation.definition.template-expression'],
      settings: { foreground: '#FBBF24' } },
    { scope: ['constant.numeric'], settings: { foreground: '#8FE8DB' } },
    { scope: ['keyword.control', 'storage.type'], settings: { foreground: '#2DD4BF' } },
    { scope: ['constant.language', 'support.class.promise'],
      settings: { foreground: '#2DD4BF', fontStyle: 'bold' } },
    { scope: ['entity.name.type'], settings: { foreground: '#A5E8DE' } },
    { scope: ['entity.name.function'], settings: { foreground: '#E6EDF3' } },
    { scope: ['keyword.operator.pipeline'], settings: { foreground: '#FBBF24', fontStyle: 'bold' } },
    { scope: ['keyword.operator.arrow'], settings: { foreground: '#FBBF24' } },
    { scope: ['keyword.operator'], settings: { foreground: '#9FB6C9' } },
    // non-olang languages (toml, json, bash) reuse the same family
    { scope: ['support.type.property-name', 'entity.name.tag'], settings: { foreground: '#A5E8DE' } },
    { scope: ['punctuation'], settings: { foreground: '#9FB6C9' } }
  ]
};
