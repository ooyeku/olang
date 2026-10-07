# accesskit_macos 0.27.1, patched

AccessKit's macOS adapter (https://github.com/AccessKit/accesskit,
MIT OR Apache-2.0, The AccessKit contributors), vendored from the
crates.io release 0.27.1 and used through `[patch.crates-io]` in
olang's Cargo.toml.

One change, meant to go upstream:

- `src/node.rs`: `PlatformNode` answers `accessibilityCustomActions`
  with one `NSAccessibilityCustomAction` per `Node::custom_actions`
  entry (the action's `description` as its name). The action's handler
  holds the adapter's context weakly and sends
  `ActionRequest { action: Action::CustomAction, data:
  Some(ActionData::CustomAction(id)), .. }` while the node still offers
  that id. `isAccessibilitySelectorAllowed:` allows the selector when
  the node supports `Action::CustomAction` and has custom actions.
- `Cargo.toml`: `block2` (the handler is a block) and objc2-app-kit's
  `block2` and `NSAccessibilityCustomAction` features.

Without it VoiceOver's Actions rotor (VO-Command-Space) is empty for a
node with custom actions. Remove the vendored copy and the patch once a
released accesskit_macos publishes custom actions.

`custom-actions.patch` is the `src/node.rs` change against the release,
with AccessKit's repository paths, for a pull request (its Cargo.toml
also needs `block2` and the two objc2-app-kit features above).
