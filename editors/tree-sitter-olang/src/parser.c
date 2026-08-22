#include "tree_sitter/parser.h"

#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic ignored "-Wmissing-field-initializers"
#endif

#define LANGUAGE_VERSION 14
#define STATE_COUNT 23
#define LARGE_STATE_COUNT 14
#define SYMBOL_COUNT 83
#define ALIAS_COUNT 0
#define TOKEN_COUNT 71
#define EXTERNAL_TOKEN_COUNT 0
#define FIELD_COUNT 0
#define MAX_ALIAS_SEQUENCE_LENGTH 3
#define PRODUCTION_ID_COUNT 1

enum ts_symbol_identifiers {
  sym_comment = 1,
  anon_sym_fn = 2,
  anon_sym_let = 3,
  anon_sym_mut = 4,
  anon_sym_type = 5,
  anon_sym_if = 6,
  anon_sym_else = 7,
  anon_sym_match = 8,
  anon_sym_for = 9,
  anon_sym_par = 10,
  anon_sym_while = 11,
  anon_sym_loop = 12,
  anon_sym_break = 13,
  anon_sym_continue = 14,
  anon_sym_return = 15,
  anon_sym_spawn = 16,
  anon_sym_meta = 17,
  anon_sym_error = 18,
  anon_sym_share = 19,
  anon_sym_use = 20,
  anon_sym_struct = 21,
  anon_sym_enum = 22,
  anon_sym_test = 23,
  anon_sym_trait = 24,
  anon_sym_impl = 25,
  anon_sym_in = 26,
  anon_sym_true = 27,
  anon_sym_false = 28,
  sym_type_identifier = 29,
  sym_identifier = 30,
  sym_number = 31,
  anon_sym_DQUOTE = 32,
  aux_sym_string_token1 = 33,
  aux_sym_string_token2 = 34,
  anon_sym_BQUOTE = 35,
  aux_sym_template_token1 = 36,
  anon_sym_r_DQUOTE = 37,
  sym_macro = 38,
  anon_sym_PIPE_GT = 39,
  anon_sym_EQ_GT = 40,
  anon_sym_DASH_GT = 41,
  anon_sym_EQ_EQ = 42,
  anon_sym_BANG_EQ = 43,
  anon_sym_LT_EQ = 44,
  anon_sym_GT_EQ = 45,
  anon_sym_AMP_AMP = 46,
  anon_sym_PIPE_PIPE = 47,
  anon_sym_DOT_DOT_EQ = 48,
  anon_sym_DOT_DOT = 49,
  anon_sym_PLUS = 50,
  anon_sym_DASH = 51,
  anon_sym_STAR = 52,
  anon_sym_SLASH = 53,
  anon_sym_PERCENT = 54,
  anon_sym_LT = 55,
  anon_sym_GT = 56,
  anon_sym_EQ = 57,
  anon_sym_BANG = 58,
  anon_sym_QMARK = 59,
  anon_sym_LBRACE = 60,
  anon_sym_RBRACE = 61,
  anon_sym_LPAREN = 62,
  anon_sym_RPAREN = 63,
  anon_sym_LBRACK = 64,
  anon_sym_RBRACK = 65,
  anon_sym_COMMA = 66,
  anon_sym_COLON = 67,
  anon_sym_SEMI = 68,
  anon_sym_DOT = 69,
  anon_sym_POUND = 70,
  sym_source_file = 71,
  sym__token = 72,
  sym_keyword = 73,
  sym_boolean = 74,
  sym_string = 75,
  sym_template = 76,
  sym_raw_string = 77,
  sym_operator = 78,
  sym_punctuation = 79,
  aux_sym_source_file_repeat1 = 80,
  aux_sym_string_repeat1 = 81,
  aux_sym_template_repeat1 = 82,
};

static const char * const ts_symbol_names[] = {
  [ts_builtin_sym_end] = "end",
  [sym_comment] = "comment",
  [anon_sym_fn] = "fn",
  [anon_sym_let] = "let",
  [anon_sym_mut] = "mut",
  [anon_sym_type] = "type",
  [anon_sym_if] = "if",
  [anon_sym_else] = "else",
  [anon_sym_match] = "match",
  [anon_sym_for] = "for",
  [anon_sym_par] = "par",
  [anon_sym_while] = "while",
  [anon_sym_loop] = "loop",
  [anon_sym_break] = "break",
  [anon_sym_continue] = "continue",
  [anon_sym_return] = "return",
  [anon_sym_spawn] = "spawn",
  [anon_sym_meta] = "meta",
  [anon_sym_error] = "error",
  [anon_sym_share] = "share",
  [anon_sym_use] = "use",
  [anon_sym_struct] = "struct",
  [anon_sym_enum] = "enum",
  [anon_sym_test] = "test",
  [anon_sym_trait] = "trait",
  [anon_sym_impl] = "impl",
  [anon_sym_in] = "in",
  [anon_sym_true] = "true",
  [anon_sym_false] = "false",
  [sym_type_identifier] = "type_identifier",
  [sym_identifier] = "identifier",
  [sym_number] = "number",
  [anon_sym_DQUOTE] = "\"",
  [aux_sym_string_token1] = "string_token1",
  [aux_sym_string_token2] = "string_token2",
  [anon_sym_BQUOTE] = "`",
  [aux_sym_template_token1] = "template_token1",
  [anon_sym_r_DQUOTE] = "r\"",
  [sym_macro] = "macro",
  [anon_sym_PIPE_GT] = "|>",
  [anon_sym_EQ_GT] = "=>",
  [anon_sym_DASH_GT] = "->",
  [anon_sym_EQ_EQ] = "==",
  [anon_sym_BANG_EQ] = "!=",
  [anon_sym_LT_EQ] = "<=",
  [anon_sym_GT_EQ] = ">=",
  [anon_sym_AMP_AMP] = "&&",
  [anon_sym_PIPE_PIPE] = "||",
  [anon_sym_DOT_DOT_EQ] = "..=",
  [anon_sym_DOT_DOT] = "..",
  [anon_sym_PLUS] = "+",
  [anon_sym_DASH] = "-",
  [anon_sym_STAR] = "*",
  [anon_sym_SLASH] = "/",
  [anon_sym_PERCENT] = "%",
  [anon_sym_LT] = "<",
  [anon_sym_GT] = ">",
  [anon_sym_EQ] = "=",
  [anon_sym_BANG] = "!",
  [anon_sym_QMARK] = "\?",
  [anon_sym_LBRACE] = "{",
  [anon_sym_RBRACE] = "}",
  [anon_sym_LPAREN] = "(",
  [anon_sym_RPAREN] = ")",
  [anon_sym_LBRACK] = "[",
  [anon_sym_RBRACK] = "]",
  [anon_sym_COMMA] = ",",
  [anon_sym_COLON] = ":",
  [anon_sym_SEMI] = ";",
  [anon_sym_DOT] = ".",
  [anon_sym_POUND] = "#",
  [sym_source_file] = "source_file",
  [sym__token] = "_token",
  [sym_keyword] = "keyword",
  [sym_boolean] = "boolean",
  [sym_string] = "string",
  [sym_template] = "template",
  [sym_raw_string] = "raw_string",
  [sym_operator] = "operator",
  [sym_punctuation] = "punctuation",
  [aux_sym_source_file_repeat1] = "source_file_repeat1",
  [aux_sym_string_repeat1] = "string_repeat1",
  [aux_sym_template_repeat1] = "template_repeat1",
};

static const TSSymbol ts_symbol_map[] = {
  [ts_builtin_sym_end] = ts_builtin_sym_end,
  [sym_comment] = sym_comment,
  [anon_sym_fn] = anon_sym_fn,
  [anon_sym_let] = anon_sym_let,
  [anon_sym_mut] = anon_sym_mut,
  [anon_sym_type] = anon_sym_type,
  [anon_sym_if] = anon_sym_if,
  [anon_sym_else] = anon_sym_else,
  [anon_sym_match] = anon_sym_match,
  [anon_sym_for] = anon_sym_for,
  [anon_sym_par] = anon_sym_par,
  [anon_sym_while] = anon_sym_while,
  [anon_sym_loop] = anon_sym_loop,
  [anon_sym_break] = anon_sym_break,
  [anon_sym_continue] = anon_sym_continue,
  [anon_sym_return] = anon_sym_return,
  [anon_sym_spawn] = anon_sym_spawn,
  [anon_sym_meta] = anon_sym_meta,
  [anon_sym_error] = anon_sym_error,
  [anon_sym_share] = anon_sym_share,
  [anon_sym_use] = anon_sym_use,
  [anon_sym_struct] = anon_sym_struct,
  [anon_sym_enum] = anon_sym_enum,
  [anon_sym_test] = anon_sym_test,
  [anon_sym_trait] = anon_sym_trait,
  [anon_sym_impl] = anon_sym_impl,
  [anon_sym_in] = anon_sym_in,
  [anon_sym_true] = anon_sym_true,
  [anon_sym_false] = anon_sym_false,
  [sym_type_identifier] = sym_type_identifier,
  [sym_identifier] = sym_identifier,
  [sym_number] = sym_number,
  [anon_sym_DQUOTE] = anon_sym_DQUOTE,
  [aux_sym_string_token1] = aux_sym_string_token1,
  [aux_sym_string_token2] = aux_sym_string_token2,
  [anon_sym_BQUOTE] = anon_sym_BQUOTE,
  [aux_sym_template_token1] = aux_sym_template_token1,
  [anon_sym_r_DQUOTE] = anon_sym_r_DQUOTE,
  [sym_macro] = sym_macro,
  [anon_sym_PIPE_GT] = anon_sym_PIPE_GT,
  [anon_sym_EQ_GT] = anon_sym_EQ_GT,
  [anon_sym_DASH_GT] = anon_sym_DASH_GT,
  [anon_sym_EQ_EQ] = anon_sym_EQ_EQ,
  [anon_sym_BANG_EQ] = anon_sym_BANG_EQ,
  [anon_sym_LT_EQ] = anon_sym_LT_EQ,
  [anon_sym_GT_EQ] = anon_sym_GT_EQ,
  [anon_sym_AMP_AMP] = anon_sym_AMP_AMP,
  [anon_sym_PIPE_PIPE] = anon_sym_PIPE_PIPE,
  [anon_sym_DOT_DOT_EQ] = anon_sym_DOT_DOT_EQ,
  [anon_sym_DOT_DOT] = anon_sym_DOT_DOT,
  [anon_sym_PLUS] = anon_sym_PLUS,
  [anon_sym_DASH] = anon_sym_DASH,
  [anon_sym_STAR] = anon_sym_STAR,
  [anon_sym_SLASH] = anon_sym_SLASH,
  [anon_sym_PERCENT] = anon_sym_PERCENT,
  [anon_sym_LT] = anon_sym_LT,
  [anon_sym_GT] = anon_sym_GT,
  [anon_sym_EQ] = anon_sym_EQ,
  [anon_sym_BANG] = anon_sym_BANG,
  [anon_sym_QMARK] = anon_sym_QMARK,
  [anon_sym_LBRACE] = anon_sym_LBRACE,
  [anon_sym_RBRACE] = anon_sym_RBRACE,
  [anon_sym_LPAREN] = anon_sym_LPAREN,
  [anon_sym_RPAREN] = anon_sym_RPAREN,
  [anon_sym_LBRACK] = anon_sym_LBRACK,
  [anon_sym_RBRACK] = anon_sym_RBRACK,
  [anon_sym_COMMA] = anon_sym_COMMA,
  [anon_sym_COLON] = anon_sym_COLON,
  [anon_sym_SEMI] = anon_sym_SEMI,
  [anon_sym_DOT] = anon_sym_DOT,
  [anon_sym_POUND] = anon_sym_POUND,
  [sym_source_file] = sym_source_file,
  [sym__token] = sym__token,
  [sym_keyword] = sym_keyword,
  [sym_boolean] = sym_boolean,
  [sym_string] = sym_string,
  [sym_template] = sym_template,
  [sym_raw_string] = sym_raw_string,
  [sym_operator] = sym_operator,
  [sym_punctuation] = sym_punctuation,
  [aux_sym_source_file_repeat1] = aux_sym_source_file_repeat1,
  [aux_sym_string_repeat1] = aux_sym_string_repeat1,
  [aux_sym_template_repeat1] = aux_sym_template_repeat1,
};

