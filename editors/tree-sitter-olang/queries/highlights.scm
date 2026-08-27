; olang's visual identity. The deliberate choices, in order of what
; should catch the eye:
;
;   |>  =>  ->  #{        the marks that make olang code olang, as
;                         punctuation.special — visually distinct from
;                         ordinary operators in every shipped theme
;   `templates`           string.special, with ${...} as embedded code
;                         so interpolations read as live expressions
;   share                 attribute — a module's exports pop
;   fs.read_file(...)     the receiver as variable.special (the same
;                         voice themes give `self`), members as
;                         properties, the called member as a function
;   /// and //!           comment.doc, distinct from // commentary
;
; Generic tokens come first; specific captures later override them.

(identifier) @variable
(operator) @operator
(bracket) @punctuation.bracket
(delimiter) @punctuation.delimiter

(comment) @comment
(doc_comment) @comment.doc

(string) @string
(raw_string) @string
(escape) @string.escape
(char) @constant

(template) @string.special
(template_text) @string.special
(interpolation) @embedded

(number) @number
(boolean) @boolean
(type_identifier) @type
(keyword) @keyword
(share) @attribute
(macro_name) @function.macro

; Call sites: the function name reads as a function.
(call function: (identifier) @function)

; Dotted paths: receiver like `self`, members as properties — and the
; member anchored directly before the open paren is the call target.
(dotted module: (identifier) @variable.special)
(dotted (member) @property)
(dotted (member) @function . (open_call))
(dotted (open_call) @punctuation.bracket)

; The olang marks, last so nothing shadows them.
(pipe) @punctuation.special
(arrow) @punctuation.special
(hash_brace) @punctuation.special
