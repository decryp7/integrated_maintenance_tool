# Changelog

What changed for people who use IMT, one section per release. This file is curated by hand (the in-app "What's new"
panel reads it at build time); `git cliff` lists every commit if you need the full history.

## 0.1.0 (2026-10-22)

### Added

- **Shortcuts:** Edit every key binding in Settings > Keyboard, with conflict warnings; the F1 cheat sheet shows your bindings
- **Themes:** A high contrast theme (WCAG AAA text, strong outlines and focus rings) that is chosen automatically when your system asks for more contrast; "Follow system" is the new default theme choice
- **What's new:** This panel, shown once after an update and always under Help > What's new
- **Parsers:** Generate a parser with AI, check it against your file and keep it in the parser library
- **Store:** A Store pane with reports, maintenance and a read-only SQL console
- **Settings:** Performance settings for memory caps, CPU use and presets, with suggested values

### Changed

- **Menus:** Theme choices live under View > Theme; live tail moved to View > Live
- **Palette:** The command palette matches word starts and lists recent commands first

### Fixed

- **Time:** Dates without a year are read near the file's own date
