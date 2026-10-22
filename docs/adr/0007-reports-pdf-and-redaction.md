# 7. Incident reports: engine-side model, HTML and pure-Rust PDF; one redactor

- Status: accepted, extended by [ADR 0039](0039-known-issues-incident-summary-playbooks.md) (the incident summary is an ordinary report) and
  [ADR 0041](0041-redacted-project-copy.md) (a redacted copy of a whole project)
- Date: 2026-10-17

## Context

Phase 5 ("explain and share") adds incident reports, chart/timeline images, dashboards, a
redacted view and shareable `.imtz` bundles. Reports must export to self-contained HTML and to
PDF with selectable text and vector charts, and every way data leaves IMT (views, exports,
reports, bundles, the collector) must apply the same redaction rules.

PDF options considered:

1. **Webview print-to-PDF.** Tauri 2 has no cross-platform API for it (WebView2 `PrintToPdf`,
   WKWebView `createPDF` and WebKitGTK print operations differ and are not exposed uniformly);
   `window.print()` opens a dialog and cannot be automated or tested headlessly.
2. **typst** (Apache-2.0): excellent output, but a large dependency tree (compiler, fonts,
   packages) for one export format.
3. **printpdf**: its HTML/layout features pull in a heavy stack; SVG support goes through the
   same svg2pdf we would use anyway.
4. **krilla** (MIT/Apache-2.0): high-level, but text requires embedding a font file we would
   have to ship.
5. **pdf-writer + svg2pdf** (MIT/Apache-2.0, from the typst project): a small, low-level writer
   and a converter of SVG trees into PDF XObjects.

## Decision

- The **report document** (`imt_core::report::Report`) lives in the engine: versioned, saved in
  the project (`reports` in `.imtproj`), read leniently (unknown block types are kept verbatim,
  an unreadable report is skipped). The UI edits it; the engine validates it (limits, SVG
  sanitising) at every edge.
- Export resolves a report in the workspace (events looked up by id, every text redacted when
  asked) into a `ResolvedReport`, which a `ReportRenderer` port writes: **HTML** in `imt-core`
  (self-contained, inline CSS and SVG, no scripts, restrictive CSP meta, every text escaped)
  and **PDF** in the adapter crate **`imt-pdf`** using **pdf-writer + svg2pdf**:
  - text uses embedded subsets of bundled OFL fonts (IBM Plex Sans regular, bold and italic,
    JetBrains Mono; renamed "IMT Report Sans/Mono" because a modified Plex may not carry the
    reserved name), written as `Type0`/`CIDFontType2` with `Identity-H` and a `ToUnicode` CMap,
    so text is selectable, searchable and extracts correctly. The fonts cover Latin, Greek and
    Cyrillic; other characters (CJK, emoji) print and extract as U+FFFD, one per character, and
    invisible format characters are dropped (the HTML export keeps every character). Subsetting
    uses `subsetter` and `skrifa` (MIT/Apache-2.0); `crates/imt-pdf/fonts/build-fonts.sh`
    rebuilds the font files (about 250 KB in the binary);
  - chart and timeline snapshots (already sanitised SVG) become vector XObjects, their labels
    set in the system fonts (usvg/fontdb), so they stay selectable too.
- **SVG from the webview is untrusted**: `imt_core::svg` rebuilds it from an allowlist
  (shapes, text, clip paths, gradients, filtered `style`; local `url(#id)` only) for reports and
  image exports; marker icons use the same code with a stricter policy.
- **One redactor** (`imt_core::redact`): built-in detectors (secrets, user paths, e-mail, card
  numbers with Luhn, IPs, GUIDs, phones, host names), custom rules, field scope and a mode
  (mask, salted hash with a fresh salt per export, drop) from the profile's `[redact]` section.
  The redacted view, CSV/CLEF exports, reports and bundles use it; `imt-collect` keeps its own
  replacement texts through a thin adapter over the same rules.

## Consequences

- `cargo deny` stays green: every new crate is MIT, Apache-2.0, BSD or Zlib.
- `imt-core` does not depend on the PDF stack; another renderer (e.g. DOCX) is a new
  `ReportRenderer` adapter.
- PDF body text covers Latin, Greek and Cyrillic. CJK and emoji are replaced by U+FFFD; adding a
  CJK fallback font would grow the binary by several MB and can be done in `imt-pdf` alone,
  without touching the report model or the UI. Rendering has been checked in macOS Preview and
  with pypdf, not yet in Acrobat or on Windows.
