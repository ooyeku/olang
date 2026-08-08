(comment) @comment

[
  (string)
  (raw_string)
  (template_string)
] @string

(character) @string.escape
(integer) @number
(float) @number
(boolean) @boolean

[
  "fn"
  "let"
  "mut"
  "type"
  "struct"
  "enum"
  "trait"
  "impl"
  "for"
  "if"
  "else"
  "match"
  "while"
  "loop"
  "break"
  "return"
  "async"
  "await"
  "spawn"
  "try"
  "catch"
  "error"
  "share"
  "use"
  "test"
] @keyword

(continue_expression) @keyword

[
  (builtin_type)
  (type_identifier)
] @type

(type_declaration name: (type_identifier) @type)
(trait_declaration name: (type_identifier) @type)
(error_declaration name: (type_identifier) @type)
(struct_literal type: (identifier) @type)
(enum_pattern variant: (identifier) @constructor)
(struct_pattern type: (identifier) @type)

(function_declaration name: (identifier) @function)
(trait_method name: (identifier) @function)
(call_expression function: (identifier) @function.call)
(call_expression function: (member_expression property: (identifier) @function.call))

(parameter name: (identifier) @variable.parameter)
(named_argument name: (identifier) @variable.parameter)
(field_declaration name: (identifier) @property)
(field_initializer name: (identifier) @property)
(member_expression property: (identifier) @property)
(map_entry key: (string) @property)

[
  "="
  "=>"
  "->"
  "|>"
  "|"
  "&"
  "^"
  "&&"
  "||"
  "=="
  "!="
  "<"
  "<="
  ">"
  ">="
  "+"
  "-"
  "*"
  "/"
  "%"
  "!"
  "?"
  ".."
  "..="
] @operator

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
  "#{"
] @punctuation.bracket

[
  ","
  "."
  ":"
  ";"
] @punctuation.delimiter