static const TSSymbolMetadata ts_symbol_metadata[] = {
  [ts_builtin_sym_end] = {
    .visible = false,
    .named = true,
  },
  [sym_comment] = {
    .visible = true,
    .named = true,
  },
  [anon_sym_fn] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_let] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_mut] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_type] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_if] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_else] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_match] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_for] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_par] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_while] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_loop] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_break] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_continue] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_return] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_spawn] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_meta] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_error] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_share] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_use] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_struct] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_enum] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_test] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_trait] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_impl] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_in] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_true] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_false] = {
    .visible = true,
    .named = false,
  },
  [sym_type_identifier] = {
    .visible = true,
    .named = true,
  },
  [sym_identifier] = {
    .visible = true,
    .named = true,
  },
  [sym_number] = {
    .visible = true,
    .named = true,
  },
  [anon_sym_DQUOTE] = {
    .visible = true,
    .named = false,
  },
  [aux_sym_string_token1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_string_token2] = {
    .visible = false,
    .named = false,
  },
  [anon_sym_BQUOTE] = {
    .visible = true,
    .named = false,
  },
  [aux_sym_template_token1] = {
    .visible = false,
    .named = false,
  },
  [anon_sym_r_DQUOTE] = {
    .visible = true,
    .named = false,
  },
  [sym_macro] = {
    .visible = true,
    .named = true,
  },
  [anon_sym_PIPE_GT] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_EQ_GT] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_DASH_GT] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_EQ_EQ] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_BANG_EQ] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_LT_EQ] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_GT_EQ] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_AMP_AMP] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_PIPE_PIPE] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_DOT_DOT_EQ] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_DOT_DOT] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_PLUS] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_DASH] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_STAR] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_SLASH] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_PERCENT] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_LT] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_GT] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_EQ] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_BANG] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_QMARK] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_LBRACE] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_RBRACE] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_LPAREN] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_RPAREN] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_LBRACK] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_RBRACK] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_COMMA] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_COLON] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_SEMI] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_DOT] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_POUND] = {
    .visible = true,
    .named = false,
  },
  [sym_source_file] = {
    .visible = true,
    .named = true,
  },
  [sym__token] = {
    .visible = false,
    .named = true,
  },
  [sym_keyword] = {
    .visible = true,
    .named = true,
  },
  [sym_boolean] = {
    .visible = true,
    .named = true,
  },
  [sym_string] = {
    .visible = true,
    .named = true,
  },
  [sym_template] = {
    .visible = true,
    .named = true,
  },
  [sym_raw_string] = {
    .visible = true,
    .named = true,
  },
  [sym_operator] = {
    .visible = true,
    .named = true,
  },
  [sym_punctuation] = {
    .visible = true,
    .named = true,
  },
  [aux_sym_source_file_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_string_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_template_repeat1] = {
    .visible = false,
    .named = false,
  },
};

static const TSSymbol ts_alias_sequences[PRODUCTION_ID_COUNT][MAX_ALIAS_SEQUENCE_LENGTH] = {
  [0] = {0},
};

static const uint16_t ts_non_terminal_alias_map[] = {
  0,
};

static const TSStateId ts_primary_state_ids[STATE_COUNT] = {
  [0] = 0,
  [1] = 1,
  [2] = 2,
  [3] = 3,
  [4] = 4,
  [5] = 5,
  [6] = 6,
  [7] = 7,
  [8] = 8,
  [9] = 9,
  [10] = 10,
  [11] = 11,
  [12] = 12,
  [13] = 13,
  [14] = 14,
  [15] = 15,
  [16] = 16,
  [17] = 17,
  [18] = 18,
  [19] = 19,
  [20] = 20,
  [21] = 21,
  [22] = 22,
};

static bool ts_lex(TSLexer *lexer, TSStateId state) {
  START_LEXER();
  eof = lexer->eof(lexer);
  switch (state) {
    case 0:
      if (eof) ADVANCE(8);
      ADVANCE_MAP(
        '!', 144,
        '"', 114,
        '#', 156,
        '%', 140,
        '&', 2,
        '(', 148,
        ')', 149,
        '*', 138,
        '+', 136,
        ',', 152,
        '-', 137,
        '.', 155,
        '/', 139,
        ':', 153,
        ';', 154,
        '<', 141,
        '=', 143,
        '>', 142,
        '?', 145,
        '@', 6,
        '[', 150,
        '\\', 7,
        ']', 151,
        '`', 119,
        'b', 85,
        'c', 79,
        'e', 72,
        'f', 41,
        'i', 62,
        'l', 51,
        'm', 45,
        'p', 47,
        'r', 40,
        's', 65,
        't', 60,
        'u', 93,
        'w', 63,
        '{', 146,
        '|', 4,
        '}', 147,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(0);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(112);
      if (('_' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      if (('A' <= lookahead && lookahead <= 'Z')) ADVANCE(39);
      END_STATE();
    case 1:
      if (lookahead == '"') ADVANCE(114);
      if (lookahead == '/') ADVANCE(116);
      if (lookahead == '\\') ADVANCE(7);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') ADVANCE(115);
      if (lookahead != 0) ADVANCE(117);
      END_STATE();
    case 2:
      if (lookahead == '&') ADVANCE(132);
      END_STATE();
    case 3:
      if (lookahead == '/') ADVANCE(121);
      if (lookahead == '\\') ADVANCE(7);
      if (lookahead == '`') ADVANCE(119);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') ADVANCE(120);
      if (lookahead != 0) ADVANCE(122);
      END_STATE();
    case 4:
      if (lookahead == '>') ADVANCE(125);
      if (lookahead == '|') ADVANCE(133);
      END_STATE();
    case 5:
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(113);
      END_STATE();
    case 6:
      if (lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(124);
      END_STATE();
    case 7:
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(118);
      END_STATE();
    case 8:
      ACCEPT_TOKEN(ts_builtin_sym_end);
      END_STATE();
    case 9:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead == '\n') ADVANCE(117);
      if (lookahead == '"' ||
          lookahead == '\\') ADVANCE(11);
      if (lookahead != 0) ADVANCE(9);
      END_STATE();
    case 10:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead == '\n') ADVANCE(122);
      if (lookahead == '\\' ||
          lookahead == '`') ADVANCE(11);
      if (lookahead != 0) ADVANCE(10);
      END_STATE();
    case 11:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(11);
      END_STATE();
    case 12:
      ACCEPT_TOKEN(anon_sym_fn);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 13:
      ACCEPT_TOKEN(anon_sym_let);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 14:
      ACCEPT_TOKEN(anon_sym_mut);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 15:
      ACCEPT_TOKEN(anon_sym_type);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 16:
      ACCEPT_TOKEN(anon_sym_if);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 17:
      ACCEPT_TOKEN(anon_sym_else);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 18:
      ACCEPT_TOKEN(anon_sym_match);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 19:
      ACCEPT_TOKEN(anon_sym_for);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 20:
      ACCEPT_TOKEN(anon_sym_par);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 21:
      ACCEPT_TOKEN(anon_sym_while);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 22:
      ACCEPT_TOKEN(anon_sym_loop);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 23:
      ACCEPT_TOKEN(anon_sym_break);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 24:
      ACCEPT_TOKEN(anon_sym_continue);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 25:
      ACCEPT_TOKEN(anon_sym_return);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 26:
      ACCEPT_TOKEN(anon_sym_spawn);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 27:
      ACCEPT_TOKEN(anon_sym_meta);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 28:
      ACCEPT_TOKEN(anon_sym_error);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 29:
      ACCEPT_TOKEN(anon_sym_share);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 30:
      ACCEPT_TOKEN(anon_sym_use);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 31:
      ACCEPT_TOKEN(anon_sym_struct);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 32:
      ACCEPT_TOKEN(anon_sym_enum);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 33:
      ACCEPT_TOKEN(anon_sym_test);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 34:
      ACCEPT_TOKEN(anon_sym_trait);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 35:
      ACCEPT_TOKEN(anon_sym_impl);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 36:
      ACCEPT_TOKEN(anon_sym_in);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 37:
      ACCEPT_TOKEN(anon_sym_true);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 38:
      ACCEPT_TOKEN(anon_sym_false);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 39:
      ACCEPT_TOKEN(sym_type_identifier);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(39);
      END_STATE();
    case 40:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '"') ADVANCE(123);
      if (lookahead == 'e') ADVANCE(103);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 41:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(73);
      if (lookahead == 'n') ADVANCE(12);
      if (lookahead == 'o') ADVANCE(86);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 42:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(110);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 43:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(69);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 44:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(27);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 45:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(98);
      if (lookahead == 'e') ADVANCE(105);
      if (lookahead == 'u') ADVANCE(99);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 46:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(68);
      if (lookahead == 'u') ADVANCE(54);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 47:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(87);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 48:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(92);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 49:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(64);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 50:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(102);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 51:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(97);
      if (lookahead == 'o') ADVANCE(80);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 52:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(30);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 53:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(17);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 54:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(37);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 55:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(15);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 56:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(38);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 57:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(29);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 58:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(21);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 59:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(24);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 60:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(96);
      if (lookahead == 'r') ADVANCE(46);
      if (lookahead == 'y') ADVANCE(84);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 61:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(43);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 62:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'f') ADVANCE(16);
      if (lookahead == 'm') ADVANCE(83);
      if (lookahead == 'n') ADVANCE(36);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 63:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(67);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 64:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(18);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 65:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(48);
      if (lookahead == 'p') ADVANCE(42);
      if (lookahead == 't') ADVANCE(90);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 66:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(77);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 67:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(71);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 68:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(101);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 69:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'k') ADVANCE(23);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 70:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(35);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 71:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(58);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 72:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(94);
      if (lookahead == 'n') ADVANCE(106);
      if (lookahead == 'r') ADVANCE(89);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 73:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(95);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 74:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'm') ADVANCE(32);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 75:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(26);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 76:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(25);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 77:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(109);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 78:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(104);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 79:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(78);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 80:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(82);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 81:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(88);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 82:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(22);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 83:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(70);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 84:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(55);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 85:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(61);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 86:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(19);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 87:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(20);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 88:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(28);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 89:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(81);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 90:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(107);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 91:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(76);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 92:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(57);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 93:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(52);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 94:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(53);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 95:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(56);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 96:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(100);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 97:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(13);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 98:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(49);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 99:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(14);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 100:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(33);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 101:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(34);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 102:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(31);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 103:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(108);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 104:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(66);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 105:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(44);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 106:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'u') ADVANCE(74);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 107:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'u') ADVANCE(50);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 108:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'u') ADVANCE(91);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 109:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'u') ADVANCE(59);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 110:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'w') ADVANCE(75);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 111:
      ACCEPT_TOKEN(sym_identifier);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(111);
      END_STATE();
    case 112:
      ACCEPT_TOKEN(sym_number);
      if (lookahead == '.') ADVANCE(5);
      if (('0' <= lookahead && lookahead <= '9') ||
          lookahead == '_') ADVANCE(112);
      END_STATE();
    case 113:
      ACCEPT_TOKEN(sym_number);
      if (('0' <= lookahead && lookahead <= '9') ||
          lookahead == '_') ADVANCE(113);
      END_STATE();
    case 114:
      ACCEPT_TOKEN(anon_sym_DQUOTE);
      END_STATE();
    case 115:
      ACCEPT_TOKEN(aux_sym_string_token1);
      if (lookahead == '/') ADVANCE(116);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') ADVANCE(115);
      if (lookahead != 0 &&
          lookahead != '"' &&
          lookahead != '\\') ADVANCE(117);
      END_STATE();
    case 116:
      ACCEPT_TOKEN(aux_sym_string_token1);
      if (lookahead == '/') ADVANCE(9);
      if (lookahead != 0 &&
          lookahead != '"' &&
          lookahead != '\\') ADVANCE(117);
      END_STATE();
    case 117:
      ACCEPT_TOKEN(aux_sym_string_token1);
      if (lookahead != 0 &&
          lookahead != '"' &&
          lookahead != '\\') ADVANCE(117);
      END_STATE();
    case 118:
      ACCEPT_TOKEN(aux_sym_string_token2);
      END_STATE();
    case 119:
      ACCEPT_TOKEN(anon_sym_BQUOTE);
      END_STATE();
    case 120:
      ACCEPT_TOKEN(aux_sym_template_token1);
      if (lookahead == '/') ADVANCE(121);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') ADVANCE(120);
      if (lookahead != 0 &&
          lookahead != '\\' &&
          lookahead != '`') ADVANCE(122);
      END_STATE();
    case 121:
      ACCEPT_TOKEN(aux_sym_template_token1);
      if (lookahead == '/') ADVANCE(10);
      if (lookahead != 0 &&
          lookahead != '\\' &&
          lookahead != '`') ADVANCE(122);
      END_STATE();
    case 122:
      ACCEPT_TOKEN(aux_sym_template_token1);
      if (lookahead != 0 &&
          lookahead != '\\' &&
          lookahead != '`') ADVANCE(122);
      END_STATE();
    case 123:
      ACCEPT_TOKEN(anon_sym_r_DQUOTE);
      END_STATE();
    case 124:
      ACCEPT_TOKEN(sym_macro);
      if (('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(124);
      END_STATE();
    case 125:
      ACCEPT_TOKEN(anon_sym_PIPE_GT);
      END_STATE();
    case 126:
      ACCEPT_TOKEN(anon_sym_EQ_GT);
      END_STATE();
    case 127:
      ACCEPT_TOKEN(anon_sym_DASH_GT);
      END_STATE();
    case 128:
      ACCEPT_TOKEN(anon_sym_EQ_EQ);
      END_STATE();
    case 129:
      ACCEPT_TOKEN(anon_sym_BANG_EQ);
      END_STATE();
    case 130:
      ACCEPT_TOKEN(anon_sym_LT_EQ);
      END_STATE();
    case 131:
      ACCEPT_TOKEN(anon_sym_GT_EQ);
      END_STATE();
    case 132:
      ACCEPT_TOKEN(anon_sym_AMP_AMP);
      END_STATE();
    case 133:
      ACCEPT_TOKEN(anon_sym_PIPE_PIPE);
      END_STATE();
    case 134:
      ACCEPT_TOKEN(anon_sym_DOT_DOT_EQ);
      END_STATE();
    case 135:
      ACCEPT_TOKEN(anon_sym_DOT_DOT);
      if (lookahead == '=') ADVANCE(134);
      END_STATE();
    case 136:
      ACCEPT_TOKEN(anon_sym_PLUS);
      END_STATE();
    case 137:
      ACCEPT_TOKEN(anon_sym_DASH);
      if (lookahead == '>') ADVANCE(127);
      END_STATE();
    case 138:
      ACCEPT_TOKEN(anon_sym_STAR);
      END_STATE();
    case 139:
      ACCEPT_TOKEN(anon_sym_SLASH);
      if (lookahead == '/') ADVANCE(11);
      END_STATE();
    case 140:
      ACCEPT_TOKEN(anon_sym_PERCENT);
      END_STATE();
    case 141:
      ACCEPT_TOKEN(anon_sym_LT);
      if (lookahead == '=') ADVANCE(130);
      END_STATE();
    case 142:
      ACCEPT_TOKEN(anon_sym_GT);
      if (lookahead == '=') ADVANCE(131);
      END_STATE();
    case 143:
      ACCEPT_TOKEN(anon_sym_EQ);
      if (lookahead == '=') ADVANCE(128);
      if (lookahead == '>') ADVANCE(126);
      END_STATE();
    case 144:
      ACCEPT_TOKEN(anon_sym_BANG);
      if (lookahead == '=') ADVANCE(129);
      END_STATE();
    case 145:
      ACCEPT_TOKEN(anon_sym_QMARK);
      END_STATE();
    case 146:
      ACCEPT_TOKEN(anon_sym_LBRACE);
      END_STATE();
    case 147:
      ACCEPT_TOKEN(anon_sym_RBRACE);
      END_STATE();
    case 148:
      ACCEPT_TOKEN(anon_sym_LPAREN);
      END_STATE();
    case 149:
      ACCEPT_TOKEN(anon_sym_RPAREN);
      END_STATE();
    case 150:
      ACCEPT_TOKEN(anon_sym_LBRACK);
      END_STATE();
    case 151:
      ACCEPT_TOKEN(anon_sym_RBRACK);
      END_STATE();
    case 152:
      ACCEPT_TOKEN(anon_sym_COMMA);
      END_STATE();
    case 153:
      ACCEPT_TOKEN(anon_sym_COLON);
      END_STATE();
    case 154:
      ACCEPT_TOKEN(anon_sym_SEMI);
      END_STATE();
    case 155:
      ACCEPT_TOKEN(anon_sym_DOT);
      if (lookahead == '.') ADVANCE(135);
      END_STATE();
    case 156:
      ACCEPT_TOKEN(anon_sym_POUND);
      END_STATE();
    default:
      return false;
  }
}

