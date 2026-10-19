# accesskit_macos 0.27.1, patched

AccessKit's macOS adapter (https://github.com/AccessKit/accesskit,
MIT OR Apache-2.0, The AccessKit contributors), vendored from the
crates.io release 0.27.1 and used through `[patch.crates-io]` in
olang's Cargo.toml.

Changes, meant to go upstream (the first is the custom actions; the
rest are olang's accessibility depth, ROADMAP item 28):

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

Accessibility depth (`src/node.rs`, `src/event.rs`):

- `Role::Heading` is `AXHeading` (it was `Heading`, a role VoiceOver
  does not know); `Role::ScrollView` is `AXScrollArea` (it was unknown).
- Tables and trees: `accessibilityRows` answers a table's (or a tree's)
  rows whether or not they are chosen, found through the groups between;
  `accessibilityRowCount` and `accessibilityColumnCount` (the node's
  counts, else its rows), a row's `accessibilityIndex` (its row index,
  else its place in its set), a cell's `accessibilityRowIndexRange` and
  `accessibilityColumnIndexRange`, a table's
  `accessibilityColumnHeaderUIElements` and
  `accessibilityCellForColumn:row:`; a tree item's
  `accessibilityDisclosureLevel`, `isAccessibilityDisclosed` and
  `setAccessibilityDisclosed:` (`Action::Expand`/`Collapse`), and
  `isAccessibilityExpanded`/`setAccessibilityExpanded:` for anything
  with `expanded`.
- VoiceOver's rotor: `accessibilityParameterizedAttributeNames` adds
  `AXUIElementsForSearchPredicate` and
  `AXUIElementCountForSearchPredicate`, answered in
  `accessibilityAttributeValue:forParameter:` (WebKit's and Chromium's
  keys: headings by level, links, landmarks, controls, buttons, check
  boxes, radio groups, text fields, tables, lists, static text, graphics,
  focusable, live regions, the same or another type; `AXStartElement`,
  `AXDirection`, `AXResultsLimit`, `AXSearchText`,
  `AXImmediateDescendantsOnly`); other parameterized attributes go to
  the superclass.
- Page scrolls: a node that supports `Action::ScrollUp`/`Down`/`Left`/
  `Right` lists `AXScrollUpByPage` (and the others) in
  `accessibilityActionNames` and performs it as that action.
- Live regions (`src/event.rs`): a live node's value that changes under
  the same label is announced too ("label, value"), unless the node has
  the focus.

`tests/gui_macos_a11y_depth_test.rs` (olang) reads all of these from a
real window.
