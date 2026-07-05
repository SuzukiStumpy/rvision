# ADR 0022 — Release process, versioning, and the v1.0.0 cut

- **Status:** Accepted
- **Date:** 2026-07-03

## Context

`rvision` now stands on its own (roadmap.md, "Now that `rvision` is standalone").
CI (`test` + `lint`) runs on every push and PR, but there is no release
automation: no tagging, no changelog, no automatic version bumping.

`edit`, the monorepo `rvision` was extracted from, already solved this for
itself in its own ADR 0024: Conventional Commits + `release-please`, one
workspace version shared by both crates via `version.workspace = true`, with a
gated downstream job that builds and attaches cross-platform binaries when a
release is cut. That ADR explicitly named two triggers for revisiting the
question for `rvision` specifically: gaining a second consumer, or moving to
its own repository. The second has now happened. It also already anticipated
the answer for `rvision`'s own release act: "a library has no binary to
ship... `rvision`'s sole future 'release' act is publishing to crates.io."

Two things make `rvision`'s situation simpler than `edit`'s ever was:

- `rvision` is a single ordinary crate — `version = "0.1.0"` lives directly in
  `[package]`. There's no workspace, so none of `edit`'s `version.workspace`
  inheritance or per-crate commit scoping (`feat(rvision):` / `fix(edit):`)
  is needed.
- It's a library, not a binary. There's nothing to build and attach to a
  release; the only artefact anyone will ever want is the published crate
  itself, and publishing to crates.io is intentionally out of scope here (see
  "Consequences").

Separately, the roadmap also flags an open question about how much API
stability to promise, tied to `rvision` still having only one (private, pinned)
consumer. That question is about the *ongoing* discipline of avoiding breakage
— it doesn't have to block *deciding* the version number for the first
independent release, since `edit` depends on `rvision` through a pinned git
reference and isn't tracking its HEAD. Cutting v1.0.0 today costs nothing
downstream; it only commits to bumping a major version number when the API
does eventually break, which is cheap to do.

## Decision

**Adopt `release-please`, matching `edit`'s proven pattern.** Deriving a
semver bump from Conventional Commit history is fiddly bookkeeping with real
edge cases; getting it right doesn't teach us anything about Rust, so it's not
worth hand-rolling. It's CI-only tooling and doesn't touch the runtime crate
budget (ADR 0001).

- `release-please-config.json` uses **`release-type: "rust"`** rather than
  `edit`'s `"simple"` + generic `extra-files`. `edit` needed the generic
  file-patch approach because its version lives in `[workspace.package]` and
  is inherited by member crates; `rvision` has an ordinary `[package].version`,
  which release-please's native Rust support bumps directly (Cargo.toml and
  Cargo.lock's matching entry) with no extra config.
- `release-please` runs on every push to `main` and maintains an open release
  PR from Conventional Commits, updating the version and `CHANGELOG.md`
  (created on first use — no need to hand-author one now). This does not
  build anything; merging that PR is what tags `vX.Y.Z` and cuts the GitHub
  Release.
- **No cross-platform build/attach job.** `edit`'s ADR 0024 build job exists
  because `edit` is a binary. `rvision` has no artefact to build for a
  release — merging the release-please PR is the entire release act for now.
  Publishing to crates.io stays a separate, not-yet-scheduled roadmap item; a
  `cargo publish` step gated on `release_created` (mirroring how `edit` gates
  its build job on the same output) is the natural place to add it later.
- Commit messages stay **plain** Conventional Commits (`feat:`, `fix:`,
  `feat!:` / `BREAKING CHANGE:`), as CLAUDE.md already documents. `edit` needs
  per-crate scoping (`feat(rvision):`) to disambiguate a lockstep multi-crate
  workspace; there is no workspace here, so scoping would add ceremony without
  resolving anything.
- **Cut v1.0.0 for the first independent release.** The manifest declares the
  last real release (`0.1.0`, matching the existing tag); the commit landing
  this ADR and its supporting files carries a `Release-As: 1.0.0` footer so
  the first release-please PR jumps straight to `1.0.0` rather than computing
  a bump from history.

## Consequences

- Day-to-day process is simple: one version, one tag, one changelog, cutting
  a release is "merge the PR."
- The first tag is `v1.0.0` — a public commitment that future breaking changes
  bump the major version. The roadmap's "second consumer" / API-stability
  question is now specifically about how that ongoing promise shapes
  development, not about whether to make it; it stays open, unaffected by this
  cut.
- `rvision` takes on one third-party GitHub Action (`release-please`) on the
  release path only; it touches no runtime code.
- `CHANGELOG.md` doesn't exist yet — release-please creates and maintains it
  starting with the first PR.
