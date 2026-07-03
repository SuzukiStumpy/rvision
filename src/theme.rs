//! Semantic styling roles and the theme that resolves them (ADR 0005).
//!
//! Widgets ask for a [`Role`] — what a piece of UI *is* — rather than naming
//! colours. A [`Theme`] maps every role to a concrete [`Style`], so swapping the
//! theme re-skins the entire interface. One default 16-colour CGA theme ships
//! here; alternative (dark, monochrome) themes can be added later without
//! touching widget code.

use crate::color::{Attributes, Color, Color16, Style};

/// A semantic role describing what a piece of UI is, independent of its colours.
///
/// `Role` is used to index a [`Theme`]'s style table via `role as usize`, so the
/// variants and [`Role::ALL`] must stay in the same (discriminant) order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// The desktop backdrop behind all windows.
    DesktopBackground,
    /// A window's border/frame.
    WindowFrame,
    /// The active window's title text (bright, to stand out).
    WindowTitle,
    /// An inactive window's title text (dimmer than the active one).
    WindowTitleInactive,
    /// The top menu bar background.
    MenuBar,
    /// The currently highlighted menu item.
    MenuSelected,
    /// A disabled (greyed) menu item.
    MenuDisabled,
    /// The highlighted accelerator letter within a menu title or item label
    /// (TurboVision's red hot-key character).
    MenuHotkey,
    /// The bottom status bar background.
    StatusBar,
    /// A highlighted shortcut key in the status bar.
    StatusKey,
    /// A button at rest.
    ButtonNormal,
    /// A focused button.
    ButtonFocused,
    /// Normal text in an editor window.
    EditorText,
    /// Selected (highlighted) text.
    Selection,
    /// A dialog box's background, frame, and label text (the classic grey dialog).
    DialogBackground,
    /// An editable input field (an [`InputLine`](crate::widgets::InputLine), a
    /// list-box interior).
    Input,
    /// The drop shadow cast by a window or dialog over what lies behind it.
    Shadow,
    /// A followable `{label|target}` link at rest in a
    /// [`HelpPane`](crate::widgets::HelpPane) (ADR 0020). The *current,
    /// keyboard-focused* link reuses [`Role::Selection`] instead, mirroring
    /// how [`ListBox`](crate::widgets::ListBox) highlights its selected row.
    HelpLink,
    /// A `ListBox`'s selected row when the list itself isn't focused, for a
    /// list opted into always showing its current item (ADR 0020 addendum —
    /// e.g. `HelpWindow`'s topic list, so "what topic is this?" stays
    /// answerable while the page pane holds focus). A dimmer relative of
    /// [`Role::Selection`], mirroring [`Role::WindowTitleInactive`]'s
    /// receded-but-still-legible relationship to [`Role::WindowTitle`].
    SelectionInactive,
}

impl Role {
    /// Every role, in discriminant order (so `ALL[i] as usize == i`).
    pub const ALL: [Role; 19] = [
        Role::DesktopBackground,
        Role::WindowFrame,
        Role::WindowTitle,
        Role::WindowTitleInactive,
        Role::MenuBar,
        Role::MenuSelected,
        Role::MenuDisabled,
        Role::MenuHotkey,
        Role::StatusBar,
        Role::StatusKey,
        Role::ButtonNormal,
        Role::ButtonFocused,
        Role::EditorText,
        Role::Selection,
        Role::DialogBackground,
        Role::Input,
        Role::Shadow,
        Role::HelpLink,
        Role::SelectionInactive,
    ];

    /// The number of roles.
    pub const COUNT: usize = Self::ALL.len();
}

/// Resolves [`Role`]s to concrete [`Style`]s. Clone and [`Theme::with`] to
/// derive a variant; swap the whole thing to re-skin the UI.
#[derive(Debug, Clone)]
pub struct Theme {
    styles: [Style; Role::COUNT],
}

impl Theme {
    /// Returns the style associated with `role`.
    pub fn style(&self, role: Role) -> Style {
        self.styles[role as usize]
    }

    /// Returns the theme with `role`'s style replaced, leaving all other roles
    /// untouched.
    pub fn with(mut self, role: Role, style: Style) -> Self {
        self.styles[role as usize] = style;
        self
    }

