(function_declaration
  body: (_) @function.inside) @function.around

(trait_method) @function.around

[
  (type_declaration)
  (trait_declaration)
  (impl_declaration)
] @class.around

(comment)+ @comment.around