- Publishing to crates.io remains unaddressed; this ADR deliberately leaves
  that gap for a follow-up rather than bundling it in.

## Alternatives considered

- **Hand-rolled semver script.** Fully in the project's build-it-ourselves
  spirit, but it reinvents `release-please`'s commit-parsing and version
  edge-case handling without teaching any Rust — the wrong thing to
  hand-roll, for the same reason `edit`'s ADR 0024 rejected it.
- **Staying pre-1.0 (e.g. `0.2.0`) until a second consumer exists.**
  Considered directly: `edit`'s pinned git dependency means it isn't exposed
  to any breakage a `1.0.0` commitment might invite, so deferring further
  only delays a decision that's cheap to correct later via a major bump.
- **`cargo-release` / `cargo-smart-release`.** Rust-native alternatives to
  `release-please`. Not chosen, to stay consistent with the pattern already
  proven in `edit` rather than introduce a second release tool into the
  broader project's toolbox.
- **Independent-versions-from-day-one workspace mode.** Not applicable —
  `rvision` was already the only crate in this repository at extraction, so
  there was never a lockstep workspace to unwind here.

## Addendum (2026-07-05): crates.io publishing wired in

Closes the gap the original decision deliberately left open ("Publishing to
crates.io remains unaddressed").

- **`publish` job added to `release.yml`, gated on `release_created`.**
  Exactly the shape anticipated above: the `release-please` job gains an
  `id: release` and exposes `release_created` as a job output; a new
  `publish` job runs only when it's `true`, checks out the just-tagged
  commit (the default ref for the triggering push — no separate ref
  resolution needed), and runs `cargo publish --locked` with
  `CARGO_REGISTRY_TOKEN` from repository secrets. An ordinary commit to
  `main` still runs `release-please` but never touches `publish`; only
  merging the release PR does.
- **`CARGO_REGISTRY_TOKEN` is a manual, one-time setup step**, not something
  this ADR can automate: create a crates.io account (GitHub OAuth login),
  generate an API token scoped to publish-new/publish-update for `rvision`
  under Account Settings → API Tokens, and add it as a repository secret
  (Settings → Secrets and variables → Actions) under that exact name. Until
  that secret exists, the `publish` job simply fails at the `cargo publish`
  step on the next release PR merge — it does not block `release-please`
  itself or the GitHub Release/tag.
- **Manifest metadata needed for a listing, added to `Cargo.toml`:**
  `readme`, `keywords` (`tui`, `terminal`, `turbovision`, `widgets`), and
  `categories` (`command-line-interface`, `gui` — matching the precedent set
  by comparable crates, e.g. `cursive`; `ratatui` uses
  `command-line-interface` alone). `description`, `license`, and
  `repository` were already present from the original cut. Crate name
  availability was checked directly against the registry API
  (`crates.io/api/v1/crates/rvision` → 404) rather than assumed.

No change to the version/tagging decision above — this only adds the one
remaining release step.

## Addendum (2026-07-05): first real publish, and the API-stability question closed

The `CARGO_REGISTRY_TOKEN` secret above was added the same day, and the very
next release-please PR merge (#4) exercised the whole pipeline for real, not
just in review: it tagged `v2.0.0`, and the `publish` job packaged, verified,
uploaded, and published `rvision v2.0.0` to crates.io without any manual
step — confirmed live via the registry API afterwards.

This also closes the roadmap's long-open "second consumer" question about how
much API-stability to promise once one exists (see roadmap.md, "Now that
`rvision` is standalone"). Two things line up to close it, not just one:

- **The trigger has now genuinely fired.** `edit` depended on `rvision`
  through a pinned git reference, insulated from any breakage; a published
  crate is not — anyone can `cargo add rvision` today, so the semver promise
  this ADR made at the `v1.0.0` cut is no longer a formality, it is a live
  commitment to real strangers.
- **The discipline was already being honoured before publishing made it
  matter.** The jump from `v1.1.0` to `v2.0.0` was not a manual decision —
  it fell out of `release-please` reading a `feat!:` commit (ADR 0028's
  global keyboard accelerator table) as a breaking change and bumping the
  major version automatically, exactly per this ADR's original design.
  There is no daylight between the promise ("breaking changes bump major")
  and the practice (a real breaking change already did, before there was
  any external consumer relying on it) to reconcile.

Nothing further to decide here: the policy is "ordinary semver, derived
mechanically from Conventional Commits," it already governs every release
including the one just published, and there is no separate stricter promise
(e.g. a deprecation window, an LTS branch) being made beyond that. Revisit
only if a real breaking-change incident against a downstream consumer shows
the mechanical policy isn't enough in practice.
