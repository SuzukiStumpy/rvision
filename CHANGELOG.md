# Changelog

## [2.0.2](https://github.com/SuzukiStumpy/rvision/compare/v2.0.1...v2.0.2) (2026-07-05)


### Bug Fixes

* sync FileDialog's result handle when Open/Save ends via mouse or Space ([70a03d0](https://github.com/SuzukiStumpy/rvision/commit/70a03d0e956f7f5b04a24fda682fd5981d996e8b))

## [2.0.1](https://github.com/SuzukiStumpy/rvision/compare/v2.0.0...v2.0.1) (2026-07-05)


### Bug Fixes

* stop linking public docs to private items ([8edc51d](https://github.com/SuzukiStumpy/rvision/commit/8edc51dc2c40ee3a0c321ac3edb2468021a93581))

## [2.0.0](https://github.com/SuzukiStumpy/rvision/compare/v1.1.0...v2.0.0) (2026-07-05)


### ⚠ BREAKING CHANGES

* StatusItem::new's signature changes from (hint, label, key, command) to (hint, label, accelerator).

### Features

* add a system-level global keyboard accelerator table (ADR 0028) ([49cb4d9](https://github.com/SuzukiStumpy/rvision/commit/49cb4d9ac65f3f620c0f229df9db3757dcebea93))
* add ComboBox widget with filtering, type-ahead, and select-only modes (roadmap [#6](https://github.com/SuzukiStumpy/rvision/issues/6)) ([d2f5029](https://github.com/SuzukiStumpy/rvision/commit/d2f502926a1f695338a3ee8539425a6b95f56240))
* add GroupBox widget with titled border, plus a nested-focus-group fix (roadmap [#6](https://github.com/SuzukiStumpy/rvision/issues/6)) ([42f3b9c](https://github.com/SuzukiStumpy/rvision/commit/42f3b9c436bdd53f73638ead9afbe0a5302421c4))
* add StatusPanel widget: line/col + insert/overtype indicator (roadmap [#6](https://github.com/SuzukiStumpy/rvision/issues/6), ADR 0032) ([c5f2545](https://github.com/SuzukiStumpy/rvision/commit/c5f2545371121b8e9180031d6c359d8ba74e9cb2))
* add TextArea widget with word motion and selection (roadmap [#6](https://github.com/SuzukiStumpy/rvision/issues/6)) ([f595150](https://github.com/SuzukiStumpy/rvision/commit/f5951500bed34d26e341a61f36dca3f91f62f53b))
* backslash-escape syntax for the .help format (ADR 0029) ([cb6e62b](https://github.com/SuzukiStumpy/rvision/commit/cb6e62b2192230fe0c4b59a3c10add88f8f9d5f9))
* help_builder example, scroll-bar thumb dragging (roadmap [#3](https://github.com/SuzukiStumpy/rvision/issues/3), ADR 0027) ([c9f02b5](https://github.com/SuzukiStumpy/rvision/commit/c9f02b579a5fbb403f68579f63c3b0b3db86bc07))
* insert/overtype, a truecolour theme, and a theme picker (roadmap [#1](https://github.com/SuzukiStumpy/rvision/issues/1)/[#2](https://github.com/SuzukiStumpy/rvision/issues/2)/[#7](https://github.com/SuzukiStumpy/rvision/issues/7)) ([396e1c5](https://github.com/SuzukiStumpy/rvision/commit/396e1c5835401aa4b6bed53c78b0f1a598f901ba))
* permanent Guide window in help_builder (roadmap [#3](https://github.com/SuzukiStumpy/rvision/issues/3)) ([5a2384c](https://github.com/SuzukiStumpy/rvision/commit/5a2384ce9c2acd0d95d2beb73e1bcd6cbe2a6c80))
* publish to crates.io on release, add package metadata (ADR 0022 addendum) ([6c5cd77](https://github.com/SuzukiStumpy/rvision/commit/6c5cd77fe5044ae08df62914a12895bb69d2710d))
* show the current directory in Open/Save dialogs ([9752c28](https://github.com/SuzukiStumpy/rvision/commit/9752c28cf165e1a229f499346ffc3910ff0e4fba))


### Bug Fixes

* teach help_builder's link-target test to skip escaped braces ([60a7ecf](https://github.com/SuzukiStumpy/rvision/commit/60a7ecffa81f995c746b05eafdc920b714119685))

## [1.1.0](https://github.com/SuzukiStumpy/rvision/compare/v1.0.0...v1.1.0) (2026-07-03)


### Features

* add ColorPicker widget (roadmap [#2](https://github.com/SuzukiStumpy/rvision/issues/2)) ([6b4cac1](https://github.com/SuzukiStumpy/rvision/commit/6b4cac136aab0d416f3c7b1203a365a6ea96dbb2))
* add layered resource loader (ADR 0024, roadmap [#9](https://github.com/SuzukiStumpy/rvision/issues/9)) ([5ff67de](https://github.com/SuzukiStumpy/rvision/commit/5ff67de81b1c2b8223aa7b063aad49fa474798f7))
* add theme file format and merge function (ADR 0025, roadmap [#9](https://github.com/SuzukiStumpy/rvision/issues/9)) ([55efbf2](https://github.com/SuzukiStumpy/rvision/commit/55efbf22e8e2de22c971d86255fc1d0cf2440791))
* add ThemeEditor widget with Restore Defaults (roadmap [#2](https://github.com/SuzukiStumpy/rvision/issues/2)/[#3](https://github.com/SuzukiStumpy/rvision/issues/3), ADR 0026) ([7145a09](https://github.com/SuzukiStumpy/rvision/commit/7145a09c79a1c196d3d70e70688dd0127023bc16))
* detect terminal truecolour capability (ADR 0023) ([c975abe](https://github.com/SuzukiStumpy/rvision/commit/c975abe4b8a923b500a5715da712583c650ebf46))


### Bug Fixes

* ColorPicker OK click didn't commit the selected colour ([dba4ae2](https://github.com/SuzukiStumpy/rvision/commit/dba4ae25a8132f326f982c4f05c03e0d1d1417d1))

## [1.0.0](https://github.com/SuzukiStumpy/rvision/compare/v0.1.0...v1.0.0) (2026-07-03)


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