static const TSLexMode ts_lex_modes[STATE_COUNT] = {
  [0] = {.lex_state = 0},
  [1] = {.lex_state = 0},
  [2] = {.lex_state = 0},
  [3] = {.lex_state = 0},
  [4] = {.lex_state = 0},
  [5] = {.lex_state = 0},
  [6] = {.lex_state = 0},
  [7] = {.lex_state = 0},
  [8] = {.lex_state = 0},
  [9] = {.lex_state = 0},
  [10] = {.lex_state = 0},
  [11] = {.lex_state = 0},
  [12] = {.lex_state = 0},
  [13] = {.lex_state = 0},
  [14] = {.lex_state = 1},
  [15] = {.lex_state = 3},
  [16] = {.lex_state = 1},
  [17] = {.lex_state = 3},
  [18] = {.lex_state = 1},
  [19] = {.lex_state = 1},
  [20] = {.lex_state = 1},
  [21] = {.lex_state = 3},
  [22] = {.lex_state = 0},
};

static const uint16_t ts_parse_table[LARGE_STATE_COUNT][SYMBOL_COUNT] = {
  [0] = {
    [ts_builtin_sym_end] = ACTIONS(1),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(1),
    [anon_sym_let] = ACTIONS(1),
    [anon_sym_mut] = ACTIONS(1),
    [anon_sym_type] = ACTIONS(1),
    [anon_sym_if] = ACTIONS(1),
    [anon_sym_else] = ACTIONS(1),
    [anon_sym_match] = ACTIONS(1),
    [anon_sym_for] = ACTIONS(1),
    [anon_sym_par] = ACTIONS(1),
    [anon_sym_while] = ACTIONS(1),
    [anon_sym_loop] = ACTIONS(1),
    [anon_sym_break] = ACTIONS(1),
    [anon_sym_continue] = ACTIONS(1),
    [anon_sym_return] = ACTIONS(1),
    [anon_sym_spawn] = ACTIONS(1),
    [anon_sym_meta] = ACTIONS(1),
    [anon_sym_error] = ACTIONS(1),
    [anon_sym_share] = ACTIONS(1),
    [anon_sym_use] = ACTIONS(1),
    [anon_sym_struct] = ACTIONS(1),
    [anon_sym_enum] = ACTIONS(1),
    [anon_sym_test] = ACTIONS(1),
    [anon_sym_trait] = ACTIONS(1),
    [anon_sym_impl] = ACTIONS(1),
    [anon_sym_in] = ACTIONS(1),
    [anon_sym_true] = ACTIONS(1),
    [anon_sym_false] = ACTIONS(1),
    [sym_type_identifier] = ACTIONS(1),
    [sym_identifier] = ACTIONS(1),
    [sym_number] = ACTIONS(1),
    [anon_sym_DQUOTE] = ACTIONS(1),
    [aux_sym_string_token2] = ACTIONS(1),
    [anon_sym_BQUOTE] = ACTIONS(1),
    [anon_sym_r_DQUOTE] = ACTIONS(1),
    [sym_macro] = ACTIONS(1),
    [anon_sym_PIPE_GT] = ACTIONS(1),
    [anon_sym_EQ_GT] = ACTIONS(1),
    [anon_sym_DASH_GT] = ACTIONS(1),
    [anon_sym_EQ_EQ] = ACTIONS(1),
    [anon_sym_BANG_EQ] = ACTIONS(1),
    [anon_sym_LT_EQ] = ACTIONS(1),
    [anon_sym_GT_EQ] = ACTIONS(1),
    [anon_sym_AMP_AMP] = ACTIONS(1),
    [anon_sym_PIPE_PIPE] = ACTIONS(1),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(1),
    [anon_sym_DOT_DOT] = ACTIONS(1),
    [anon_sym_PLUS] = ACTIONS(1),
    [anon_sym_DASH] = ACTIONS(1),
    [anon_sym_STAR] = ACTIONS(1),
    [anon_sym_SLASH] = ACTIONS(1),
    [anon_sym_PERCENT] = ACTIONS(1),
    [anon_sym_LT] = ACTIONS(1),
    [anon_sym_GT] = ACTIONS(1),
    [anon_sym_EQ] = ACTIONS(1),
    [anon_sym_BANG] = ACTIONS(1),
    [anon_sym_QMARK] = ACTIONS(1),
    [anon_sym_LBRACE] = ACTIONS(1),
    [anon_sym_RBRACE] = ACTIONS(1),
    [anon_sym_LPAREN] = ACTIONS(1),
    [anon_sym_RPAREN] = ACTIONS(1),
    [anon_sym_LBRACK] = ACTIONS(1),
    [anon_sym_RBRACK] = ACTIONS(1),
    [anon_sym_COMMA] = ACTIONS(1),
    [anon_sym_COLON] = ACTIONS(1),
    [anon_sym_SEMI] = ACTIONS(1),
    [anon_sym_DOT] = ACTIONS(1),
    [anon_sym_POUND] = ACTIONS(1),
  },
  [1] = {
    [sym_source_file] = STATE(22),
    [sym__token] = STATE(2),
    [sym_keyword] = STATE(2),
    [sym_boolean] = STATE(2),
    [sym_string] = STATE(2),
    [sym_template] = STATE(2),
    [sym_raw_string] = STATE(2),
    [sym_operator] = STATE(2),
    [sym_punctuation] = STATE(2),
    [aux_sym_source_file_repeat1] = STATE(2),
    [ts_builtin_sym_end] = ACTIONS(5),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(7),
    [anon_sym_let] = ACTIONS(7),
    [anon_sym_mut] = ACTIONS(7),
    [anon_sym_type] = ACTIONS(7),
    [anon_sym_if] = ACTIONS(7),
    [anon_sym_else] = ACTIONS(7),
    [anon_sym_match] = ACTIONS(7),
    [anon_sym_for] = ACTIONS(7),
    [anon_sym_par] = ACTIONS(7),
    [anon_sym_while] = ACTIONS(7),
    [anon_sym_loop] = ACTIONS(7),
    [anon_sym_break] = ACTIONS(7),
    [anon_sym_continue] = ACTIONS(7),
    [anon_sym_return] = ACTIONS(7),
    [anon_sym_spawn] = ACTIONS(7),
    [anon_sym_meta] = ACTIONS(7),
    [anon_sym_error] = ACTIONS(7),
    [anon_sym_share] = ACTIONS(7),
    [anon_sym_use] = ACTIONS(7),
    [anon_sym_struct] = ACTIONS(7),
    [anon_sym_enum] = ACTIONS(7),
    [anon_sym_test] = ACTIONS(7),
    [anon_sym_trait] = ACTIONS(7),
    [anon_sym_impl] = ACTIONS(7),
    [anon_sym_in] = ACTIONS(7),
    [anon_sym_true] = ACTIONS(9),
    [anon_sym_false] = ACTIONS(9),
    [sym_type_identifier] = ACTIONS(11),
    [sym_identifier] = ACTIONS(13),
    [sym_number] = ACTIONS(11),
    [anon_sym_DQUOTE] = ACTIONS(15),
    [anon_sym_BQUOTE] = ACTIONS(17),
    [anon_sym_r_DQUOTE] = ACTIONS(19),
    [sym_macro] = ACTIONS(11),
    [anon_sym_PIPE_GT] = ACTIONS(21),
    [anon_sym_EQ_GT] = ACTIONS(21),
    [anon_sym_DASH_GT] = ACTIONS(21),
    [anon_sym_EQ_EQ] = ACTIONS(21),
    [anon_sym_BANG_EQ] = ACTIONS(21),
    [anon_sym_LT_EQ] = ACTIONS(21),
    [anon_sym_GT_EQ] = ACTIONS(21),
    [anon_sym_AMP_AMP] = ACTIONS(21),
    [anon_sym_PIPE_PIPE] = ACTIONS(21),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(21),
    [anon_sym_DOT_DOT] = ACTIONS(23),
    [anon_sym_PLUS] = ACTIONS(21),
    [anon_sym_DASH] = ACTIONS(23),
    [anon_sym_STAR] = ACTIONS(21),
    [anon_sym_SLASH] = ACTIONS(23),
    [anon_sym_PERCENT] = ACTIONS(21),
    [anon_sym_LT] = ACTIONS(23),
    [anon_sym_GT] = ACTIONS(23),
    [anon_sym_EQ] = ACTIONS(23),
    [anon_sym_BANG] = ACTIONS(23),
    [anon_sym_QMARK] = ACTIONS(21),
    [anon_sym_LBRACE] = ACTIONS(25),
    [anon_sym_RBRACE] = ACTIONS(25),
    [anon_sym_LPAREN] = ACTIONS(25),
    [anon_sym_RPAREN] = ACTIONS(25),
    [anon_sym_LBRACK] = ACTIONS(25),
    [anon_sym_RBRACK] = ACTIONS(25),
    [anon_sym_COMMA] = ACTIONS(25),
    [anon_sym_COLON] = ACTIONS(25),
    [anon_sym_SEMI] = ACTIONS(25),
    [anon_sym_DOT] = ACTIONS(27),
    [anon_sym_POUND] = ACTIONS(25),
  },
  [2] = {
    [sym__token] = STATE(3),
    [sym_keyword] = STATE(3),
    [sym_boolean] = STATE(3),
    [sym_string] = STATE(3),
    [sym_template] = STATE(3),
    [sym_raw_string] = STATE(3),
    [sym_operator] = STATE(3),
    [sym_punctuation] = STATE(3),
    [aux_sym_source_file_repeat1] = STATE(3),
    [ts_builtin_sym_end] = ACTIONS(29),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(7),
    [anon_sym_let] = ACTIONS(7),
    [anon_sym_mut] = ACTIONS(7),
    [anon_sym_type] = ACTIONS(7),
    [anon_sym_if] = ACTIONS(7),
    [anon_sym_else] = ACTIONS(7),
    [anon_sym_match] = ACTIONS(7),
    [anon_sym_for] = ACTIONS(7),
    [anon_sym_par] = ACTIONS(7),
    [anon_sym_while] = ACTIONS(7),
    [anon_sym_loop] = ACTIONS(7),
    [anon_sym_break] = ACTIONS(7),
    [anon_sym_continue] = ACTIONS(7),
    [anon_sym_return] = ACTIONS(7),
    [anon_sym_spawn] = ACTIONS(7),
    [anon_sym_meta] = ACTIONS(7),
    [anon_sym_error] = ACTIONS(7),
    [anon_sym_share] = ACTIONS(7),
    [anon_sym_use] = ACTIONS(7),
    [anon_sym_struct] = ACTIONS(7),
    [anon_sym_enum] = ACTIONS(7),
    [anon_sym_test] = ACTIONS(7),
    [anon_sym_trait] = ACTIONS(7),
    [anon_sym_impl] = ACTIONS(7),
    [anon_sym_in] = ACTIONS(7),
    [anon_sym_true] = ACTIONS(9),
    [anon_sym_false] = ACTIONS(9),
    [sym_type_identifier] = ACTIONS(31),
    [sym_identifier] = ACTIONS(33),
    [sym_number] = ACTIONS(31),
    [anon_sym_DQUOTE] = ACTIONS(15),
    [anon_sym_BQUOTE] = ACTIONS(17),
    [anon_sym_r_DQUOTE] = ACTIONS(19),
    [sym_macro] = ACTIONS(31),
    [anon_sym_PIPE_GT] = ACTIONS(21),
    [anon_sym_EQ_GT] = ACTIONS(21),
    [anon_sym_DASH_GT] = ACTIONS(21),
    [anon_sym_EQ_EQ] = ACTIONS(21),
    [anon_sym_BANG_EQ] = ACTIONS(21),
    [anon_sym_LT_EQ] = ACTIONS(21),
    [anon_sym_GT_EQ] = ACTIONS(21),
    [anon_sym_AMP_AMP] = ACTIONS(21),
    [anon_sym_PIPE_PIPE] = ACTIONS(21),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(21),
    [anon_sym_DOT_DOT] = ACTIONS(23),
    [anon_sym_PLUS] = ACTIONS(21),
    [anon_sym_DASH] = ACTIONS(23),
    [anon_sym_STAR] = ACTIONS(21),
    [anon_sym_SLASH] = ACTIONS(23),
    [anon_sym_PERCENT] = ACTIONS(21),
    [anon_sym_LT] = ACTIONS(23),
    [anon_sym_GT] = ACTIONS(23),
    [anon_sym_EQ] = ACTIONS(23),
    [anon_sym_BANG] = ACTIONS(23),
    [anon_sym_QMARK] = ACTIONS(21),
    [anon_sym_LBRACE] = ACTIONS(25),
    [anon_sym_RBRACE] = ACTIONS(25),
    [anon_sym_LPAREN] = ACTIONS(25),
    [anon_sym_RPAREN] = ACTIONS(25),
    [anon_sym_LBRACK] = ACTIONS(25),
    [anon_sym_RBRACK] = ACTIONS(25),
    [anon_sym_COMMA] = ACTIONS(25),
    [anon_sym_COLON] = ACTIONS(25),
    [anon_sym_SEMI] = ACTIONS(25),
    [anon_sym_DOT] = ACTIONS(27),
    [anon_sym_POUND] = ACTIONS(25),
  },
  [3] = {
    [sym__token] = STATE(3),
    [sym_keyword] = STATE(3),
    [sym_boolean] = STATE(3),
    [sym_string] = STATE(3),
    [sym_template] = STATE(3),
    [sym_raw_string] = STATE(3),
    [sym_operator] = STATE(3),
    [sym_punctuation] = STATE(3),
    [aux_sym_source_file_repeat1] = STATE(3),
    [ts_builtin_sym_end] = ACTIONS(35),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(37),
    [anon_sym_let] = ACTIONS(37),
    [anon_sym_mut] = ACTIONS(37),
    [anon_sym_type] = ACTIONS(37),
    [anon_sym_if] = ACTIONS(37),
    [anon_sym_else] = ACTIONS(37),
    [anon_sym_match] = ACTIONS(37),
    [anon_sym_for] = ACTIONS(37),
    [anon_sym_par] = ACTIONS(37),
    [anon_sym_while] = ACTIONS(37),
    [anon_sym_loop] = ACTIONS(37),
    [anon_sym_break] = ACTIONS(37),
    [anon_sym_continue] = ACTIONS(37),
    [anon_sym_return] = ACTIONS(37),
    [anon_sym_spawn] = ACTIONS(37),
    [anon_sym_meta] = ACTIONS(37),
    [anon_sym_error] = ACTIONS(37),
    [anon_sym_share] = ACTIONS(37),
    [anon_sym_use] = ACTIONS(37),
    [anon_sym_struct] = ACTIONS(37),
    [anon_sym_enum] = ACTIONS(37),
    [anon_sym_test] = ACTIONS(37),
    [anon_sym_trait] = ACTIONS(37),
    [anon_sym_impl] = ACTIONS(37),
    [anon_sym_in] = ACTIONS(37),
    [anon_sym_true] = ACTIONS(40),
    [anon_sym_false] = ACTIONS(40),
    [sym_type_identifier] = ACTIONS(43),
    [sym_identifier] = ACTIONS(46),
    [sym_number] = ACTIONS(43),
    [anon_sym_DQUOTE] = ACTIONS(49),
    [anon_sym_BQUOTE] = ACTIONS(52),
    [anon_sym_r_DQUOTE] = ACTIONS(55),
    [sym_macro] = ACTIONS(43),
    [anon_sym_PIPE_GT] = ACTIONS(58),
    [anon_sym_EQ_GT] = ACTIONS(58),
    [anon_sym_DASH_GT] = ACTIONS(58),
    [anon_sym_EQ_EQ] = ACTIONS(58),
    [anon_sym_BANG_EQ] = ACTIONS(58),
    [anon_sym_LT_EQ] = ACTIONS(58),
    [anon_sym_GT_EQ] = ACTIONS(58),
    [anon_sym_AMP_AMP] = ACTIONS(58),
    [anon_sym_PIPE_PIPE] = ACTIONS(58),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(58),
    [anon_sym_DOT_DOT] = ACTIONS(61),
    [anon_sym_PLUS] = ACTIONS(58),
    [anon_sym_DASH] = ACTIONS(61),
    [anon_sym_STAR] = ACTIONS(58),
    [anon_sym_SLASH] = ACTIONS(61),
    [anon_sym_PERCENT] = ACTIONS(58),
    [anon_sym_LT] = ACTIONS(61),
    [anon_sym_GT] = ACTIONS(61),
    [anon_sym_EQ] = ACTIONS(61),
    [anon_sym_BANG] = ACTIONS(61),
    [anon_sym_QMARK] = ACTIONS(58),
    [anon_sym_LBRACE] = ACTIONS(64),
    [anon_sym_RBRACE] = ACTIONS(64),
    [anon_sym_LPAREN] = ACTIONS(64),
    [anon_sym_RPAREN] = ACTIONS(64),
    [anon_sym_LBRACK] = ACTIONS(64),
    [anon_sym_RBRACK] = ACTIONS(64),
    [anon_sym_COMMA] = ACTIONS(64),
    [anon_sym_COLON] = ACTIONS(64),
    [anon_sym_SEMI] = ACTIONS(64),
    [anon_sym_DOT] = ACTIONS(67),
    [anon_sym_POUND] = ACTIONS(64),
  },
  [4] = {
    [ts_builtin_sym_end] = ACTIONS(70),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(72),
    [anon_sym_let] = ACTIONS(72),
    [anon_sym_mut] = ACTIONS(72),
    [anon_sym_type] = ACTIONS(72),
    [anon_sym_if] = ACTIONS(72),
    [anon_sym_else] = ACTIONS(72),
    [anon_sym_match] = ACTIONS(72),
    [anon_sym_for] = ACTIONS(72),
    [anon_sym_par] = ACTIONS(72),
    [anon_sym_while] = ACTIONS(72),
    [anon_sym_loop] = ACTIONS(72),
    [anon_sym_break] = ACTIONS(72),
    [anon_sym_continue] = ACTIONS(72),
    [anon_sym_return] = ACTIONS(72),
    [anon_sym_spawn] = ACTIONS(72),
    [anon_sym_meta] = ACTIONS(72),
    [anon_sym_error] = ACTIONS(72),
    [anon_sym_share] = ACTIONS(72),
    [anon_sym_use] = ACTIONS(72),
    [anon_sym_struct] = ACTIONS(72),
    [anon_sym_enum] = ACTIONS(72),
    [anon_sym_test] = ACTIONS(72),
    [anon_sym_trait] = ACTIONS(72),
    [anon_sym_impl] = ACTIONS(72),
    [anon_sym_in] = ACTIONS(72),
    [anon_sym_true] = ACTIONS(72),
    [anon_sym_false] = ACTIONS(72),
    [sym_type_identifier] = ACTIONS(70),
    [sym_identifier] = ACTIONS(72),
    [sym_number] = ACTIONS(70),
    [anon_sym_DQUOTE] = ACTIONS(70),
    [anon_sym_BQUOTE] = ACTIONS(70),
    [anon_sym_r_DQUOTE] = ACTIONS(70),
    [sym_macro] = ACTIONS(70),
    [anon_sym_PIPE_GT] = ACTIONS(70),
    [anon_sym_EQ_GT] = ACTIONS(70),
    [anon_sym_DASH_GT] = ACTIONS(70),
    [anon_sym_EQ_EQ] = ACTIONS(70),
    [anon_sym_BANG_EQ] = ACTIONS(70),
    [anon_sym_LT_EQ] = ACTIONS(70),
    [anon_sym_GT_EQ] = ACTIONS(70),
    [anon_sym_AMP_AMP] = ACTIONS(70),
    [anon_sym_PIPE_PIPE] = ACTIONS(70),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(70),
    [anon_sym_DOT_DOT] = ACTIONS(72),
    [anon_sym_PLUS] = ACTIONS(70),
    [anon_sym_DASH] = ACTIONS(72),
    [anon_sym_STAR] = ACTIONS(70),
    [anon_sym_SLASH] = ACTIONS(72),
    [anon_sym_PERCENT] = ACTIONS(70),
    [anon_sym_LT] = ACTIONS(72),
    [anon_sym_GT] = ACTIONS(72),
    [anon_sym_EQ] = ACTIONS(72),
    [anon_sym_BANG] = ACTIONS(72),
    [anon_sym_QMARK] = ACTIONS(70),
    [anon_sym_LBRACE] = ACTIONS(70),
    [anon_sym_RBRACE] = ACTIONS(70),
    [anon_sym_LPAREN] = ACTIONS(70),
    [anon_sym_RPAREN] = ACTIONS(70),
    [anon_sym_LBRACK] = ACTIONS(70),
    [anon_sym_RBRACK] = ACTIONS(70),
    [anon_sym_COMMA] = ACTIONS(70),
    [anon_sym_COLON] = ACTIONS(70),
    [anon_sym_SEMI] = ACTIONS(70),
    [anon_sym_DOT] = ACTIONS(72),
    [anon_sym_POUND] = ACTIONS(70),
  },
  [5] = {
    [ts_builtin_sym_end] = ACTIONS(74),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(76),
    [anon_sym_let] = ACTIONS(76),
    [anon_sym_mut] = ACTIONS(76),
    [anon_sym_type] = ACTIONS(76),
    [anon_sym_if] = ACTIONS(76),
    [anon_sym_else] = ACTIONS(76),
    [anon_sym_match] = ACTIONS(76),
    [anon_sym_for] = ACTIONS(76),
    [anon_sym_par] = ACTIONS(76),
    [anon_sym_while] = ACTIONS(76),
    [anon_sym_loop] = ACTIONS(76),
    [anon_sym_break] = ACTIONS(76),
    [anon_sym_continue] = ACTIONS(76),
    [anon_sym_return] = ACTIONS(76),
    [anon_sym_spawn] = ACTIONS(76),
    [anon_sym_meta] = ACTIONS(76),
    [anon_sym_error] = ACTIONS(76),
    [anon_sym_share] = ACTIONS(76),
    [anon_sym_use] = ACTIONS(76),
    [anon_sym_struct] = ACTIONS(76),
    [anon_sym_enum] = ACTIONS(76),
    [anon_sym_test] = ACTIONS(76),
    [anon_sym_trait] = ACTIONS(76),
    [anon_sym_impl] = ACTIONS(76),
    [anon_sym_in] = ACTIONS(76),
    [anon_sym_true] = ACTIONS(76),
    [anon_sym_false] = ACTIONS(76),
    [sym_type_identifier] = ACTIONS(74),
    [sym_identifier] = ACTIONS(76),
    [sym_number] = ACTIONS(74),
    [anon_sym_DQUOTE] = ACTIONS(74),
    [anon_sym_BQUOTE] = ACTIONS(74),
    [anon_sym_r_DQUOTE] = ACTIONS(74),
    [sym_macro] = ACTIONS(74),
    [anon_sym_PIPE_GT] = ACTIONS(74),
    [anon_sym_EQ_GT] = ACTIONS(74),
    [anon_sym_DASH_GT] = ACTIONS(74),
    [anon_sym_EQ_EQ] = ACTIONS(74),
    [anon_sym_BANG_EQ] = ACTIONS(74),
    [anon_sym_LT_EQ] = ACTIONS(74),
    [anon_sym_GT_EQ] = ACTIONS(74),
    [anon_sym_AMP_AMP] = ACTIONS(74),
    [anon_sym_PIPE_PIPE] = ACTIONS(74),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(74),
    [anon_sym_DOT_DOT] = ACTIONS(76),
    [anon_sym_PLUS] = ACTIONS(74),
    [anon_sym_DASH] = ACTIONS(76),
    [anon_sym_STAR] = ACTIONS(74),
    [anon_sym_SLASH] = ACTIONS(76),
    [anon_sym_PERCENT] = ACTIONS(74),
    [anon_sym_LT] = ACTIONS(76),
    [anon_sym_GT] = ACTIONS(76),
    [anon_sym_EQ] = ACTIONS(76),
    [anon_sym_BANG] = ACTIONS(76),
    [anon_sym_QMARK] = ACTIONS(74),
    [anon_sym_LBRACE] = ACTIONS(74),
    [anon_sym_RBRACE] = ACTIONS(74),
    [anon_sym_LPAREN] = ACTIONS(74),
    [anon_sym_RPAREN] = ACTIONS(74),
    [anon_sym_LBRACK] = ACTIONS(74),
    [anon_sym_RBRACK] = ACTIONS(74),
    [anon_sym_COMMA] = ACTIONS(74),
    [anon_sym_COLON] = ACTIONS(74),
    [anon_sym_SEMI] = ACTIONS(74),
    [anon_sym_DOT] = ACTIONS(76),
    [anon_sym_POUND] = ACTIONS(74),
  },
  [6] = {
    [ts_builtin_sym_end] = ACTIONS(78),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(80),
    [anon_sym_let] = ACTIONS(80),
    [anon_sym_mut] = ACTIONS(80),
    [anon_sym_type] = ACTIONS(80),
    [anon_sym_if] = ACTIONS(80),
    [anon_sym_else] = ACTIONS(80),
    [anon_sym_match] = ACTIONS(80),
    [anon_sym_for] = ACTIONS(80),
    [anon_sym_par] = ACTIONS(80),
    [anon_sym_while] = ACTIONS(80),
    [anon_sym_loop] = ACTIONS(80),
    [anon_sym_break] = ACTIONS(80),
    [anon_sym_continue] = ACTIONS(80),
    [anon_sym_return] = ACTIONS(80),
    [anon_sym_spawn] = ACTIONS(80),
    [anon_sym_meta] = ACTIONS(80),
    [anon_sym_error] = ACTIONS(80),
    [anon_sym_share] = ACTIONS(80),
    [anon_sym_use] = ACTIONS(80),
    [anon_sym_struct] = ACTIONS(80),
    [anon_sym_enum] = ACTIONS(80),
    [anon_sym_test] = ACTIONS(80),
    [anon_sym_trait] = ACTIONS(80),
    [anon_sym_impl] = ACTIONS(80),
    [anon_sym_in] = ACTIONS(80),
    [anon_sym_true] = ACTIONS(80),
    [anon_sym_false] = ACTIONS(80),
    [sym_type_identifier] = ACTIONS(78),
    [sym_identifier] = ACTIONS(80),
    [sym_number] = ACTIONS(78),
    [anon_sym_DQUOTE] = ACTIONS(78),
    [anon_sym_BQUOTE] = ACTIONS(78),
    [anon_sym_r_DQUOTE] = ACTIONS(78),
    [sym_macro] = ACTIONS(78),
    [anon_sym_PIPE_GT] = ACTIONS(78),
    [anon_sym_EQ_GT] = ACTIONS(78),
    [anon_sym_DASH_GT] = ACTIONS(78),
    [anon_sym_EQ_EQ] = ACTIONS(78),
    [anon_sym_BANG_EQ] = ACTIONS(78),
    [anon_sym_LT_EQ] = ACTIONS(78),
    [anon_sym_GT_EQ] = ACTIONS(78),
    [anon_sym_AMP_AMP] = ACTIONS(78),
    [anon_sym_PIPE_PIPE] = ACTIONS(78),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(78),
    [anon_sym_DOT_DOT] = ACTIONS(80),
    [anon_sym_PLUS] = ACTIONS(78),
    [anon_sym_DASH] = ACTIONS(80),
    [anon_sym_STAR] = ACTIONS(78),
    [anon_sym_SLASH] = ACTIONS(80),
    [anon_sym_PERCENT] = ACTIONS(78),
    [anon_sym_LT] = ACTIONS(80),
    [anon_sym_GT] = ACTIONS(80),
    [anon_sym_EQ] = ACTIONS(80),
    [anon_sym_BANG] = ACTIONS(80),
    [anon_sym_QMARK] = ACTIONS(78),
    [anon_sym_LBRACE] = ACTIONS(78),
    [anon_sym_RBRACE] = ACTIONS(78),
    [anon_sym_LPAREN] = ACTIONS(78),
    [anon_sym_RPAREN] = ACTIONS(78),
    [anon_sym_LBRACK] = ACTIONS(78),
    [anon_sym_RBRACK] = ACTIONS(78),
    [anon_sym_COMMA] = ACTIONS(78),
    [anon_sym_COLON] = ACTIONS(78),
    [anon_sym_SEMI] = ACTIONS(78),
    [anon_sym_DOT] = ACTIONS(80),
    [anon_sym_POUND] = ACTIONS(78),
  },
  [7] = {
    [ts_builtin_sym_end] = ACTIONS(82),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(84),
    [anon_sym_let] = ACTIONS(84),
    [anon_sym_mut] = ACTIONS(84),
    [anon_sym_type] = ACTIONS(84),
    [anon_sym_if] = ACTIONS(84),
    [anon_sym_else] = ACTIONS(84),
    [anon_sym_match] = ACTIONS(84),
    [anon_sym_for] = ACTIONS(84),
    [anon_sym_par] = ACTIONS(84),
    [anon_sym_while] = ACTIONS(84),
    [anon_sym_loop] = ACTIONS(84),
    [anon_sym_break] = ACTIONS(84),
    [anon_sym_continue] = ACTIONS(84),
    [anon_sym_return] = ACTIONS(84),
    [anon_sym_spawn] = ACTIONS(84),
    [anon_sym_meta] = ACTIONS(84),
    [anon_sym_error] = ACTIONS(84),
    [anon_sym_share] = ACTIONS(84),
    [anon_sym_use] = ACTIONS(84),
    [anon_sym_struct] = ACTIONS(84),
    [anon_sym_enum] = ACTIONS(84),
    [anon_sym_test] = ACTIONS(84),
    [anon_sym_trait] = ACTIONS(84),
    [anon_sym_impl] = ACTIONS(84),
    [anon_sym_in] = ACTIONS(84),
    [anon_sym_true] = ACTIONS(84),
    [anon_sym_false] = ACTIONS(84),
    [sym_type_identifier] = ACTIONS(82),
    [sym_identifier] = ACTIONS(84),
    [sym_number] = ACTIONS(82),
    [anon_sym_DQUOTE] = ACTIONS(82),
    [anon_sym_BQUOTE] = ACTIONS(82),
    [anon_sym_r_DQUOTE] = ACTIONS(82),
    [sym_macro] = ACTIONS(82),
    [anon_sym_PIPE_GT] = ACTIONS(82),
    [anon_sym_EQ_GT] = ACTIONS(82),
    [anon_sym_DASH_GT] = ACTIONS(82),
    [anon_sym_EQ_EQ] = ACTIONS(82),
    [anon_sym_BANG_EQ] = ACTIONS(82),
    [anon_sym_LT_EQ] = ACTIONS(82),
    [anon_sym_GT_EQ] = ACTIONS(82),
    [anon_sym_AMP_AMP] = ACTIONS(82),
    [anon_sym_PIPE_PIPE] = ACTIONS(82),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(82),
    [anon_sym_DOT_DOT] = ACTIONS(84),
    [anon_sym_PLUS] = ACTIONS(82),
    [anon_sym_DASH] = ACTIONS(84),
    [anon_sym_STAR] = ACTIONS(82),
    [anon_sym_SLASH] = ACTIONS(84),
    [anon_sym_PERCENT] = ACTIONS(82),
    [anon_sym_LT] = ACTIONS(84),
    [anon_sym_GT] = ACTIONS(84),
    [anon_sym_EQ] = ACTIONS(84),
    [anon_sym_BANG] = ACTIONS(84),
    [anon_sym_QMARK] = ACTIONS(82),
    [anon_sym_LBRACE] = ACTIONS(82),
    [anon_sym_RBRACE] = ACTIONS(82),
    [anon_sym_LPAREN] = ACTIONS(82),
    [anon_sym_RPAREN] = ACTIONS(82),
    [anon_sym_LBRACK] = ACTIONS(82),
    [anon_sym_RBRACK] = ACTIONS(82),
    [anon_sym_COMMA] = ACTIONS(82),
    [anon_sym_COLON] = ACTIONS(82),
    [anon_sym_SEMI] = ACTIONS(82),
    [anon_sym_DOT] = ACTIONS(84),
    [anon_sym_POUND] = ACTIONS(82),
  },
  [8] = {
    [ts_builtin_sym_end] = ACTIONS(86),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(88),
    [anon_sym_let] = ACTIONS(88),
    [anon_sym_mut] = ACTIONS(88),
    [anon_sym_type] = ACTIONS(88),
    [anon_sym_if] = ACTIONS(88),
    [anon_sym_else] = ACTIONS(88),
    [anon_sym_match] = ACTIONS(88),
    [anon_sym_for] = ACTIONS(88),
    [anon_sym_par] = ACTIONS(88),
    [anon_sym_while] = ACTIONS(88),
    [anon_sym_loop] = ACTIONS(88),
    [anon_sym_break] = ACTIONS(88),
    [anon_sym_continue] = ACTIONS(88),
    [anon_sym_return] = ACTIONS(88),
    [anon_sym_spawn] = ACTIONS(88),
    [anon_sym_meta] = ACTIONS(88),
    [anon_sym_error] = ACTIONS(88),
    [anon_sym_share] = ACTIONS(88),
    [anon_sym_use] = ACTIONS(88),
    [anon_sym_struct] = ACTIONS(88),
    [anon_sym_enum] = ACTIONS(88),
    [anon_sym_test] = ACTIONS(88),
    [anon_sym_trait] = ACTIONS(88),
    [anon_sym_impl] = ACTIONS(88),
    [anon_sym_in] = ACTIONS(88),
    [anon_sym_true] = ACTIONS(88),
    [anon_sym_false] = ACTIONS(88),
    [sym_type_identifier] = ACTIONS(86),
    [sym_identifier] = ACTIONS(88),
    [sym_number] = ACTIONS(86),
    [anon_sym_DQUOTE] = ACTIONS(86),
    [anon_sym_BQUOTE] = ACTIONS(86),
    [anon_sym_r_DQUOTE] = ACTIONS(86),
    [sym_macro] = ACTIONS(86),
    [anon_sym_PIPE_GT] = ACTIONS(86),
    [anon_sym_EQ_GT] = ACTIONS(86),
    [anon_sym_DASH_GT] = ACTIONS(86),
    [anon_sym_EQ_EQ] = ACTIONS(86),
    [anon_sym_BANG_EQ] = ACTIONS(86),
    [anon_sym_LT_EQ] = ACTIONS(86),
    [anon_sym_GT_EQ] = ACTIONS(86),
    [anon_sym_AMP_AMP] = ACTIONS(86),
    [anon_sym_PIPE_PIPE] = ACTIONS(86),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(86),
    [anon_sym_DOT_DOT] = ACTIONS(88),
    [anon_sym_PLUS] = ACTIONS(86),
    [anon_sym_DASH] = ACTIONS(88),
    [anon_sym_STAR] = ACTIONS(86),
    [anon_sym_SLASH] = ACTIONS(88),
    [anon_sym_PERCENT] = ACTIONS(86),
    [anon_sym_LT] = ACTIONS(88),
    [anon_sym_GT] = ACTIONS(88),
    [anon_sym_EQ] = ACTIONS(88),
    [anon_sym_BANG] = ACTIONS(88),
    [anon_sym_QMARK] = ACTIONS(86),
    [anon_sym_LBRACE] = ACTIONS(86),
    [anon_sym_RBRACE] = ACTIONS(86),
    [anon_sym_LPAREN] = ACTIONS(86),
    [anon_sym_RPAREN] = ACTIONS(86),
    [anon_sym_LBRACK] = ACTIONS(86),
    [anon_sym_RBRACK] = ACTIONS(86),
    [anon_sym_COMMA] = ACTIONS(86),
    [anon_sym_COLON] = ACTIONS(86),
    [anon_sym_SEMI] = ACTIONS(86),
    [anon_sym_DOT] = ACTIONS(88),
    [anon_sym_POUND] = ACTIONS(86),
  },
  [9] = {
    [ts_builtin_sym_end] = ACTIONS(90),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(92),
    [anon_sym_let] = ACTIONS(92),
    [anon_sym_mut] = ACTIONS(92),
    [anon_sym_type] = ACTIONS(92),
    [anon_sym_if] = ACTIONS(92),
    [anon_sym_else] = ACTIONS(92),
    [anon_sym_match] = ACTIONS(92),
    [anon_sym_for] = ACTIONS(92),
    [anon_sym_par] = ACTIONS(92),
    [anon_sym_while] = ACTIONS(92),
    [anon_sym_loop] = ACTIONS(92),
    [anon_sym_break] = ACTIONS(92),
    [anon_sym_continue] = ACTIONS(92),
    [anon_sym_return] = ACTIONS(92),
    [anon_sym_spawn] = ACTIONS(92),
    [anon_sym_meta] = ACTIONS(92),
    [anon_sym_error] = ACTIONS(92),
    [anon_sym_share] = ACTIONS(92),
    [anon_sym_use] = ACTIONS(92),
    [anon_sym_struct] = ACTIONS(92),
    [anon_sym_enum] = ACTIONS(92),
    [anon_sym_test] = ACTIONS(92),
    [anon_sym_trait] = ACTIONS(92),
    [anon_sym_impl] = ACTIONS(92),
    [anon_sym_in] = ACTIONS(92),
    [anon_sym_true] = ACTIONS(92),
    [anon_sym_false] = ACTIONS(92),
    [sym_type_identifier] = ACTIONS(90),
    [sym_identifier] = ACTIONS(92),
    [sym_number] = ACTIONS(90),
    [anon_sym_DQUOTE] = ACTIONS(90),
    [anon_sym_BQUOTE] = ACTIONS(90),
    [anon_sym_r_DQUOTE] = ACTIONS(90),
    [sym_macro] = ACTIONS(90),
    [anon_sym_PIPE_GT] = ACTIONS(90),
    [anon_sym_EQ_GT] = ACTIONS(90),
    [anon_sym_DASH_GT] = ACTIONS(90),
    [anon_sym_EQ_EQ] = ACTIONS(90),
    [anon_sym_BANG_EQ] = ACTIONS(90),
    [anon_sym_LT_EQ] = ACTIONS(90),
    [anon_sym_GT_EQ] = ACTIONS(90),
    [anon_sym_AMP_AMP] = ACTIONS(90),
    [anon_sym_PIPE_PIPE] = ACTIONS(90),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(90),
    [anon_sym_DOT_DOT] = ACTIONS(92),
    [anon_sym_PLUS] = ACTIONS(90),
    [anon_sym_DASH] = ACTIONS(92),
    [anon_sym_STAR] = ACTIONS(90),
    [anon_sym_SLASH] = ACTIONS(92),
    [anon_sym_PERCENT] = ACTIONS(90),
    [anon_sym_LT] = ACTIONS(92),
    [anon_sym_GT] = ACTIONS(92),
    [anon_sym_EQ] = ACTIONS(92),
    [anon_sym_BANG] = ACTIONS(92),
    [anon_sym_QMARK] = ACTIONS(90),
    [anon_sym_LBRACE] = ACTIONS(90),
    [anon_sym_RBRACE] = ACTIONS(90),
    [anon_sym_LPAREN] = ACTIONS(90),
    [anon_sym_RPAREN] = ACTIONS(90),
    [anon_sym_LBRACK] = ACTIONS(90),
    [anon_sym_RBRACK] = ACTIONS(90),
    [anon_sym_COMMA] = ACTIONS(90),
    [anon_sym_COLON] = ACTIONS(90),
    [anon_sym_SEMI] = ACTIONS(90),
    [anon_sym_DOT] = ACTIONS(92),
    [anon_sym_POUND] = ACTIONS(90),
  },
  [10] = {
    [ts_builtin_sym_end] = ACTIONS(94),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(96),
    [anon_sym_let] = ACTIONS(96),
    [anon_sym_mut] = ACTIONS(96),
    [anon_sym_type] = ACTIONS(96),
    [anon_sym_if] = ACTIONS(96),
    [anon_sym_else] = ACTIONS(96),
    [anon_sym_match] = ACTIONS(96),
    [anon_sym_for] = ACTIONS(96),
    [anon_sym_par] = ACTIONS(96),
    [anon_sym_while] = ACTIONS(96),
    [anon_sym_loop] = ACTIONS(96),
    [anon_sym_break] = ACTIONS(96),
    [anon_sym_continue] = ACTIONS(96),
    [anon_sym_return] = ACTIONS(96),
    [anon_sym_spawn] = ACTIONS(96),
    [anon_sym_meta] = ACTIONS(96),
    [anon_sym_error] = ACTIONS(96),
    [anon_sym_share] = ACTIONS(96),
    [anon_sym_use] = ACTIONS(96),
    [anon_sym_struct] = ACTIONS(96),
    [anon_sym_enum] = ACTIONS(96),
    [anon_sym_test] = ACTIONS(96),
    [anon_sym_trait] = ACTIONS(96),
    [anon_sym_impl] = ACTIONS(96),
    [anon_sym_in] = ACTIONS(96),
    [anon_sym_true] = ACTIONS(96),
    [anon_sym_false] = ACTIONS(96),
    [sym_type_identifier] = ACTIONS(94),
    [sym_identifier] = ACTIONS(96),
    [sym_number] = ACTIONS(94),
    [anon_sym_DQUOTE] = ACTIONS(94),
    [anon_sym_BQUOTE] = ACTIONS(94),
    [anon_sym_r_DQUOTE] = ACTIONS(94),
    [sym_macro] = ACTIONS(94),
    [anon_sym_PIPE_GT] = ACTIONS(94),
    [anon_sym_EQ_GT] = ACTIONS(94),
    [anon_sym_DASH_GT] = ACTIONS(94),
    [anon_sym_EQ_EQ] = ACTIONS(94),
    [anon_sym_BANG_EQ] = ACTIONS(94),
    [anon_sym_LT_EQ] = ACTIONS(94),
    [anon_sym_GT_EQ] = ACTIONS(94),
    [anon_sym_AMP_AMP] = ACTIONS(94),
    [anon_sym_PIPE_PIPE] = ACTIONS(94),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(94),
    [anon_sym_DOT_DOT] = ACTIONS(96),
    [anon_sym_PLUS] = ACTIONS(94),
    [anon_sym_DASH] = ACTIONS(96),
    [anon_sym_STAR] = ACTIONS(94),
    [anon_sym_SLASH] = ACTIONS(96),
    [anon_sym_PERCENT] = ACTIONS(94),
    [anon_sym_LT] = ACTIONS(96),
    [anon_sym_GT] = ACTIONS(96),
    [anon_sym_EQ] = ACTIONS(96),
    [anon_sym_BANG] = ACTIONS(96),
    [anon_sym_QMARK] = ACTIONS(94),
    [anon_sym_LBRACE] = ACTIONS(94),
    [anon_sym_RBRACE] = ACTIONS(94),
    [anon_sym_LPAREN] = ACTIONS(94),
    [anon_sym_RPAREN] = ACTIONS(94),
    [anon_sym_LBRACK] = ACTIONS(94),
    [anon_sym_RBRACK] = ACTIONS(94),
    [anon_sym_COMMA] = ACTIONS(94),
    [anon_sym_COLON] = ACTIONS(94),
    [anon_sym_SEMI] = ACTIONS(94),
    [anon_sym_DOT] = ACTIONS(96),
    [anon_sym_POUND] = ACTIONS(94),
  },
  [11] = {
    [ts_builtin_sym_end] = ACTIONS(98),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(100),
    [anon_sym_let] = ACTIONS(100),
    [anon_sym_mut] = ACTIONS(100),
    [anon_sym_type] = ACTIONS(100),
    [anon_sym_if] = ACTIONS(100),
    [anon_sym_else] = ACTIONS(100),
    [anon_sym_match] = ACTIONS(100),
    [anon_sym_for] = ACTIONS(100),
    [anon_sym_par] = ACTIONS(100),
    [anon_sym_while] = ACTIONS(100),
    [anon_sym_loop] = ACTIONS(100),
    [anon_sym_break] = ACTIONS(100),
    [anon_sym_continue] = ACTIONS(100),
    [anon_sym_return] = ACTIONS(100),
    [anon_sym_spawn] = ACTIONS(100),
    [anon_sym_meta] = ACTIONS(100),
    [anon_sym_error] = ACTIONS(100),
    [anon_sym_share] = ACTIONS(100),
    [anon_sym_use] = ACTIONS(100),
    [anon_sym_struct] = ACTIONS(100),
    [anon_sym_enum] = ACTIONS(100),
    [anon_sym_test] = ACTIONS(100),
    [anon_sym_trait] = ACTIONS(100),
    [anon_sym_impl] = ACTIONS(100),
    [anon_sym_in] = ACTIONS(100),
    [anon_sym_true] = ACTIONS(100),
    [anon_sym_false] = ACTIONS(100),
    [sym_type_identifier] = ACTIONS(98),
    [sym_identifier] = ACTIONS(100),
    [sym_number] = ACTIONS(98),
    [anon_sym_DQUOTE] = ACTIONS(98),
    [anon_sym_BQUOTE] = ACTIONS(98),
    [anon_sym_r_DQUOTE] = ACTIONS(98),
    [sym_macro] = ACTIONS(98),
    [anon_sym_PIPE_GT] = ACTIONS(98),
    [anon_sym_EQ_GT] = ACTIONS(98),
    [anon_sym_DASH_GT] = ACTIONS(98),
    [anon_sym_EQ_EQ] = ACTIONS(98),
    [anon_sym_BANG_EQ] = ACTIONS(98),
    [anon_sym_LT_EQ] = ACTIONS(98),
    [anon_sym_GT_EQ] = ACTIONS(98),
    [anon_sym_AMP_AMP] = ACTIONS(98),
    [anon_sym_PIPE_PIPE] = ACTIONS(98),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(98),
    [anon_sym_DOT_DOT] = ACTIONS(100),
    [anon_sym_PLUS] = ACTIONS(98),
    [anon_sym_DASH] = ACTIONS(100),
    [anon_sym_STAR] = ACTIONS(98),
    [anon_sym_SLASH] = ACTIONS(100),
    [anon_sym_PERCENT] = ACTIONS(98),
    [anon_sym_LT] = ACTIONS(100),
    [anon_sym_GT] = ACTIONS(100),
    [anon_sym_EQ] = ACTIONS(100),
    [anon_sym_BANG] = ACTIONS(100),
    [anon_sym_QMARK] = ACTIONS(98),
    [anon_sym_LBRACE] = ACTIONS(98),
    [anon_sym_RBRACE] = ACTIONS(98),
    [anon_sym_LPAREN] = ACTIONS(98),
    [anon_sym_RPAREN] = ACTIONS(98),
    [anon_sym_LBRACK] = ACTIONS(98),
    [anon_sym_RBRACK] = ACTIONS(98),
    [anon_sym_COMMA] = ACTIONS(98),
    [anon_sym_COLON] = ACTIONS(98),
    [anon_sym_SEMI] = ACTIONS(98),
    [anon_sym_DOT] = ACTIONS(100),
    [anon_sym_POUND] = ACTIONS(98),
  },
  [12] = {
    [ts_builtin_sym_end] = ACTIONS(102),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(104),
    [anon_sym_let] = ACTIONS(104),
    [anon_sym_mut] = ACTIONS(104),
    [anon_sym_type] = ACTIONS(104),
    [anon_sym_if] = ACTIONS(104),
    [anon_sym_else] = ACTIONS(104),
    [anon_sym_match] = ACTIONS(104),
    [anon_sym_for] = ACTIONS(104),
    [anon_sym_par] = ACTIONS(104),
    [anon_sym_while] = ACTIONS(104),
    [anon_sym_loop] = ACTIONS(104),
    [anon_sym_break] = ACTIONS(104),
    [anon_sym_continue] = ACTIONS(104),
    [anon_sym_return] = ACTIONS(104),
    [anon_sym_spawn] = ACTIONS(104),
    [anon_sym_meta] = ACTIONS(104),
    [anon_sym_error] = ACTIONS(104),
    [anon_sym_share] = ACTIONS(104),
    [anon_sym_use] = ACTIONS(104),
    [anon_sym_struct] = ACTIONS(104),
    [anon_sym_enum] = ACTIONS(104),
    [anon_sym_test] = ACTIONS(104),
    [anon_sym_trait] = ACTIONS(104),
    [anon_sym_impl] = ACTIONS(104),
    [anon_sym_in] = ACTIONS(104),
    [anon_sym_true] = ACTIONS(104),
    [anon_sym_false] = ACTIONS(104),
    [sym_type_identifier] = ACTIONS(102),
    [sym_identifier] = ACTIONS(104),
    [sym_number] = ACTIONS(102),
    [anon_sym_DQUOTE] = ACTIONS(102),
    [anon_sym_BQUOTE] = ACTIONS(102),
    [anon_sym_r_DQUOTE] = ACTIONS(102),
    [sym_macro] = ACTIONS(102),
    [anon_sym_PIPE_GT] = ACTIONS(102),
    [anon_sym_EQ_GT] = ACTIONS(102),
    [anon_sym_DASH_GT] = ACTIONS(102),
    [anon_sym_EQ_EQ] = ACTIONS(102),
    [anon_sym_BANG_EQ] = ACTIONS(102),
    [anon_sym_LT_EQ] = ACTIONS(102),
    [anon_sym_GT_EQ] = ACTIONS(102),
    [anon_sym_AMP_AMP] = ACTIONS(102),
    [anon_sym_PIPE_PIPE] = ACTIONS(102),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(102),
    [anon_sym_DOT_DOT] = ACTIONS(104),
    [anon_sym_PLUS] = ACTIONS(102),
    [anon_sym_DASH] = ACTIONS(104),
    [anon_sym_STAR] = ACTIONS(102),
    [anon_sym_SLASH] = ACTIONS(104),
    [anon_sym_PERCENT] = ACTIONS(102),
    [anon_sym_LT] = ACTIONS(104),
    [anon_sym_GT] = ACTIONS(104),
    [anon_sym_EQ] = ACTIONS(104),
    [anon_sym_BANG] = ACTIONS(104),
    [anon_sym_QMARK] = ACTIONS(102),
    [anon_sym_LBRACE] = ACTIONS(102),
    [anon_sym_RBRACE] = ACTIONS(102),
    [anon_sym_LPAREN] = ACTIONS(102),
    [anon_sym_RPAREN] = ACTIONS(102),
    [anon_sym_LBRACK] = ACTIONS(102),
    [anon_sym_RBRACK] = ACTIONS(102),
    [anon_sym_COMMA] = ACTIONS(102),
    [anon_sym_COLON] = ACTIONS(102),
    [anon_sym_SEMI] = ACTIONS(102),
    [anon_sym_DOT] = ACTIONS(104),
    [anon_sym_POUND] = ACTIONS(102),
  },
  [13] = {
    [ts_builtin_sym_end] = ACTIONS(106),
    [sym_comment] = ACTIONS(3),
    [anon_sym_fn] = ACTIONS(108),
    [anon_sym_let] = ACTIONS(108),
    [anon_sym_mut] = ACTIONS(108),
    [anon_sym_type] = ACTIONS(108),
    [anon_sym_if] = ACTIONS(108),
    [anon_sym_else] = ACTIONS(108),
    [anon_sym_match] = ACTIONS(108),
    [anon_sym_for] = ACTIONS(108),
    [anon_sym_par] = ACTIONS(108),
    [anon_sym_while] = ACTIONS(108),
    [anon_sym_loop] = ACTIONS(108),
    [anon_sym_break] = ACTIONS(108),
    [anon_sym_continue] = ACTIONS(108),
    [anon_sym_return] = ACTIONS(108),
    [anon_sym_spawn] = ACTIONS(108),
    [anon_sym_meta] = ACTIONS(108),
    [anon_sym_error] = ACTIONS(108),
    [anon_sym_share] = ACTIONS(108),
    [anon_sym_use] = ACTIONS(108),
    [anon_sym_struct] = ACTIONS(108),
    [anon_sym_enum] = ACTIONS(108),
    [anon_sym_test] = ACTIONS(108),
    [anon_sym_trait] = ACTIONS(108),
    [anon_sym_impl] = ACTIONS(108),
    [anon_sym_in] = ACTIONS(108),
    [anon_sym_true] = ACTIONS(108),
    [anon_sym_false] = ACTIONS(108),
    [sym_type_identifier] = ACTIONS(106),
    [sym_identifier] = ACTIONS(108),
    [sym_number] = ACTIONS(106),
    [anon_sym_DQUOTE] = ACTIONS(106),
    [anon_sym_BQUOTE] = ACTIONS(106),
    [anon_sym_r_DQUOTE] = ACTIONS(106),
    [sym_macro] = ACTIONS(106),
    [anon_sym_PIPE_GT] = ACTIONS(106),
    [anon_sym_EQ_GT] = ACTIONS(106),
    [anon_sym_DASH_GT] = ACTIONS(106),
    [anon_sym_EQ_EQ] = ACTIONS(106),
    [anon_sym_BANG_EQ] = ACTIONS(106),
    [anon_sym_LT_EQ] = ACTIONS(106),
    [anon_sym_GT_EQ] = ACTIONS(106),
    [anon_sym_AMP_AMP] = ACTIONS(106),
    [anon_sym_PIPE_PIPE] = ACTIONS(106),
    [anon_sym_DOT_DOT_EQ] = ACTIONS(106),
    [anon_sym_DOT_DOT] = ACTIONS(108),
    [anon_sym_PLUS] = ACTIONS(106),
    [anon_sym_DASH] = ACTIONS(108),
    [anon_sym_STAR] = ACTIONS(106),
    [anon_sym_SLASH] = ACTIONS(108),
    [anon_sym_PERCENT] = ACTIONS(106),
    [anon_sym_LT] = ACTIONS(108),
    [anon_sym_GT] = ACTIONS(108),
    [anon_sym_EQ] = ACTIONS(108),
    [anon_sym_BANG] = ACTIONS(108),
    [anon_sym_QMARK] = ACTIONS(106),
    [anon_sym_LBRACE] = ACTIONS(106),
    [anon_sym_RBRACE] = ACTIONS(106),
    [anon_sym_LPAREN] = ACTIONS(106),
    [anon_sym_RPAREN] = ACTIONS(106),
    [anon_sym_LBRACK] = ACTIONS(106),
    [anon_sym_RBRACK] = ACTIONS(106),
    [anon_sym_COMMA] = ACTIONS(106),
    [anon_sym_COLON] = ACTIONS(106),
    [anon_sym_SEMI] = ACTIONS(106),
    [anon_sym_DOT] = ACTIONS(108),
    [anon_sym_POUND] = ACTIONS(106),
  },
};

