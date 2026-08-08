(function_declaration
  name: (identifier) @name) @item

(trait_method
  name: (identifier) @name) @item

(type_declaration
  name: (type_identifier) @name) @item

(trait_declaration
  name: (type_identifier) @name) @item

(impl_declaration
  trait: (type_identifier) @name) @item

(test_declaration
  name: (string) @name) @item
