# Olang for Zed

Native Olang syntax support for Zed, backed by the bundled Tree-sitter
grammar. It recognizes both `.ol` and `.olang` files and provides syntax
highlighting, indentation, bracket matching, an outline, and text objects.

## Install during development

1. Open Zed's command palette.
2. Run **zed: install dev extension**.
3. Select this `editors/zed-olang` directory.

Zed recompiles the Tree-sitter grammar when the dev extension is reloaded.