static const uint16_t ts_small_parse_table[] = {
  [0] = 4,
    ACTIONS(110), 1,
      sym_comment,
    ACTIONS(112), 1,
      anon_sym_DQUOTE,
    STATE(18), 1,
      aux_sym_string_repeat1,
    ACTIONS(114), 2,
      aux_sym_string_token1,
      aux_sym_string_token2,
  [14] = 4,
    ACTIONS(110), 1,
      sym_comment,
    ACTIONS(118), 1,
      anon_sym_BQUOTE,
    STATE(17), 1,
      aux_sym_template_repeat1,
    ACTIONS(116), 2,
      aux_sym_string_token2,
      aux_sym_template_token1,
  [28] = 4,
    ACTIONS(110), 1,
      sym_comment,
    ACTIONS(120), 1,
      anon_sym_DQUOTE,
    STATE(19), 1,
      aux_sym_string_repeat1,
    ACTIONS(122), 2,
      aux_sym_string_token1,
      aux_sym_string_token2,
  [42] = 4,
    ACTIONS(110), 1,
      sym_comment,
    ACTIONS(126), 1,
      anon_sym_BQUOTE,
    STATE(21), 1,
      aux_sym_template_repeat1,
    ACTIONS(124), 2,
      aux_sym_string_token2,
      aux_sym_template_token1,
  [56] = 4,
    ACTIONS(110), 1,
      sym_comment,
    ACTIONS(128), 1,
      anon_sym_DQUOTE,
    STATE(20), 1,
      aux_sym_string_repeat1,
    ACTIONS(130), 2,
      aux_sym_string_token1,
      aux_sym_string_token2,
  [70] = 4,
    ACTIONS(110), 1,
      sym_comment,
    ACTIONS(132), 1,
      anon_sym_DQUOTE,
    STATE(20), 1,
      aux_sym_string_repeat1,
    ACTIONS(130), 2,
      aux_sym_string_token1,
      aux_sym_string_token2,
  [84] = 4,
    ACTIONS(110), 1,
      sym_comment,
    ACTIONS(134), 1,
      anon_sym_DQUOTE,
    STATE(20), 1,
      aux_sym_string_repeat1,
    ACTIONS(136), 2,
      aux_sym_string_token1,
      aux_sym_string_token2,
  [98] = 4,
    ACTIONS(110), 1,
      sym_comment,
    ACTIONS(142), 1,
      anon_sym_BQUOTE,
    STATE(21), 1,
      aux_sym_template_repeat1,
    ACTIONS(139), 2,
      aux_sym_string_token2,
      aux_sym_template_token1,
  [112] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(144), 1,
      ts_builtin_sym_end,
};

