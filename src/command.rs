//! UI commands and their enabled/disabled state (ADR 0003, 0004).
//!
//! A [`Command`] is the integer id of an action — TurboVision's `cmXxx`. A view
//! posts one when, say, its button is pressed; the command then bubbles up the
//! owner chain to whoever handles it (`view::Group`). This module owns only the
//! *vocabulary* and the [`CommandSet`] that says which commands are live right
//! now, so a disabled command never fires and a control can grey itself.

use std::collections::BTreeSet;

pub use crate::event::Command;

/// Quit the application (TurboVision's `cmQuit`).
pub const CM_QUIT: Command = Command(1);
/// Accept a dialog (TurboVision's `cmOK`).
pub const CM_OK: Command = Command(2);
/// Dismiss a dialog without accepting (TurboVision's `cmCancel`).
pub const CM_CANCEL: Command = Command(3);
/// Affirmative answer to a confirmation (TurboVision's `cmYes`).
pub const CM_YES: Command = Command(4);
/// Negative answer to a confirmation (TurboVision's `cmNo`).
pub const CM_NO: Command = Command(5);
/// Open the help viewer (TurboVision's `cmHelp`). The framework standardises the
/// id so the `Shell` and a bespoke app driver (e.g. the editor's) can share it.
pub const CM_HELP: Command = Command(6);
/// Close the active window (TurboVision's `cmClose`). `Window` posts this from
/// its own close-glyph click; `Desktop` is what acts on it (ADR 0016).
pub const CM_CLOSE: Command = Command(7);
/// Maximise or restore the active window (TurboVision's `cmZoom`). `Window`
/// posts this from its own zoom-glyph click; `Desktop` acts on it (ADR 0016).
pub const CM_ZOOM: Command = Command(8);
/// Cycle the active window forward (TurboVision's `cmNext`), acted on by
/// `Desktop` (ADR 0016).
pub const CM_NEXT: Command = Command(9);
/// Cycle the active window backward, acted on by `Desktop` (ADR 0016).
pub const CM_PREV: Command = Command(10);
/// Request that the currently-selected foreground colour be edited via a
/// nested `ColorPicker` (`widgets::ThemeEditor`, ADR 0026). A `View` hosted
/// on a `Desktop` can't open another window itself (ADR 0003: commands only
/// bubble up); this is the standard signal for "please open one for me" that
/// a hosting `Program` intercepts, mirroring how it already intercepts its
/// own app-specific commands (see `examples/mdi.rs`'s `Mdi::dispatch`).
pub const CM_EDIT_FG: Command = Command(11);
/// As `CM_EDIT_FG`, for the background colour.
pub const CM_EDIT_BG: Command = Command(12);

/// The first command id reserved for the **application**.
///
/// The command id space is open and partitioned (ADR 0003): ids below this are
/// the framework's own standard commands — the ones rvision's widgets emit and
/// handle — and an application numbers its commands from here up
/// (`Command(CM_USER + 1)`, …), so the two namespaces never collide. The
/// framework routes application commands opaquely; it never needs to name them.
/// (TurboVision's `cmUser`.)
pub const CM_USER: u16 = 100;

/// Which commands are currently enabled.
///
/// Stored as the set of *disabled* ids, so a default set enables everything and
/// the whole `u16` command space is usable without pre-populating it. Disable the
/// exceptions; query with [`is_enabled`](Self::is_enabled).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandSet {
    disabled: BTreeSet<u16>,
}

impl CommandSet {
    /// Creates a set with every command enabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables `command` (idempotent — a no-op if it was never disabled).
    pub fn enable(&mut self, command: Command) {
        self.disabled.remove(&command.0);
    }

    /// Disables `command` (idempotent).
    pub fn disable(&mut self, command: Command) {
        self.disabled.insert(command.0);
    }

    /// Returns whether `command` is currently enabled.
    pub fn is_enabled(&self, command: Command) -> bool {
        !self.disabled.contains(&command.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tracer bullet: everything is enabled until explicitly disabled.
    #[test]
    fn new_set_enables_everything() {
        let set = CommandSet::new();
        assert!(set.is_enabled(CM_OK));
        assert!(set.is_enabled(Command(9999)));
    }

    #[test]
    fn disable_then_enable_round_trips() {
        let mut set = CommandSet::new();
        set.disable(CM_OK);
        assert!(!set.is_enabled(CM_OK));
        // Disabling is idempotent.
        set.disable(CM_OK);
        assert!(!set.is_enabled(CM_OK));

        set.enable(CM_OK);
        assert!(set.is_enabled(CM_OK));
        // Enabling an already-enabled command is a no-op, not a panic.
        set.enable(CM_OK);
        assert!(set.is_enabled(CM_OK));
    }

    #[test]
    fn commands_are_independent() {
        let mut set = CommandSet::new();
        set.disable(CM_OK);
        // Disabling one leaves the others alone.
        assert!(!set.is_enabled(CM_OK));
        assert!(set.is_enabled(CM_CANCEL));
        assert!(set.is_enabled(CM_QUIT));
    }

    #[test]
    fn standard_ids_are_distinct_and_non_zero() {
        let ids = [
            CM_QUIT, CM_OK, CM_CANCEL, CM_YES, CM_NO, CM_HELP, CM_CLOSE, CM_ZOOM, CM_NEXT, CM_PREV,
            CM_EDIT_FG, CM_EDIT_BG,
        ];
        for id in ids {
            assert_ne!(id.0, 0, "id 0 is reserved for 'no command'");
        }
        // No two standard commands collide.
        let mut seen = BTreeSet::new();
        for id in ids {
            assert!(
                seen.insert(id.0),
                "{id:?} collides with another standard id"
            );
        }
    }

    #[test]
    fn standard_commands_sit_below_the_application_range() {
        // The framework's own commands live below CM_USER; apps number from there
        // up, so the two namespaces never collide (ADR 0003).
        for id in [
            CM_QUIT, CM_OK, CM_CANCEL, CM_YES, CM_NO, CM_HELP, CM_CLOSE, CM_ZOOM, CM_NEXT, CM_PREV,
            CM_EDIT_FG, CM_EDIT_BG,
        ] {
            assert!(id.0 < CM_USER, "{id:?} must be a framework-reserved id");
        }
    }
}
