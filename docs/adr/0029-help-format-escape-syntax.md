# ADR 0029 — Backslash-escape syntax for the `.help` format

- **Status:** Accepted
- **Date:** 2026-07-04

## Context

Authoring `examples/help_builder.help` (a usage guide for the help-authoring
tool itself, which also has to teach its readers the ADR 0013 markup they'll
be writing) hit a wall the format has no answer for: showing the format's
own directives literally. `@topic id Title` as an example line is always
read as a real topic header; `{label|target}` is always eagerly parsed as a
real link. The only workaround available was a `<pre>` block — which stops
link-scanning (so a literal `{label|target}` example works there), but
doesn't stop the `@topic`-shaped-line problem (see below), and is a detour
for something that should be writable as plain prose.

Reading `src/help.rs`'s parser closely while working around this narrowed
the actual problem considerably. `HelpContents::parse` checks for `@topic`,
`<pre>`, `</pre>`, and `#` using exact-prefix/exact-equality tests against
the trimmed line — a line starting `\@topic id Title` *already* fails
`topic_header`'s `strip_prefix("@topic")` today, with zero code changes,
because of the extra leading byte. Same for `\<pre>`/`\</pre>` against the
exact-equality fence check, and `\#` against the comment prefix check. So a
backslash-prefixed line was already falling through as ordinary text; the
only missing piece was that the backslash itself stayed visible in the
rendered output — not an escape mechanism, just an accident of string
matching with a cosmetic gap.

`{label|target}` is genuinely different: `parse_spans` eagerly matches any
well-formed `{...|...}` run as a link, with no dependence on what precedes
the `{`. A backslash there does nothing today. Making it suppress
link-parsing needs a real code change, not just cosmetic stripping.

There was also a real, separate bug worth naming precisely: inside an
*active* `<pre>` block, an unprefixed, exact `@topic id Title` line ends the
block early (`is_topic` is checked unconditionally, even mid-`<pre>`) — the
one place where a literal-directive example can't just be worked around by
placing it off the start of a line, the way it can everywhere else in
prose.

## Decision

**A backslash escapes exactly five characters, each only where it already
has meaning, and nowhere else:**

- `\@`, `\<`, `\#` — recognized only as the first two characters of a
  left-trimmed prose line (the only place `@topic`/`<pre>`/`</pre>`/`#` ever
  have structural meaning). The one leading backslash is stripped when the
  line is stored as paragraph text — `\@topic id Title` renders as
  `@topic id Title`. No change to `topic_header` or the fence/comment
  checks: they already don't match these lines; `strip_leading_escape`
  (`src/help.rs`) only strips the backslash from what gets stored.