    /// Applies dotted `role.field = value` overrides from a theme-file layer
    /// (ADR 0025) on top of `self`. Each override reads the role's *current*
    /// style first and replaces only the one field (`fg`/`bg`/`attrs`) it
    /// names, so a line that sets only `editor_text.fg` leaves
    /// `editor_text`'s other fields exactly as `self` had them — layering
    /// several files means calling this once per layer in order
    /// (`Theme::default().merge(app).merge(user)`).
    ///
    /// Infallible, matching [`HelpContents::parse`](crate::help::HelpContents::parse)'s
    /// precedent (ADR 0013): a line that doesn't split on `.`/`=`, names an
    /// unrecognised role or field, or has an unparseable value is skipped —
    /// that one override doesn't apply, nothing else is affected.
    pub fn merge(mut self, text: &str) -> Self {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            let Some((role_key, field)) = key.trim().split_once('.') else {
                continue;
            };
            let Some(role) = role_from_key(role_key) else {
                continue;
            };
            let value = value.trim();
            let mut style = self.style(role);
            match field {
                "fg" => match parse_color(value) {
                    Some(c) => style.fg = c,
                    None => continue,
                },
                "bg" => match parse_color(value) {
                    Some(c) => style.bg = c,
                    None => continue,
                },
                "attrs" => match parse_attrs(value) {
                    Some(a) => style.attrs = a,
                    None => continue,
                },
                _ => continue,
            }
            self = self.with(role, style);
        }
        self
    }
}

/// The `snake_case` theme-file key for each [`Role`] (ADR 0025). A
/// hand-written table rather than a runtime `PascalCase`→`snake_case`
/// conversion: `Role`'s 19 variants are fixed and small enough that a table
/// is both cheaper and more obviously correct than a general algorithm.
fn role_from_key(key: &str) -> Option<Role> {
    match key {
        "desktop_background" => Some(Role::DesktopBackground),
        "window_frame" => Some(Role::WindowFrame),
        "window_title" => Some(Role::WindowTitle),
        "window_title_inactive" => Some(Role::WindowTitleInactive),
        "menu_bar" => Some(Role::MenuBar),
        "menu_selected" => Some(Role::MenuSelected),
        "menu_disabled" => Some(Role::MenuDisabled),
        "menu_hotkey" => Some(Role::MenuHotkey),
        "status_bar" => Some(Role::StatusBar),
        "status_key" => Some(Role::StatusKey),
        "button_normal" => Some(Role::ButtonNormal),
        "button_focused" => Some(Role::ButtonFocused),
        "editor_text" => Some(Role::EditorText),
        "selection" => Some(Role::Selection),
        "dialog_background" => Some(Role::DialogBackground),
        "input" => Some(Role::Input),
        "shadow" => Some(Role::Shadow),
        "help_link" => Some(Role::HelpLink),
        "selection_inactive" => Some(Role::SelectionInactive),
        _ => None,
    }
}

/// The `snake_case` theme-file key for each [`Color16`] (ADR 0025), the same
/// hand-written-table approach as [`role_from_key`].
fn color16_from_key(key: &str) -> Option<Color16> {
    match key {
        "black" => Some(Color16::Black),
        "blue" => Some(Color16::Blue),
        "green" => Some(Color16::Green),
        "cyan" => Some(Color16::Cyan),
        "red" => Some(Color16::Red),
        "magenta" => Some(Color16::Magenta),
        "brown" => Some(Color16::Brown),
        "light_gray" => Some(Color16::LightGray),
        "dark_gray" => Some(Color16::DarkGray),
        "light_blue" => Some(Color16::LightBlue),
        "light_green" => Some(Color16::LightGreen),
        "light_cyan" => Some(Color16::LightCyan),
        "light_red" => Some(Color16::LightRed),
        "light_magenta" => Some(Color16::LightMagenta),
        "yellow" => Some(Color16::Yellow),
        "white" => Some(Color16::White),
        _ => None,
    }
}

/// Parses a theme-file colour value: `default`, a [`Color16`] `snake_case`
/// name, or `rgb(r, g, b)` with decimal `u8` components (ADR 0025).
fn parse_color(value: &str) -> Option<Color> {
    if value == "default" {
        return Some(Color::Default);
    }
    if let Some(inner) = value.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let mut parts = inner.split(',').map(|p| p.trim());
        let r = parts.next()?.parse().ok()?;
        let g = parts.next()?.parse().ok()?;
        let b = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        return Some(Color::Rgb(r, g, b));
    }
    color16_from_key(value).map(Color::Named)
}

/// Parses a theme-file `attrs` value: `none`, or a `|`-combined list of
/// [`Attributes`] names (ADR 0025). Any unrecognised token invalidates the
/// whole value — an attrs override is all-or-nothing per line.
fn parse_attrs(value: &str) -> Option<Attributes> {
    if value == "none" {
        return Some(Attributes::NONE);
    }
    let mut attrs = Attributes::NONE;
    for token in value.split('|') {
        attrs = attrs.union(attr_from_key(token.trim())?);
    }
    Some(attrs)
}

