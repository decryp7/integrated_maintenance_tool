//! What may appear in an activity entry. Never: file contents, log lines, record messages, AI prompts or
//! answers, web content, secrets. File paths depend on [`Detail`]; query and search text are never logged
//! here (the UI logs a fingerprint, or the text in detailed mode: `core/activity/privacy.ts`).
//!
//! The fingerprint is the same 32-bit FNV-1a as the UI's (`core/activity/privacy.ts`), so a query
//! fingerprinted in the UI matches one fingerprinted here. It is not a cryptographic hash: it shows that
//! two entries used the same text, not what the text was.

use super::config::{config, Detail};
use std::path::Path;
use std::sync::OnceLock;

/// Most parameters one entry carries.
pub const MAX_PARAMS: usize = 16;
/// Longest parameter value (characters); longer ones are cut with `…`.
pub const MAX_VALUE: usize = 200;
/// What replaces a secret.
pub const SECRET: &str = "***";

/// 32-bit FNV-1a of the UTF-8 bytes, as 8 lowercase hex digits.
pub fn fingerprint(text: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in text.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

/// Parameter names whose values are secrets, whatever they contain: `ApiKey`, `serviceKey`, `api_key`,
/// `Token`, `Password`, `Authorization`, ... (but not `Keys` or `Keybinding`, which name things).
pub fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k == "auth"
        || k.ends_with("key")
        || k.contains("_key")
        || [
            "secret",
            "token",
            "password",
            "passwd",
            "credential",
            "cookie",
            "authorization",
            "apikey",
        ]
        .iter()
        .any(|s| k.contains(s))
}

fn cap(text: &str) -> String {
    if text.chars().count() <= MAX_VALUE {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(MAX_VALUE).collect();
        s.push('…');
        s
    }
}

/// Removes secrets (API keys, passwords, connection strings, ...) with the diagnostic-bundle redactor.
pub fn redact(text: &str) -> String {
    static REDACTOR: OnceLock<Option<debrief_collect::Redactor>> = OnceLock::new();
    match REDACTOR.get_or_init(|| debrief_collect::Redactor::new(&debrief_collect::RedactConfig::default()).ok()) {
        Some(r) => r.apply(text, &mut debrief_collect::Counts::new()),
        None => text.to_string(),
    }
}

/// A parameter value as it may be logged: secrets masked by name or by content, capped.
pub fn safe_param(key: &str, value: &str) -> String {
    if is_secret_key(key) {
        return SECRET.into();
    }
    cap(&redact(value))
}

/// A path as it may be logged: `<dir#1a2b3c4d>/name.log` in normal mode, the full path in detailed mode.
pub fn safe_path(path: &Path) -> String {
    safe_path_in(config().detail, path)
}

pub fn safe_path_in(detail: Detail, path: &Path) -> String {
    if detail == Detail::Detailed {
        return cap(&path.to_string_lossy());
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match path.parent().map(|p| p.to_string_lossy().into_owned()).filter(|p| !p.is_empty()) {
        Some(dir) => cap(&format!("<dir#{}>/{name}", fingerprint(&dir))),
        None => cap(&name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_match_the_ui() {
        // Same vectors as core/activity/privacy.spec.ts.
        assert_eq!(fingerprint(""), "811c9dc5");
        assert_eq!(fingerprint("a"), "e40c292c");
        assert_eq!(fingerprint("level:error"), fingerprint("level:error"));
        assert_ne!(fingerprint("level:error"), fingerprint("level:warn"));
        assert_eq!(fingerprint("é").len(), 8);
    }

    #[test]
    fn paths_are_hashed_in_normal_mode() {
        let p = Path::new("/Users/alice/secret-project/app.log");
        let s = safe_path_in(Detail::Normal, p);
        assert!(s.ends_with("/app.log") && s.starts_with("<dir#"), "{s}");
        assert!(!s.contains("alice") && !s.contains("secret-project"));
        assert_eq!(safe_path_in(Detail::Detailed, p), "/Users/alice/secret-project/app.log");
        assert_eq!(safe_path_in(Detail::Normal, Path::new("x.log")), "x.log");
    }

    #[test]
    fn secrets_are_never_logged() {
        assert_eq!(safe_param("serviceApiKey", "abc"), SECRET);
        assert_eq!(safe_param("Token", "abc"), SECRET);
        assert_eq!(safe_param("Password", ""), SECRET);
        assert!(is_secret_key("api_key_id") && is_secret_key("Authorization") && is_secret_key("auth"));
        assert!(!is_secret_key("Keys") && !is_secret_key("Keybinding") && !is_secret_key("Author"));
        let v = safe_param("Note", "password=hunter2 rest");
        assert!(!v.contains("hunter2"), "{v}");
        assert_eq!(safe_param("Count", "3"), "3");
        let long = "x".repeat(500);
        assert_eq!(safe_param("Long", &long).chars().count(), MAX_VALUE + 1);
    }
}