static const uint32_t ts_small_parse_table_map[] = {
  [SMALL_STATE(14)] = 0,
  [SMALL_STATE(15)] = 14,
  [SMALL_STATE(16)] = 28,
  [SMALL_STATE(17)] = 42,
  [SMALL_STATE(18)] = 56,
  [SMALL_STATE(19)] = 70,
  [SMALL_STATE(20)] = 84,
  [SMALL_STATE(21)] = 98,
  [SMALL_STATE(22)] = 112,
};

static const TSParseActionEntry ts_parse_actions[] = {
  [0] = {.entry = {.count = 0, .reusable = false}},
  [1] = {.entry = {.count = 1, .reusable = false}}, RECOVER(),
  [3] = {.entry = {.count = 1, .reusable = true}}, SHIFT_EXTRA(),
  [5] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 0, 0, 0),
  [7] = {.entry = {.count = 1, .reusable = false}}, SHIFT(4),
  [9] = {.entry = {.count = 1, .reusable = false}}, SHIFT(5),
  [11] = {.entry = {.count = 1, .reusable = true}}, SHIFT(2),
  [13] = {.entry = {.count = 1, .reusable = false}}, SHIFT(2),
  [15] = {.entry = {.count = 1, .reusable = true}}, SHIFT(16),
  [17] = {.entry = {.count = 1, .reusable = true}}, SHIFT(15),
  [19] = {.entry = {.count = 1, .reusable = true}}, SHIFT(14),
  [21] = {.entry = {.count = 1, .reusable = true}}, SHIFT(7),
  [23] = {.entry = {.count = 1, .reusable = false}}, SHIFT(7),
  [25] = {.entry = {.count = 1, .reusable = true}}, SHIFT(6),
  [27] = {.entry = {.count = 1, .reusable = false}}, SHIFT(6),
  [29] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 1, 0, 0),
  [31] = {.entry = {.count = 1, .reusable = true}}, SHIFT(3),
  [33] = {.entry = {.count = 1, .reusable = false}}, SHIFT(3),
  [35] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0),
  [37] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(4),
  [40] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(5),
  [43] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(3),
  [46] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(3),
  [49] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(16),
  [52] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(15),
  [55] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(14),
  [58] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(7),
  [61] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(7),
  [64] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(6),
  [67] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(6),
  [70] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_keyword, 1, 0, 0),
  [72] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_keyword, 1, 0, 0),
  [74] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_boolean, 1, 0, 0),
  [76] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_boolean, 1, 0, 0),
  [78] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_punctuation, 1, 0, 0),
  [80] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_punctuation, 1, 0, 0),
  [82] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_operator, 1, 0, 0),
  [84] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_operator, 1, 0, 0),
  [86] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string, 2, 0, 0),
  [88] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_string, 2, 0, 0),
  [90] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_template, 2, 0, 0),
  [92] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_template, 2, 0, 0),
  [94] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_raw_string, 2, 0, 0),
  [96] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_raw_string, 2, 0, 0),
  [98] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string, 3, 0, 0),
  [100] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_string, 3, 0, 0),
  [102] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_template, 3, 0, 0),
  [104] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_template, 3, 0, 0),
  [106] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_raw_string, 3, 0, 0),
  [108] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_raw_string, 3, 0, 0),
  [110] = {.entry = {.count = 1, .reusable = false}}, SHIFT_EXTRA(),
  [112] = {.entry = {.count = 1, .reusable = false}}, SHIFT(10),
  [114] = {.entry = {.count = 1, .reusable = false}}, SHIFT(18),
  [116] = {.entry = {.count = 1, .reusable = false}}, SHIFT(17),
  [118] = {.entry = {.count = 1, .reusable = false}}, SHIFT(9),
  [120] = {.entry = {.count = 1, .reusable = false}}, SHIFT(8),
  [122] = {.entry = {.count = 1, .reusable = false}}, SHIFT(19),
  [124] = {.entry = {.count = 1, .reusable = false}}, SHIFT(21),
  [126] = {.entry = {.count = 1, .reusable = false}}, SHIFT(12),
  [128] = {.entry = {.count = 1, .reusable = false}}, SHIFT(13),
  [130] = {.entry = {.count = 1, .reusable = false}}, SHIFT(20),
  [132] = {.entry = {.count = 1, .reusable = false}}, SHIFT(11),
  [134] = {.entry = {.count = 1, .reusable = false}}, REDUCE(aux_sym_string_repeat1, 2, 0, 0),
  [136] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_string_repeat1, 2, 0, 0), SHIFT_REPEAT(20),
  [139] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_template_repeat1, 2, 0, 0), SHIFT_REPEAT(21),
  [142] = {.entry = {.count = 1, .reusable = false}}, REDUCE(aux_sym_template_repeat1, 2, 0, 0),
  [144] = {.entry = {.count = 1, .reusable = true}},  ACCEPT_INPUT(),
};

