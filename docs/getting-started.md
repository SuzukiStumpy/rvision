# Getting started

A zero-to-running tour for developers building an application on top of
`rvision`. It narrates the same ground the `examples/` demos cover in code,
in the order a new dependent would actually meet it: a bare event loop, then
a retained view tree, then the application chrome (menus, a desktop of
windows, a modal dialog). For *why* each piece is shaped the way it is, see
the linked ADRs; for a full contract, see the linked module specs.

## Install

```sh
cargo add rvision
```

or add it directly:

```toml
[dependencies]
rvision = "2"
```

It needs the Rust 2024 edition and MSRV 1.85 (this repo's own
[`rust-toolchain.toml`](../rust-toolchain.toml) pins exactly that for
building `rvision` itself; your own project just needs a toolchain at least
that new). No other setup — `rvision` talks to the terminal through
`crossterm`, which needs nothing beyond a normal terminal emulator.

If you'd rather poke at the framework before wiring it into your own crate,
clone this repo and run the demos directly: `cargo run --example hello`. Each
section below names the demo it's narrating so you can run it alongside the
prose.

## 1. A bare event loop: `Program` + `Application`

The lowest layer has no view tree, no windows, no widgets — just a loop
that hands you a blank frame and events. This is [`examples/hello.rs`](../examples/hello.rs).

Implement [`app::Program`](../src/app.rs):

```rust
use rvision::app::{Application, Program};
use rvision::buffer::Buffer;
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult, KeyCode};

struct Demo { finished: bool }

impl Program for Demo {
    fn draw(&mut self, frame: &mut Buffer) {
        // Paint into `frame` — a Cell grid, diffed against the last frame
        // and flushed with minimal writes (ADR 0002).
    }

    fn handle_event(&mut self, event: &Event) -> EventResult {
        if let Event::Key(key) = event {
            if matches!(key.code, KeyCode::Esc) {
                self.finished = true;
                return EventResult::Consumed;
            }
        }
        EventResult::Ignored
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

fn main() -> std::io::Result<()> {
    let backend = CrosstermBackend::new()?;
    let mut app = Application::new(backend);
    let mut demo = Demo { finished: false };
    app.run(&mut demo)
    // `app` drops here, restoring the terminal — even if `run` panics
    // partway through (ADR 0001).
}
```

`Application::run` is: build a fresh `Buffer`, call `draw`, flush it, check
`is_finished`, wait up to a timeout for the next event (a timed-out wait is
delivered as `Event::Idle` — this is the idle/blink cadence), call
`handle_event`, check `is_finished` again, repeat. There is nothing here
specific to `rvision`'s eventual widgets; `Program` is unit-testable against
a scripted `TestBackend` with no real terminal at all.

## 2. From flags to a view tree: `View`, `Canvas`, `Group`

Real applications don't want one `draw` matching one `handle_event` by hand
— that doesn't compose. The next layer up is a retained-mode tree of
[`view::View`](../src/view.rs) trait objects: a parent owns its children,
each draws through its own offset, clipped [`canvas::Canvas`](../src/canvas.rs)
in coordinates relative to its own top-left (ADR 0003, ADR 0008), and
"things that happen" are posted **commands**, not mutated flags, bubbling up
through a [`view::Context`](../src/view.rs) rather than being invented ad hoc
per widget (ADR 0004). This is [`examples/hello2.rs`](../examples/hello2.rs) —
the same screen as `hello.rs`, rebuilt this way:

```rust
use rvision::app::{Application, Root};
use rvision::canvas::Canvas;
use rvision::command::CM_QUIT;
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult, KeyCode};
use rvision::geometry::Rect;
use rvision::view::{Context, View};

struct Desktop;

impl View for Desktop {
    fn bounds(&self) -> Rect { Rect::default() } // root has no owner to be positioned by

    fn draw(&self, canvas: &mut Canvas) {
        // Paint through `canvas` in local coordinates.
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        if let Event::Key(key) = event {
            if matches!(key.code, KeyCode::Esc) {
                ctx.post(CM_QUIT); // a command, not a flag
                return EventResult::Consumed;
            }
        }
        EventResult::Ignored
    }
}

fn main() -> std::io::Result<()> {
    let backend = CrosstermBackend::new()?;
    let mut app = Application::new(backend);
    let mut root = Root::new(Box::new(Desktop));
    app.run(&mut root)
    // `Root` is the bridge: it implements `Program` for the loop above,
    // and turns a posted `CM_QUIT` into `is_finished() == true`.
}
```

