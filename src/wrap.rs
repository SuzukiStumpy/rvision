//! Word-wrapping for dialog and help prose.
//!
//! A pure helper that breaks paragraphs into lines fitting a maximum display
//! width, so callers pass prose rather than pre-broken lines (the ADR 0012
//! follow-up). Width is measured in display columns, the same unit the
//! [`Cell`](crate::cell)/[`Buffer`](crate::buffer) path uses (ADR 0006/0015).

use unicode_width::UnicodeWidthStr;

/// Wraps `text` to at most `width` display columns per line, breaking on ASCII
/// spaces. Returns the wrapped lines, without trailing newlines.
///
/// Existing `'\n'` are hard breaks and always survive (a blank line stays
/// blank). Within a hard line, words are greedily packed: a word joins the
/// current line if the joined result is `<= width` columns, otherwise it starts
/// the next one. Runs of spaces collapse to a single separator. A single word
/// wider than `width` takes its own line and is allowed to overflow rather than
/// split a grapheme cluster; `width == 0` therefore yields one word per line.
/// An empty string wraps to one empty line (`[""]`).
pub fn wrap(text: &str, width: u16) -> Vec<String> {
    wrap_with_offsets(text, width)
        .into_iter()
        .map(|(_, line)| line)
        .collect()
}

/// Same greedy algorithm as [`wrap`], additionally pairing each output line
/// with its start byte offset in `text`. `wrap` is a thin map over this, so
/// there is one wrapping algorithm, not two. (This variant's own space-
/// collapsing rules it out for an *editable* buffer — see
/// [`widgets::TextArea`](crate::widgets::TextArea), which reflows with its
/// own whitespace-preserving pass instead, sharing only [`word_offsets`]
/// with this module.)
pub(crate) fn wrap_with_offsets(text: &str, width: u16) -> Vec<(usize, String)> {
    let width_cols = width as usize;
    let mut out = Vec::new();
    // Hard breaks first, so '\n' (and the blank spacer lines callers rely on,
    // ADR 0012) always survive intact.
    let mut hard_line_start = 0usize;
    for hard_line in text.split('\n') {
        let mut current = String::new();
        let mut current_start = hard_line_start;
        // Mirrors `split_whitespace`'s Unicode-whitespace collapsing, but also
        // yields each word's byte offset within `hard_line`.
        for (word_off, word) in word_offsets(hard_line) {
            let abs_off = hard_line_start + word_off;
            if current.is_empty() {
                current.push_str(word);
                current_start = abs_off;
            } else if current.width() + 1 + word.width() <= width_cols {
                current.push(' ');
                current.push_str(word);
            } else {
                // The word would overflow: flush the line and start anew. A word
                // wider than `width` simply becomes its own (overflowing) line.
                out.push((current_start, std::mem::take(&mut current)));
                current.push_str(word);
                current_start = abs_off;
            }
        }
        // Emit the line even when empty, so a blank hard line stays blank.
        out.push((current_start, current));
        hard_line_start += hard_line.len() + 1; // +1 for the '\n' just consumed
    }
    out
}

/// Each maximal non-whitespace run in `line`, paired with its byte offset —
/// `str::split_whitespace` without discarding the position. Shared with
/// [`widgets::TextArea`](crate::widgets::TextArea)'s own whitespace-preserving
/// reflow, which needs the same word boundaries but none of `wrap`'s
/// space-collapsing.
pub(crate) fn word_offsets(line: &str) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut end = start + c.len_utf8();
        chars.next();
        while let Some(&(i, c2)) = chars.peek() {
            if c2.is_whitespace() {
                break;
            }
            end = i + c2.len_utf8();
            chars.next();
        }
        words.push((start, &line[start..end]));
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(s: &str) -> u16 {
        s.width() as u16
    }

    #[test]
    fn short_text_passes_through_unchanged() {
        assert_eq!(wrap("a short line", 40), vec!["a short line"]);
    }

    #[test]
    fn empty_string_is_one_empty_line() {
        assert_eq!(wrap("", 40), vec![""]);
    }

    #[test]
    fn hard_breaks_and_blank_lines_survive() {
        assert_eq!(
            wrap("one\n\ntwo", 40),
            vec!["one".to_string(), String::new(), "two".to_string()]
        );
    }

    #[test]
    fn a_long_line_breaks_on_spaces_within_width() {
        let out = wrap("the quick brown fox jumps", 9);
        // Greedy: "the quick" (9) | "brown fox" (9) | "jumps".
        assert_eq!(out, vec!["the quick", "brown fox", "jumps"]);
        for line in &out {
            assert!(cols(line) <= 9, "{line:?} fits the width");
        }
    }

    #[test]
    fn runs_of_spaces_collapse_to_one_separator() {
        assert_eq!(wrap("a    b", 40), vec!["a b"]);
    }

    #[test]
    fn an_over_long_word_lands_alone_and_overflows() {
        let out = wrap("hi superlongword ok", 6);
        assert_eq!(out, vec!["hi", "superlongword", "ok"]);
    }

    #[test]
    fn wide_text_breaks_by_columns_not_chars() {
        // Each CJK ideograph is two columns: two of them already fill width 4,
        // so a third wraps even though it is only the third character.
        let out = wrap("世界 你好", 4);
        assert_eq!(out, vec!["世界", "你好"]);
    }

    #[test]
    fn every_line_fits_unless_it_is_one_over_long_word() {
        let text = "alpha beta gammagammagamma delta epsilon zeta eta theta";
        let width = 12;
        for line in wrap(text, width) {
            let single_word = !line.contains(' ');
            assert!(
                cols(&line) <= width || single_word,
                "{line:?} either fits or is a lone over-long word"
            );
        }
    }

    // --- wrap_with_offsets ---

    #[test]
    fn wrap_with_offsets_matches_wrap_line_for_line() {
        let text = "the quick brown fox jumps\n\nover superlongword lazy 世界 你好";
        for width in [0u16, 4, 9, 12, 40] {
            let plain = wrap(text, width);
            let offsetted: Vec<String> = wrap_with_offsets(text, width)
                .into_iter()
                .map(|(_, l)| l)
                .collect();
            assert_eq!(plain, offsetted, "width {width}");
        }
    }

    #[test]
    fn wrap_with_offsets_reports_each_lines_start_in_the_source() {
        let out = wrap_with_offsets("hello world", 5);
        assert_eq!(
            out,
            vec![(0, "hello".to_string()), (6, "world".to_string())]
        );
    }

    #[test]
    fn wrap_with_offsets_points_at_the_first_word_even_after_collapsing_spaces() {
        assert_eq!(
            wrap_with_offsets("a    b", 40),
            vec![(0, "a b".to_string())]
        );
    }

    #[test]
    fn wrap_with_offsets_tracks_blank_hard_lines() {
        let out = wrap_with_offsets("one\n\ntwo", 40);
        assert_eq!(
            out,
            vec![
                (0, "one".to_string()),
                (4, String::new()),
                (5, "two".to_string()),
            ]
        );
    }

    #[test]
    fn every_offset_actually_starts_that_lines_first_word_in_the_source() {
        let text = "the quick brown fox jumps over superlongword lazy dog";
        for (offset, line) in wrap_with_offsets(text, 9) {
            if let Some(first_word) = line.split(' ').next().filter(|w| !w.is_empty()) {
                assert!(
                    text[offset..].starts_with(first_word),
                    "offset {offset} should start {first_word:?} in {line:?}"
                );
            }
        }
    }
}
