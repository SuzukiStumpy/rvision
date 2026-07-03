//! Grapheme-aware text-editing primitives shared by [`InputLine`](super::InputLine)
//! and [`TextArea`](super::TextArea).
//!
//! Pure functions over a `String` and a grapheme-index cursor (ADR 0006) — no
//! view geometry, no drawing. Kept here rather than duplicated in both
//! controls, per the roadmap's own call to share "grapheme-based cursor
//! advance, the insert/overtype toggle, bracketed-paste handling" as free
//! functions instead of merging the two controls into one type (the
//! precedent already set by the menu cascade/hit-test helpers in `menu.rs`).
//!
//! Word motion classifies a grapheme as a "word" character by its first
//! `char` (`is_alphanumeric` or `_`) and skips separator runs around it — the
//! same `backward-word`/`forward-word` convention GNU readline uses, not full
//! UAX #29 word segmentation (simpler, and punctuation reads as an ordinary
//! separator rather than stopping the cursor on its own).

use unicode_segmentation::UnicodeSegmentation;

/// Number of grapheme clusters in `text`.
pub(super) fn grapheme_len(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Byte offset of each grapheme start, with `text.len()` appended — so
/// `starts[i]` is valid for every grapheme index `i` in `0..=len`.
pub(super) fn grapheme_starts(text: &str) -> Vec<usize> {
    let mut starts: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    starts.push(text.len());
    starts
}

/// The grapheme index whose start is at or after `byte` (clamped to `len`).
pub(super) fn byte_to_grapheme(text: &str, byte: usize) -> usize {
    grapheme_starts(text)
        .iter()
        .position(|&s| s >= byte)
        .unwrap_or_else(|| grapheme_len(text))
}

/// Inserts `c` at grapheme index `cursor`, returning the cursor position just
/// past it (advancing past whatever grapheme now sits there, so a combining
/// mark that merges with the previous cluster is handled).
pub(super) fn insert(text: &mut String, cursor: usize, c: char) -> usize {
    let at = grapheme_starts(text)[cursor];
    text.insert(at, c);
    byte_to_grapheme(text, at + c.len_utf8())
}

/// Replaces the grapheme at `cursor` with `c` (overtype mode), falling back
/// to a plain insert past the end so overtype can still extend the text
/// rather than getting stuck.
pub(super) fn overwrite(text: &mut String, cursor: usize, c: char) -> usize {
    if cursor >= grapheme_len(text) {
        return insert(text, cursor, c);
    }
    let starts = grapheme_starts(text);
    text.replace_range(starts[cursor]..starts[cursor + 1], "");
    insert(text, cursor, c)
}

/// Removes the grapheme before `cursor`, returning the new cursor. A no-op
/// (returns `cursor` unchanged) at the start.
pub(super) fn backspace(text: &mut String, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let starts = grapheme_starts(text);
    text.replace_range(starts[cursor - 1]..starts[cursor], "");
    cursor - 1
}

/// Removes the grapheme at `cursor` (cursor position unchanged). A no-op past
/// the end.
pub(super) fn delete(text: &mut String, cursor: usize) {
    if cursor >= grapheme_len(text) {
        return;
    }
    let starts = grapheme_starts(text);
    text.replace_range(starts[cursor]..starts[cursor + 1], "");
}

/// Removes the grapheme range `start..end` (grapheme indices, `start <=
/// end`), returning `start` as the new cursor.
pub(super) fn delete_range(text: &mut String, start: usize, end: usize) -> usize {
    let starts = grapheme_starts(text);
    text.replace_range(starts[start]..starts[end], "");
    start
}

/// Whether a grapheme cluster counts as a "word" character for word motion —
/// its first `char` is alphanumeric or `_`.
fn is_word_char(grapheme: &str) -> bool {
    grapheme
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// The grapheme index of the word boundary to the left of `cursor`: skips a
/// separator run immediately to the left (if any), then skips the word run
/// before it. Clamps at `0`.
pub(super) fn word_left(text: &str, cursor: usize) -> usize {
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut i = cursor.min(graphemes.len());
    while i > 0 && !is_word_char(graphemes[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word_char(graphemes[i - 1]) {
        i -= 1;
    }
    i
}

/// The grapheme index of the word boundary to the right of `cursor`: skips
/// the rest of the current word run (if any), then skips the separator run
/// after it. Clamps at `len`.
pub(super) fn word_right(text: &str, cursor: usize) -> usize {
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let len = graphemes.len();
    let mut i = cursor.min(len);
    while i < len && is_word_char(graphemes[i]) {
        i += 1;
    }
    while i < len && !is_word_char(graphemes[i]) {
        i += 1;
    }
    i
}

/// The selected grapheme range `(start, end)` in document order, or `None` if
/// there is no anchor or the anchor coincides with the cursor (an empty
/// selection reads as no selection).
pub(super) fn selection_range(anchor: Option<usize>, cursor: usize) -> Option<(usize, usize)> {
    let anchor = anchor?;
    if anchor == cursor {
        return None;
    }
    Some((anchor.min(cursor), anchor.max(cursor)))
}

/// The anchor to carry forward after a navigation key seen with `shift`
/// held: extends an existing selection, starts one from `old_cursor` if none
/// exists yet, or clears it outright when `shift` is `false` (a bare
/// navigation key always collapses a selection rather than snapping to
/// either edge first).
pub(super) fn next_anchor(current: Option<usize>, shift: bool, old_cursor: usize) -> Option<usize> {
    if !shift {
        return None;
    }
    current.or(Some(old_cursor))
}

/// If `sel` is `Some((start, end))`, deletes that range and returns `start`
/// as the new cursor; otherwise returns `cursor` unchanged. Callers insert,
/// overwrite, or split a line at the returned position.
pub(super) fn collapse_selection(
    text: &mut String,
    sel: Option<(usize, usize)>,
    cursor: usize,
) -> usize {
    match sel {
        Some((start, end)) => delete_range(text, start, end),
        None => cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grapheme_len_counts_clusters_not_bytes() {
        assert_eq!(grapheme_len("a世b"), 3);
        assert_eq!(grapheme_len(""), 0);
    }

    #[test]
    fn byte_to_grapheme_maps_a_mid_string_offset() {
        let s = "a世b";
        let starts = grapheme_starts(s);
        assert_eq!(starts, vec![0, 1, 1 + "世".len(), s.len()]);
        assert_eq!(byte_to_grapheme(s, 0), 0);
        assert_eq!(byte_to_grapheme(s, 1), 1);
        assert_eq!(byte_to_grapheme(s, s.len()), 3);
    }

    #[test]
    fn insert_advances_past_the_inserted_grapheme() {
        let mut s = "ac".to_string();
        let cursor = insert(&mut s, 1, 'b');
        assert_eq!(s, "abc");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn overwrite_replaces_the_grapheme_under_the_cursor() {
        let mut s = "abc".to_string();
        let cursor = overwrite(&mut s, 1, 'X');
        assert_eq!(s, "aXc");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn overwrite_at_the_end_falls_back_to_insert() {
        let mut s = "ab".to_string();
        let cursor = overwrite(&mut s, 2, 'c');
        assert_eq!(s, "abc");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn backspace_removes_the_grapheme_before_the_cursor_and_is_a_noop_at_start() {
        let mut s = "abc".to_string();
        let cursor = backspace(&mut s, 3);
        assert_eq!(s, "ab");
        assert_eq!(cursor, 2);
        assert_eq!(backspace(&mut s, 0), 0);
        assert_eq!(s, "ab");
    }

    #[test]
    fn delete_removes_the_grapheme_at_the_cursor_and_is_a_noop_past_the_end() {
        let mut s = "abc".to_string();
        delete(&mut s, 0);
        assert_eq!(s, "bc");
        delete(&mut s, 2); // past the end now (len 2)
        assert_eq!(s, "bc");
    }

    #[test]
    fn delete_range_removes_the_whole_span_and_returns_its_start() {
        let mut s = "hello world".to_string();
        let cursor = delete_range(&mut s, 2, 8); // "llo wo" out
        assert_eq!(s, "herld");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn word_right_skips_the_current_word_then_trailing_space() {
        let s = "hello world";
        assert_eq!(word_right(s, 0), 6, "lands at the start of 'world'");
        assert_eq!(word_right(s, 6), 11, "lands at the end of text");
    }

    #[test]
    fn word_left_skips_leading_space_then_the_previous_word() {
        let s = "hello world";
        assert_eq!(word_left(s, 11), 6, "lands at the start of 'world'");
        assert_eq!(word_left(s, 6), 0, "lands at the start of 'hello'");
    }

    #[test]
    fn word_motion_treats_punctuation_as_a_plain_separator() {
        let s = "foo, bar";
        assert_eq!(word_right(s, 0), 5, "skips 'foo' then ', ' together");
        assert_eq!(word_left(s, 8), 5);
    }

    #[test]
    fn word_motion_clamps_at_the_ends() {
        let s = "abc";
        assert_eq!(word_left(s, 0), 0);
        assert_eq!(word_right(s, 3), 3);
    }

    #[test]
    fn word_motion_crosses_a_hard_newline_like_plain_left_right() {
        let s = "one\ntwo";
        assert_eq!(
            word_right(s, 0),
            4,
            "stops at the start of 'two', past the separator run '\\n'"
        );
        assert_eq!(word_left(s, 7), 4);
    }

    #[test]
    fn selection_range_is_none_without_an_anchor_or_when_empty() {
        assert_eq!(selection_range(None, 3), None);
        assert_eq!(
            selection_range(Some(3), 3),
            None,
            "anchor == cursor is empty"
        );
    }

    #[test]
    fn selection_range_orders_start_before_end_regardless_of_direction() {
        assert_eq!(selection_range(Some(2), 5), Some((2, 5)));
        assert_eq!(
            selection_range(Some(5), 2),
            Some((2, 5)),
            "cursor moved left of the anchor"
        );
    }

    #[test]
    fn next_anchor_starts_extends_or_clears() {
        assert_eq!(next_anchor(None, false, 4), None, "no shift: no selection");
        assert_eq!(
            next_anchor(None, true, 4),
            Some(4),
            "shift with no prior anchor starts one here"
        );
        assert_eq!(
            next_anchor(Some(2), true, 4),
            Some(2),
            "shift with an existing anchor keeps it"
        );
        assert_eq!(
            next_anchor(Some(2), false, 4),
            None,
            "a bare key collapses an existing selection"
        );
    }

    #[test]
    fn collapse_selection_deletes_the_range_or_passes_the_cursor_through() {
        let mut s = "hello world".to_string();
        let cursor = collapse_selection(&mut s, Some((2, 8)), 8);
        assert_eq!(s, "herld");
        assert_eq!(cursor, 2);

        let mut s2 = "hello".to_string();
        assert_eq!(collapse_selection(&mut s2, None, 3), 3);
        assert_eq!(s2, "hello", "no selection: text untouched");
    }
}
