# Changelog

## [1.0.0](https://github.com/SuzukiStumpy/rvision/compare/rvision-v0.1.0...rvision-v1.0.0) (2026-07-03)


### ⚠ BREAKING CHANGES

* Block::Paragraph's payload changes from String to Vec<Span>. Use Block::text(s) for a plain-text paragraph with no links.

### Features

* add cascading submenus to MenuBar ([b95fc61](https://github.com/SuzukiStumpy/rvision/commit/b95fc61ce1f1a81a64f65768a176c2f2b20cf56f))
* add HelpWindow, a resizable two-pane help browser ([33dfb8e](https://github.com/SuzukiStumpy/rvision/commit/33dfb8ed110ec8a2b8b57907e23bc85dc44cc49f))
* add release automation and versioning (ADR 0022) ([592c765](https://github.com/SuzukiStumpy/rvision/commit/592c765d626a77981a37bb14b9f36444dd8e5892))
* add right-click context menus ([daac945](https://github.com/SuzukiStumpy/rvision/commit/daac9454464bd09fa86924e1dfb9f0482b41f8b0))
* add window-scoped context-sensitive help (ADR 0021) ([d348b9a](https://github.com/SuzukiStumpy/rvision/commit/d348b9ad9d66eafb052f589100f17c3d315c6c8e))
* draw a resize-handle glyph in a resizable window's corner ([c241560](https://github.com/SuzukiStumpy/rvision/commit/c24156031c513739d9ca7cd625d06c89296bdb5a))
* make Desktop a dynamic MDI container (ADR 0016) ([eead15a](https://github.com/SuzukiStumpy/rvision/commit/eead15af3e26cf6204c2d4927739bbc24a9580fe))
* make help {label|target} links followable ([b68ac62](https://github.com/SuzukiStumpy/rvision/commit/b68ac62fd618701366c8992035d6b75eadf9c436))
* scroll chrome is a per-view protocol (ADR 0015) ([ca36861](https://github.com/SuzukiStumpy/rvision/commit/ca36861b4a1c9a0d6c259d4a4a896822eec29860))
* unify Window and Dialog (ADR 0016) ([1505d5c](https://github.com/SuzukiStumpy/rvision/commit/1505d5cb2292d89d6e29eee3bb2bb0a3494c5b84))


### Bug Fixes

* route mouse events to an open pull-down before the region carve-up ([8c75d30](https://github.com/SuzukiStumpy/rvision/commit/8c75d30351dcd79f9ca838eadd43d850e6685ee7))
* Shell must delegate valid() to its Desktop ([b274ccb](https://github.com/SuzukiStumpy/rvision/commit/b274ccb903262a96e000888a2a0e2ba8e337fbb5))
* shorten mdi example window text to fit its width ([5907ea1](https://github.com/SuzukiStumpy/rvision/commit/5907ea15a8353bee10b72fdbce4856b1125ea143))
* size HelpWindow's initial pane split to the true interior, not the outer window bounds ([f87e430](https://github.com/SuzukiStumpy/rvision/commit/f87e430c250fe97c9cb2332fb6d57db5cf404ffb))
