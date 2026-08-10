# Publishing the VS Code extension

Two paths. The Marketplace one needs a (free) publisher account; the
.vsix one needs nothing.

## Build the .vsix (no account required)

```bash
cd editors/vscode
npm ci
npx vsce package          # runs the esbuild bundle step, emits olang-<version>.vsix
```

`make dist` at the repository root does the same as part of staging
every release artifact into `dist/out/`.

Anyone can install the file directly — no Marketplace involved:

- UI: Extensions view → `…` menu → **Install from VSIX…**
- CLI: `code --install-extension olang-0.1.0.vsix`

Attaching the .vsix to the GitHub Release (or sharing the file any
other way) is a complete distribution channel by itself.

## Publish to the Marketplace (account required — one-time setup)

The Marketplace rides on Azure DevOps; the account and publisher are
free.

1. **Create the publisher** (once). Sign in at
   https://marketplace.visualstudio.com/manage with a Microsoft
   account, and create a publisher with ID `ooyeku` — the ID must match
   the `"publisher"` field in `package.json`, and it cannot be renamed
   later.
2. **Create a Personal Access Token** (expires; you will redo this). At
   https://dev.azure.com → User settings → Personal Access Tokens →
   New Token: Organization **All accessible organizations**, scope
   **Marketplace → Manage**.
3. **Log in and publish**:

   ```bash
   cd editors/vscode
   npx vsce login ooyeku       # paste the PAT
   npx vsce publish            # packages and uploads the current version
   ```

   `vsce publish` refuses to overwrite an existing version — bump
   `"version"` in `package.json` first (it is versioned independently
   of olang itself; it only changes when the extension does).

4. **Verify** at https://marketplace.visualstudio.com/items?itemName=ooyeku.olang
   (listing appears within a few minutes after validation).

## What the package contains

`vsce package` runs `vscode:prepublish` → esbuild, which bundles
`extension.js` + `vscode-languageclient` into `dist/extension.js`; the
.vsix ships that bundle plus the grammar, `language-configuration.json`,
icon, README, and LICENSE — no `node_modules`. `.vscodeignore` is the
manifest of what stays out.

Keep `"activationEvents": ["onLanguage:olang"]` in `package.json` —
activation breaks without it.
