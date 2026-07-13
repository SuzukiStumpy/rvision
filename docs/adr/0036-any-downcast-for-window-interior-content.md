# ADR 0036 — `Any`-based downcast access to a `Window`'s interior content

- **Status:** Accepted
- **Date:** 2026-07-13

## Context

`edit`'s ADR 0018 (Phase 6) needed the editor's own app-level code — its
driver loop, its Window-menu builder, its Save command handler — to reach the
concrete `Document` behind whatever holds it, for things no `View` method
covers (iterating every open document at once, running file I/O against one
by identifier, reading its modified state for a title bar). It considered
"reach the editor via downcast/IDs" and rejected it: *"needs `Any` on
`View`... the framework has so far avoided"* (citing this ADR's own
no-back-references stance), and rejected `Rc<RefCell<Document>>` too, as *"a
smell the codebase has kept out of production code."* `edit` chose instead to
own its `Document`s **concretely**, bypassing `Desktop`/`Window` for its main
MDI entirely and hand-rolling the equivalent chrome (ADR 0016's z-order/
drag/resize, later partly shared via ADR 0033's `arrange` module) — the
"two windowing implementations" `edit`'s own roadmap has tracked since.

That rejection was circumstantial, not principled: at Phase 6, `Desktop` was
still the atrophied single-window stub ADR 0016 later rebuilt, `arrange`
didn't exist, and `rvision` had exactly one consumer with no way to tell
whether a downcast-based seam would even generalize. None of that is true
now — `Desktop` is a real dynamic MDI host (ADR 0016), the arrangement math
is already shared (ADR 0033), and `rvision` is its own repository with a
stated intent toward reuse beyond `edit`. A dedicated grilling session
(`edit`, 2026-07-13) walked the alternatives again with that hindsight and
picked the same shape ADR 0018 rejected, on narrower grounds this time: solve
only the concrete problem — `edit`'s own App-level code needs to reach a
specific window's content by `WindowId` from *outside* the framework's own
`draw`/`handle_event` dispatch — not the more general (and currently
unneeded) case of one `View` reaching another *during* dispatch, which this
ADR's original no-back-references stance still governs untouched.

Two options were weighed for that narrower problem:

- **`Rc<RefCell<T>>`** — a shared handle between `Window`'s interior and
  whatever app-side registry wants concrete access. Rejected again, for the
  same reason ADR 0018 gave: it reintroduces exactly the shared-mutable-state
  back-reference this ADR's `Box<dyn View>` tree-ownership was chosen to
  avoid, and pays for it with a runtime-panicking borrow instead of a
  compile-time-checked one.
- **`Any`-based downcast** — adopted. `Box<dyn View>` already implies
  `Box<dyn View + 'static>` everywhere it's used today (`Window`'s interior,
  `Group`'s children), so every real `View` implementor is already `'static`
  by construction; requiring it costs nothing observable.

## Decision

**A new `AsAny: Any` trait**, with `as_any(&self) -> &dyn Any` /
`as_any_mut(&mut self) -> &mut dyn Any`, blanket-implemented for every
`T: Any` (i.e. every `'static` type) by returning `self`. **`View` gains
`AsAny` as a supertrait** (`pub trait View: AsAny`). Because the blanket impl
covers every `'static` type automatically, no existing `impl View` block
needs to change — a plain default method on `View` itself was tried first
and rejected: `fn as_any(&self) -> &dyn Any { self }` can't unsize-coerce
`&Self` to `&dyn Any` for a generic, possibly-unsized trait `Self` without
adding `where Self: Sized`, and `Sized`-bounded methods are excluded from a
trait's vtable entirely — exactly the callability this exists for. The
supertrait + blanket-impl shape is the standard working idiom for this in
Rust for that reason.

One sharp edge worth recording since it cost a red test to find: `Box<dyn
View>` is itself `'static`, so it *also* matches `impl<T: Any> AsAny for T`
directly — calling `.as_any()` straight on a `Box<dyn View>` resolves to
that (returning the box's own type, not the concrete view inside it) before
Rust ever tries deref-ing into the trait object's vtable. `as_any`/
`as_any_mut` only reach the concrete type when called on an already-`&dyn
View`/`&mut dyn View` receiver — which `Window::interior`/`interior_mut`
below return precisely so real callers can't hit this.

**`Window` gains `interior(&self) -> &dyn View` / `interior_mut(&mut self)
-> &mut dyn View`** — today the interior is reachable only for `Window`'s own
internal dispatch; nothing outside `Window` can get a reference to it at all.

**`Desktop` gains a generic convenience**, `content_mut<T: 'static>(&mut
self, id: WindowId) -> Option<&mut T>`, composing `window_mut` →
`interior_mut` → `downcast_mut` in one call — the shape any caller actually
wants, rather than three chained accessors at every call site. Returns
`None` for an unknown `id` or a window whose interior isn't a `T`, same as
`window_mut` already does for an unknown `id`.

No change to `draw`, `handle_event`, or any other dispatch-path method —
this ADR adds a side channel for app code reaching *in* from outside a
frame's dispatch, not a new capability for one `View` to reach another
*during* one.

## Consequences

- `edit` migrating its document MDI onto `Desktop`/`Window` is now
  technically unblocked — the specific constraint ADR 0018 and ADR 0033 both
  named is gone — but doing so is a separate, later, `edit`-side decision
  (its own ADR/grilling), exactly as ADR 0033 left `edit`'s adoption of
  `arrange` as a follow-up rather than bundling it in. This ADR does not
  migrate anything; `Desktop` is unused by `edit` today and stays that way
  until that separate decision is made.
- Any future `rvision` consumer inherits the same seam for free — the
  mechanism lives entirely in `rvision`, with no `edit`-specific naming or
  assumptions.
- Deliberately *not* pursued: threading a lookup/context parameter through
  `View`'s own method signatures, which would let a `View` reach arbitrary
  app data *during* dispatch. That's a materially bigger, framework-wide
  change with no current consumer needing it — `edit`'s actual requirements
  are all outside-dispatch, app-code-reaching-in. Revisit only if a real
  need for view-side reach during dispatch shows up; building it speculatively
  now would be exactly the "guessed-at abstraction" ADR 0024 warned against.

## Alternatives considered

- **`Rc<RefCell<T>>` shared handle.** Rejected — see Context; reintroduces
  the back-reference this ADR's tree ownership exists to avoid, in exchange
  for a strictly worse (runtime-panicking) failure mode than a `downcast`
  that returns `None`.
- **A closed `WindowContent` enum `edit` matches on**, instead of `Box<dyn
  View>`. Rejected for the same reason ADR 0003 rejected "single big view
  enum" generally: it would have to live in `rvision` to let `Desktop` stay
  generic, forcing the framework to enumerate every consumer's content types
  up front — the opposite of the reuse this ADR is trying to leave room for.
- **Generics over the interior** (`Window<T: View>`). Rejected: a `Desktop`
  needs to hold heterogeneous window content (a document alongside a help
  window, alongside a dialog) in one stack; a `Window<T>` can only ever hold
  one concrete `T`, so heterogeneity forces a fallback to an enum or a trait
  object at exactly the point it matters, buying nothing over `Box<dyn View>`
  plus downcast.
- **Threading a context/registry parameter through `View`'s methods.**
  Considered at length in the `edit`-side grilling this ADR followed; see
  Consequences — deferred as unneeded scope, not rejected outright.
