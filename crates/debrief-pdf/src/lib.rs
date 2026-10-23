//! PDF output for incident reports ([`PdfRenderer`]), an adapter of the
//! engine's [`ReportRenderer`] port.
//!
//! Text is set in fonts embedded in the document: reduced builds of the UI's
//! IBM Plex Sans and JetBrains Mono (SIL OFL 1.1, vendored with their
//! licences in `fonts/`, rebuilt by `fonts/build-fonts.sh`). Each PDF embeds
//! only the glyphs it uses, as `CIDFontType2` fonts with `Identity-H`
//! encoding and a `ToUnicode` CMap, so Latin, Greek and Cyrillic text is
//! selectable, searchable and copyable. The bundled fonts add about 250 KB
//! to the binary; a report embeds a few KB per font it uses.
//!
//! Fallback: characters the fonts do not cover (CJK, emoji, other scripts)
//! print, and extract, as U+FFFD (the replacement character), one per
//! character; control characters and invisible format characters (joiners,
//! variation selectors, skin-tone modifiers) are dropped and tabs print as
//! spaces. The HTML export keeps every character. Chart and timeline
//! snapshots, already sanitised by the engine, are embedded as vector
//! graphics with `svg2pdf`; their labels use the system fonts.

mod fonts;
mod layout;

use anyhow::Result;
use debrief_core::report::{ReportRenderer, ResolvedReport, Theme};

pub use layout::render_pdf;

/// Writes reports as PDF.
pub struct PdfRenderer;

impl ReportRenderer for PdfRenderer {
    fn id(&self) -> &'static str {
        "pdf"
    }

    fn extension(&self) -> &'static str {
        "pdf"
    }

    fn render(&self, report: &ResolvedReport, theme: Theme, out: &mut dyn std::io::Write) -> Result<()> {
        out.write_all(&render_pdf(report, theme)?)?;
        Ok(out.flush()?)
    }
}