#ifdef __cplusplus
extern "C" {
#endif
#ifdef TREE_SITTER_HIDE_SYMBOLS
#define TS_PUBLIC
#elif defined(_WIN32)
#define TS_PUBLIC __declspec(dllexport)
#else
#define TS_PUBLIC __attribute__((visibility("default")))
#endif

TS_PUBLIC const TSLanguage *tree_sitter_olang(void) {
  static const TSLanguage language = {
    .version = LANGUAGE_VERSION,
    .symbol_count = SYMBOL_COUNT,
    .alias_count = ALIAS_COUNT,
    .token_count = TOKEN_COUNT,
    .external_token_count = EXTERNAL_TOKEN_COUNT,
    .state_count = STATE_COUNT,
    .large_state_count = LARGE_STATE_COUNT,
    .production_id_count = PRODUCTION_ID_COUNT,
    .field_count = FIELD_COUNT,
    .max_alias_sequence_length = MAX_ALIAS_SEQUENCE_LENGTH,
    .parse_table = &ts_parse_table[0][0],
    .small_parse_table = ts_small_parse_table,
    .small_parse_table_map = ts_small_parse_table_map,
    .parse_actions = ts_parse_actions,
    .symbol_names = ts_symbol_names,
    .symbol_metadata = ts_symbol_metadata,
    .public_symbol_map = ts_symbol_map,
    .alias_map = ts_non_terminal_alias_map,
    .alias_sequences = &ts_alias_sequences[0][0],
    .lex_modes = ts_lex_modes,
    .lex_fn = ts_lex,
    .primary_state_ids = ts_primary_state_ids,
  };
  return &language;
}
#ifdef __cplusplus
}
#endif