- `\{` and `\\` — recognized anywhere inline within a paragraph, handled
  inside `parse_spans` itself (the only place that scans for `{`): `\{`
  emits a literal `{` and never attempts link-matching from that position;
  `\\` emits a literal `\`.
- Any other `\x` — the backslash is left completely untouched, both
  characters pass through as ordinary text. `\e`, `\c`, `\U`, etc. are not
  escape sequences; there is nothing to migrate in existing content that
  merely contains an incidental backslash.

**`Preformatted` (`<pre>`) content is never escape-processed.** It stays
exactly as already documented and tested — byte-for-byte, as authored,
backslashes included. This directly fixes the `<pre>`-early-close bug above
for an *escaped* `\@topic`-shaped line (it already doesn't match
`topic_header`, so it was never going to end the block; escaping changes
nothing here structurally, and this ADR's own test locks in that the stored
bytes still include the backslash, unstripped) without touching the
existing "byte-for-byte" contract at all.

This scope was checked against the one piece of real, shipped content known
to contain a literal backslash: `edit`'s own `crates/edit/src/help.txt` has
`Windows     %APPDATA%\edit\config` — inside a `<pre>` block, and neither
`\e` nor `\c` is one of the five recognized escapes, so it is unaffected by
this change from either direction.

## Consequences

- **Two independent code changes, not one.** Line-start escaping
  (`strip_leading_escape`) is a small, purely cosmetic addition — the
  classification logic needed no changes at all. Inline brace escaping is a
  real behavioural change inside `parse_spans`'s scan, since an unescaped
  `{...|...}` was previously *always* a link with no way to suppress it.
- **No existing hand-authored content needs migration** — confirmed against
  every `.help`/`HELP_SOURCE` string in this repo and in `edit`, none of
  which contain any of the five escape-significant characters in a position
  this change touches.
- **A backslash inside `<pre>` still can't be "consumed"** — an author who
  wants to show a literal `\@topic`-shaped line *inside* a `<pre>` block
  (rather than in ordinary prose) sees the backslash they typed, unstripped,
  in the rendered page. This is the accepted cost of keeping `<pre>`'s
  "byte-for-byte, as authored" promise absolute rather than carving out a
  second, different escape behaviour just for verbatim blocks.
- `examples/help_builder.help`'s own "Links" topic can, as a follow-up, show
  its `{label|target}` syntax example inline instead of inside a `<pre>`
  block, now that `\{` suppresses link-parsing directly — left to whoever
  edits that content next, not part of this change.

## Alternatives considered

- **Universal escaping: `\` suppresses whatever the very next character is,
  regardless of what it is.** The simplest rule to state, but has no
  motivating need beyond the five characters that already carry meaning, and
  would silently corrupt any existing content with an incidental backslash
  (exactly the `edit` Windows-path case above, had it not happened to sit
  inside a `<pre>` block) — every consumer's shipped content would need
  auditing for backslashes that were never meant to be escape sequences.
  Rejected in favour of a fixed, small set of escape-significant characters.
- **Escape-processing `<pre>` content too**, so a literal `\@topic`/`\{a|b}`
  line inside a verbatim block renders with the backslash stripped, same as
  prose. Rejected: it would redefine "byte-for-byte, as authored" — an
  existing, tested contract — for a benefit that's already achievable
  (an unprefixed `@topic`-shaped line already doesn't need escaping to avoid
  the early-close bug once escaped, since it already doesn't match; the only
  loss is the cosmetic backslash staying visible in that one context).
- **A dedicated escape only for `@topic` specifically** (matching the one
  example in the original request), leaving `<pre>`/`#`/`{` unescaped.
  Considered, but a coherent mechanism across every special character the
  format has is barely more code (one small function plus a two-branch
  addition to an existing scan) and avoids a format with an arbitrary,
  hard-to-remember exception list. Decided in favour of covering all five,
  after confirming the `<pre>`/`#` cases were already free (see Decision).

## Addendum (2026-07-04): a properly-closed `<pre>` block was never the bug

Manual testing straight after this landed found a real bug this ADR's own
Context section had wrongly treated as inherent: `<pre>` /
`     @topic id title` / `</pre>` — a single, indented, topic-shaped example
line inside a block that *is* explicitly, correctly closed a line later —
still split into two topics. `is_topic` was being checked unconditionally on
every line while inside a `<pre>` block, so *any* `@topic`-shaped line ended
it early, real close only two lines away or not.

Escaping (`\@topic id title`) always worked around this, but was never
supposed to be the *only* answer for well-formed input — `<pre>` content is
documented as unparsed, full stop, and a document that properly closes its
own block shouldn't need to know about escaping just because one line
inside happens to resemble a directive.

**Fix:** `HelpContents::parse` now materializes `source` into `Vec<&str>`
and, on `<pre>`, calls a new `find_pre_end` that scans *forward* for the
real `</pre>`, however far away, treating everything in between as verbatim
— *unless* a second boundary-shaped line (another `@topic` header, or a
bare `<pre>`) shows up before that close, in which case the block reverts to
being genuinely unclosed and recovers at the *first* boundary line, exactly
as before. The one-boundary tolerance is what keeps this safe: a topic that
forgets its own `</pre>` before starting a new `@topic`, whose *own*
properly-fenced `<pre>` block appears later, must not have that later
close mistaken for the first block's — `find_pre_end`'s second-boundary
check is precisely what stops the scan from reaching across a real topic
change (a scenario stress-tested and locked in as
`a_genuinely_unclosed_pre_still_recovers_at_the_next_real_topic`).

This makes the "backslash inside `<pre>` still can't be consumed" cost
noted in Consequences above largely theoretical for the common case: a
`\@topic`-shaped example line only still needs escaping inside `<pre>` if
the block itself is genuinely unclosed, or contains a second boundary-shaped
line before its own close — an isolated one, properly closed, no longer
needs it at all. Escaping remains what it always was for line-start markers
in ordinary prose and for `{label|target}` inline — this addendum only
narrows the one case that turned out to be a parser bug, not a case for the
escape mechanism to cover.
