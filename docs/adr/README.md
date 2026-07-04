# Architecture Decision Records

Each record captures one significant decision: its context, the choice, and the
consequences. They are append-only history — supersede with a new ADR rather than
rewriting an old one.

Numbers 0001–0007 and 0015–0017/0020/0022 below are inherited, renumbered,
from `rvision`'s original home in the [`edit`](https://github.com/SuzukiStumpy/edit)
monorepo, where it lived alongside the editor before being extracted into this
repository. Decisions specific to the editor itself stayed behind there. ADR
0014 is also inherited, but predates the split and applied to the shared
workspace from the start, so it kept its original number unchanged.

| #    | Decision |
|------|----------|
| [0001](0001-terminal-backend-crossterm.md) | Use crossterm at the OS/terminal boundary |
| [0002](0002-render-seam-backend-double-buffer.md) | Backend/EventSource traits + double-buffer cell diff |
| [0003](0003-view-model-trait-objects-messages.md) | Retained-mode view tree: trait objects + message passing |
| [0004](0004-event-engine-three-phase.md) | Three-phase event dispatch, `EventResult`, modal `exec_view` |
| [0005](0005-colour-roles-truecolour-ready.md) | Semantic colour roles over a truecolour-ready type |
| [0006](0006-unicode-full-now.md) | Full Unicode now (width + segmentation data crates) |
| [0007](0007-mouse-architected-keyboard-first.md) | Architect for mouse, build keyboard-first |
| [0008](0008-view-coordinates-canvas.md) | Owner-relative view coordinates via a translating `Canvas` |
| [0009](0009-application-shell-menu-overlay.md) | `TProgram`-style application shell + drawn menu overlay |
| [0010](0010-modal-dialogs-and-focus-aware-controls.md) | Modal dialogs via `exec_view` + focus-aware controls (`set_focused`) |
| [0011](0011-drop-shadows-per-view-protocol.md) | Drop shadows are a per-view protocol (`View::drop_shadow`) |
| [0012](0012-paste-in-via-bracketed-paste.md) | Paste-in via bracketed paste (`Event::Paste`) |
| [0013](0013-help-format-and-model.md) | Help content: lightweight markup format + block topic model |
| [0014](0014-documentation-process.md) | Full documentation process (ADRs, roadmap, module specs, rustdoc, CLAUDE.md) |
| [0015](0015-scroll-chrome-per-view-protocol.md) | Scroll chrome is a per-view protocol (`scroll_metrics`/`set_scroll`) |
| [0016](0016-unify-window-dialog-dynamic-desktop.md) | Unify `Window` and `Dialog`; a capable, dynamic desktop |
| [0017](0017-resize-propagation-per-view-protocol.md) | Resize propagation is a per-view protocol (`View::set_bounds`) |
| [0018](0018-cascading-menu-submenus.md) | Cascading menus: a path stack, right-anchored, item-level gating |
| [0019](0019-context-menu-anchor-request.md) | Right-click context menus: a `Context` anchor request, Shell-owned overlay |
| [0020](0020-followable-help-links.md) | Followable help links: spans, dedicated cycle keys, a pending-activation poll |
| [0021](0021-window-scoped-context-help.md) | Context-sensitive help: window-scoped topics via `CM_HELP` |
| [0022](0022-release-process-and-versioning.md) | Release process: `release-please`, single-crate config, cut v1.0.0 |
| [0023](0023-truecolour-capability-detection.md) | Truecolour capability detection: `ColorProfile::detect`, env vars, no new crate |
| [0024](0024-layered-resource-loading.md) | Layered resource loading: shared path resolution, per-kind format & merge |
| [0025](0025-theme-file-format-and-merge.md) | Theme file format: dotted `key = value`, infallible merge via `Theme::with` |
| [0026](0026-theme-editor-desktop-composition.md) | Theme editor: `Desktop`-hosted composition via bubbled commands and a read/write handle |
| [0027](0027-mouse-capture-for-drag-interactions.md) | Generic mouse capture (`Context`/`Desktop`) for continuous drag interactions, e.g. scroll-bar thumbs |

New decision? Copy [`0000-template.md`](0000-template.md).
