# tree-sitter-olang

Tree-sitter grammar for the Olang programming language. The grammar powers
the bundled Zed extension and is intentionally kept beside Olang's canonical
Pest grammar so editor support can evolve with the language.

Generate and test the parser from this directory:

```sh
npx tree-sitter-cli generate
npx tree-sitter-cli parse ../../examples/webserver/main.ol
```