/// The theme-file token for each [`Attributes`] flag (ADR 0025).
fn attr_from_key(key: &str) -> Option<Attributes> {
    match key {
        "bold" => Some(Attributes::BOLD),
        "dim" => Some(Attributes::DIM),
        "italic" => Some(Attributes::ITALIC),
        "underline" => Some(Attributes::UNDERLINE),
        "reverse" => Some(Attributes::REVERSE),
        "blink" => Some(Attributes::BLINK),
        _ => None,
    }
}

impl Default for Theme {
    /// The default 16-colour CGA theme. Colour choices are provisional and may be
    /// tuned as widgets are built.
    fn default() -> Self {
        let cga = |fg: Color16, bg: Color16| Style::new().fg(Color::Named(fg)).bg(Color::Named(bg));
        let mut styles = [Style::new(); Role::COUNT];
        styles[Role::DesktopBackground as usize] = cga(Color16::LightGray, Color16::Blue);
        styles[Role::WindowFrame as usize] = cga(Color16::White, Color16::Blue);
        // The active title pops (bright white, bold); inactive ones recede to the
        // dimmer frame grey so the focused window reads at a glance.
        styles[Role::WindowTitle as usize] =
            cga(Color16::White, Color16::Blue).attrs(Attributes::BOLD);
        styles[Role::WindowTitleInactive as usize] = cga(Color16::LightGray, Color16::Blue);
        styles[Role::MenuBar as usize] = cga(Color16::Black, Color16::LightGray);
        styles[Role::MenuSelected as usize] = cga(Color16::Black, Color16::Green);
        styles[Role::MenuDisabled as usize] = cga(Color16::DarkGray, Color16::LightGray);
        styles[Role::MenuHotkey as usize] = cga(Color16::Red, Color16::LightGray);
        styles[Role::StatusBar as usize] = cga(Color16::Black, Color16::LightGray);
        styles[Role::StatusKey as usize] = cga(Color16::Red, Color16::LightGray);
        styles[Role::ButtonNormal as usize] = cga(Color16::Black, Color16::Green);
        styles[Role::ButtonFocused as usize] = cga(Color16::White, Color16::Green);
        styles[Role::EditorText as usize] = cga(Color16::White, Color16::Blue);
        styles[Role::Selection as usize] = cga(Color16::Black, Color16::Cyan);
        styles[Role::DialogBackground as usize] = cga(Color16::Black, Color16::LightGray);
        styles[Role::Input as usize] = cga(Color16::Black, Color16::White);
        // The classic TurboVision shadow: whatever shows through is repainted
        // dark-gray on black, so it reads as "in shadow" without hiding the glyph.
        styles[Role::Shadow as usize] = cga(Color16::DarkGray, Color16::Black);
        // Classic hyperlink blue, distinct from the dialog's black-on-light-gray
        // prose and from the red used for hotkeys/status shortcuts.
        styles[Role::HelpLink as usize] = cga(Color16::Blue, Color16::LightGray);
        // A muted echo of Selection's black-on-cyan — visible enough to
        // answer "what's current here?" without competing with the actual
        // focus highlight elsewhere on screen.
        styles[Role::SelectionInactive as usize] = cga(Color16::Black, Color16::DarkGray);
        Self { styles }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cga(fg: Color16, bg: Color16) -> Style {
        Style::new().fg(Color::Named(fg)).bg(Color::Named(bg))
    }

    // Tracer bullet: the default theme resolves known roles to CGA styles.
    #[test]
    fn default_theme_resolves_known_roles() {
        let t = Theme::default();
        assert_eq!(
            t.style(Role::DesktopBackground),
            cga(Color16::LightGray, Color16::Blue)
        );
        assert_eq!(
            t.style(Role::MenuSelected),
            cga(Color16::Black, Color16::Green)
        );
        assert_eq!(
            t.style(Role::EditorText),
            cga(Color16::White, Color16::Blue)
        );
        // Phase 5 dialog/control roles.
        assert_eq!(
            t.style(Role::DialogBackground),
            cga(Color16::Black, Color16::LightGray)
        );
        assert_eq!(t.style(Role::Input), cga(Color16::Black, Color16::White));
        // Phase 10 drop shadow.
        assert_eq!(
            t.style(Role::Shadow),
            cga(Color16::DarkGray, Color16::Black)
        );
        // ADR 0020 followable help links.
        assert_eq!(
            t.style(Role::HelpLink),
            cga(Color16::Blue, Color16::LightGray)
        );
        assert_eq!(
            t.style(Role::SelectionInactive),
            cga(Color16::Black, Color16::DarkGray)
        );
        // Phase 10 active/inactive window titles: active is bright + bold, inactive
        // recedes to the dimmer frame grey.
        assert_eq!(
            t.style(Role::WindowTitle),
            cga(Color16::White, Color16::Blue).attrs(Attributes::BOLD)
        );
        assert_eq!(
            t.style(Role::WindowTitleInactive),
            cga(Color16::LightGray, Color16::Blue)
        );
        // Phase 4 menu accelerator letter: red, like the status line's key hint.
        assert_eq!(
            t.style(Role::MenuHotkey),
            cga(Color16::Red, Color16::LightGray)
        );
    }

    #[test]
    fn role_all_is_in_discriminant_order() {
        assert_eq!(Role::COUNT, Role::ALL.len());
        for (i, role) in Role::ALL.iter().enumerate() {
            assert_eq!(*role as usize, i, "Role::ALL[{i}] is out of order");
        }
    }

    #[test]
    fn with_overrides_one_role_only() {
        let custom = cga(Color16::Yellow, Color16::Black);
        let t = Theme::default().with(Role::EditorText, custom);
        // The overridden role takes the new style...
        assert_eq!(t.style(Role::EditorText), custom);
        // ...and an untouched role keeps its default.
        assert_eq!(
            t.style(Role::MenuSelected),
            cga(Color16::Black, Color16::Green)
        );
    }

    #[test]
    fn merge_overrides_only_the_named_field() {
        let base = Theme::default();
        let default_style = base.style(Role::EditorText);

        let t = base.merge("editor_text.fg = rgb(30, 30, 46)\n");

        assert_eq!(t.style(Role::EditorText).fg, Color::Rgb(30, 30, 46));
        // bg/attrs are untouched, not reset to Style::default().
        assert_eq!(t.style(Role::EditorText).bg, default_style.bg);
        assert_eq!(t.style(Role::EditorText).attrs, default_style.attrs);
    }

    #[test]
    fn merge_parses_named_and_default_colors() {
        let t = Theme::default().merge("editor_text.fg = light_gray\nselection.bg = default\n");
        assert_eq!(
            t.style(Role::EditorText).fg,
            Color::Named(Color16::LightGray)
        );
        assert_eq!(t.style(Role::Selection).bg, Color::Default);
    }

    #[test]
    fn merge_rejects_malformed_color_values_leaving_field_unchanged() {
        let base = Theme::default();
        let original = base.style(Role::EditorText).fg;

        let t = base.clone().merge("editor_text.fg = rgb(999, 0, 0)\n");
        assert_eq!(
            t.style(Role::EditorText).fg,
            original,
            "out-of-range rgb component"
        );

        let t = base.clone().merge("editor_text.fg = rgb(1, 2)\n");
        assert_eq!(
            t.style(Role::EditorText).fg,
            original,
            "too few rgb components"
        );

        let t = base.merge("editor_text.fg = not_a_color\n");
        assert_eq!(t.style(Role::EditorText).fg, original, "unknown color name");
    }

    #[test]
    fn merge_parses_attrs_none_and_combinations() {
        let t =
            Theme::default().merge("selection.attrs = bold|underline\nwindow_title.attrs = none\n");
        assert_eq!(
            t.style(Role::Selection).attrs,
            Attributes::BOLD | Attributes::UNDERLINE
        );
        assert_eq!(t.style(Role::WindowTitle).attrs, Attributes::NONE);
    }

    #[test]
    fn merge_rejects_unknown_attrs_token_leaving_attrs_unchanged() {
        let base = Theme::default();
        let original = base.style(Role::Selection).attrs;

        let t = base.merge("selection.attrs = bold|sparkly\n");
        assert_eq!(t.style(Role::Selection).attrs, original);
    }

    #[test]
    fn merge_skips_comments_blank_and_malformed_lines() {
        let base = Theme::default();
        let text = "\n\
            # a comment\n\
            no equals or dot here\n\
            unknown_role.fg = red\n\
            editor_text.unknown_field = red\n\
            editor_text.fg = red\n";

        let t = base.clone().merge(text);
        assert_eq!(t.style(Role::EditorText).fg, Color::Named(Color16::Red));
        // Nothing else moved.
        assert_eq!(t.style(Role::Selection), base.style(Role::Selection));
    }

    #[test]
    fn merge_two_layers_lets_the_second_override_one_field_of_the_first() {
        let t = Theme::default()
            .merge("editor_text.fg = red\neditor_text.bg = black\n")
            .merge("editor_text.fg = blue\n");

        // Second layer's fg wins...
        assert_eq!(t.style(Role::EditorText).fg, Color::Named(Color16::Blue));
        // ...but the first layer's bg survives untouched.
        assert_eq!(t.style(Role::EditorText).bg, Color::Named(Color16::Black));
    }
}