A container like [`view::Group`](../src/view.rs) composes several children
(each in its own sub-`Canvas`), handles focus and Tab/Shift-Tab between them,
and is itself a `View` — so trees nest without the root needing to know how
deep they go.

## 3. Application chrome: `Shell`, `Desktop`, and a modal dialog

`rvision` ships a ready-made screen shape — TurboVision's `TProgram` — so
you don't hand-roll layout for every app: [`app::Shell`](../src/app.rs) is a
menu bar across the top, a status line across the bottom, and a
[`widgets::Desktop`](../src/widgets/desktop.rs) of windows between (ADR 0009).
`Desktop` supports opening/closing/dragging/resizing/zooming several
[`widgets::Window`](../src/widgets/window.rs)s and switching which is active
(ADR 0016). See [`examples/chrome.rs`](../examples/chrome.rs) for a full menu
bar + desktop + status line screen, run through the same `Application`/`Root`
loop as step 2 — `Shell` is just another `View`.

Alongside tree-resident windows, a `Window` can instead be run **modally**,
blocking the rest of the tree until it's dismissed — the shape a Settings
dialog or a message box wants (ADR 0010). That's
[`app::Application::exec_view`](../src/app.rs), driving one `Window` over a
drawn (but non-interactive) background:

```rust
use rvision::command::CM_OK;
use rvision::view::Group;
use rvision::widgets::{Button, CheckBox, Window};

// `background` implements `Program` and only ever draws (exec_view never
// feeds it events); `theme` is a `Theme` in scope.
let controls: Vec<Box<dyn rvision::view::View>> = vec![
    Box::new(CheckBox::new(check_bounds, "Word wrap", theme).with_checked(true)),
    Box::new(Button::new(ok_bounds, "OK", CM_OK, theme).default(true)),
];
let mut dialog = Window::dialog(dialog_bounds, "Settings", theme, Box::new(Group::new(interior, controls)))
    .centered()
    .esc_cancels(true)
    .with_default(CM_OK);

let result = app.exec_view(&mut background, &mut dialog)?;
// `result` is whichever Command ended the dialog (CM_OK, a Cancel command
// you registered via `also_ends_on`, ...).
```

`Window::dialog` builds on the same `Window` chrome as a normal desktop
window (ADR 0016) — `Button`, `CheckBox`, `InputLine`, `RadioButtons`,
`GroupBox`, and `ListBox` all compose inside it the same way regardless of
whether the window ends up desktop-resident or modal. See
[`examples/dialogs.rs`](../examples/dialogs.rs) for a full sequence — a
message box, a Settings dialog exercising every control, a file-open dialog,
and a colour picker, one after another over a single backdrop.

## Where to go next

- [`docs/adr/`](adr/) — the *why* behind every decision above and everything
  built since, one numbered record per decision; start from
  [`docs/adr/README.md`](adr/README.md).
- [`docs/specs/`](specs/) — one contract per module: purpose, public
  interface, invariants, collaborators. `app.md`, `view.md`, `shell.md`,
  `desktop.md`, and `window.md` cover everything narrated above in full.
- [`docs/roadmap.md`](roadmap.md) — what's built, what's in flight, and
  what's still just an idea.
- [`CLAUDE.md`](../CLAUDE.md) — working conventions if you're contributing
  to `rvision` itself rather than only depending on it.
- `examples/` — every demo is runnable (`cargo run --example <name>`) and
  documents itself in its module-level doc comment; beyond the ones above,
  `combo_box`, `text_area`, `mdi`, `theme_picker`, `theme_editor`,
  `truecolour`, and `help_builder` each cover one further widget or system in
  isolation.
