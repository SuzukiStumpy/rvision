# Architecture Decision Records

Each record captures one significant decision: its context, the choice, and the
consequences. They are append-only history — supersede with a new ADR rather than
rewriting an old one.

Numbers 0001–0007 and 0015–0017/0020/0022/0023 below are inherited, renumbered,
from `rvision`'s original home in the [`edit`](https://github.com/SuzukiStumpy/edit)
monorepo, where it lived alongside the editor before being extracted into this
repository. Decisions specific to the editor itself stayed behind there.

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

New decision? Copy [`0000-template.md`](0000-template.md).
