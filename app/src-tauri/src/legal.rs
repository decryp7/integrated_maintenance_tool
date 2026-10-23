//! The legal texts of the About window: the third-party notices (about 15,000 lines) and the two licences. They are
//! compiled into the binary and handed over by this command when the window asks, so none of them is part of the
//! UI's JavaScript (the old bundled chunk was about 780 KB that every webview of the About window had to parse).

use crate::{err, CmdResult};

const NOTICES: &str = include_str!("../resources/THIRD_PARTY_NOTICES.md");
const LICENSE_MIT: &str = include_str!("../../../LICENSE-MIT");
const LICENSE_APACHE: &str = include_str!("../../../LICENSE-APACHE");

/// The text of one legal document: `notices`, `mit` or `apache`.
pub fn text(doc: &str) -> anyhow::Result<&'static str> {
    match doc {
        "notices" => Ok(NOTICES),
        "mit" => Ok(LICENSE_MIT),
        "apache" => Ok(LICENSE_APACHE),
        other => anyhow::bail!("there is no legal document '{}'", other.chars().take(32).collect::<String>()),
    }
}

/// A legal document of this build (read-only text, no state).
#[tauri::command]
pub async fn legal_text(doc: String) -> CmdResult<String> {
    crate::error::lift(text(&doc).map(str::to_string).map_err(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_documents_are_the_real_texts() {
        assert!(text("notices").unwrap().contains("# Third-party notices"));
        assert!(text("mit").unwrap().contains("MIT License"));
        assert!(text("apache").unwrap().contains("Apache License"));
        assert!(text("notices").unwrap().lines().count() > 1000, "the full list, not a stub");
    }

    #[test]
    fn anything_else_is_refused_with_a_short_message() {
        let e = text(&"x".repeat(500)).unwrap_err().to_string();
        assert!(e.starts_with("there is no legal document") && e.len() < 100);
    }
}
