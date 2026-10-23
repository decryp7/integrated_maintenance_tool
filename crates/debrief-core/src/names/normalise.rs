//! Normalising a name for matching: the file name of a path, without extension, vendor prefix, version or
//! architecture, split into lower-case words (see [`super::segment`]).
//!
//! `CompanyA.Graphic.Builder.exe`, `graphicbuilder.exe`, `GraphicBuilder_x64_1.2.3.exe` and `graphic-builder` all
//! normalise to the words `graphic builder`.

use super::segment::{pieces, Vocabulary};
use std::collections::{HashMap, HashSet};

/// Extensions dropped from the end of a name.
const EXTENSIONS: &[&str] = &[
    "exe", "dll", "so", "dylib", "log", "sys", "bin", "app", "dmp", "tmp", "txt", "json", "xml",
];
/// Architecture and platform words dropped from the end of a name.
const ARCH: &[&str] = &[
    "x86", "x64", "x32", "amd64", "arm64", "win32", "win64", "i386", "i686", "ia64", "arm", "aarch64",
];
/// Most dotted/separated parts of a name that are read.
const MAX_PARTS: usize = 16;

/// A normalised name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormName {
    /// Title-cased words: `Graphic Builder`.
    pub display: String,
    /// Lower-case words: `["graphic", "builder"]`.
    pub tokens: Vec<String>,
}

impl NormName {
    /// The words joined by a space: the grouping key.
    pub fn key(&self) -> String {
        self.tokens.join(" ")
    }

    /// The words joined without separators.
    pub fn squashed(&self) -> String {
        self.tokens.concat()
    }
}

/// The last component of a path or URL-like name.
pub fn file_name(name: &str) -> &str {
    let t = name.trim_end_matches(['/', '\\']);
    t.rsplit(['/', '\\']).next().unwrap_or(t)
}

/// The name without a known extension (`graphicbuilder.exe` is `graphicbuilder`).
pub fn strip_extension(name: &str) -> &str {
    if let Some((stem, ext)) = name.rsplit_once('.') {
        if !stem.is_empty() && EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            return stem;
        }
    }
    name
}

fn is_version_part(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    if l.is_empty() {
        return false;
    }
    if l.chars().all(|c| c.is_ascii_digit()) || ARCH.contains(&l.as_str()) {
        return true;
    }
    l.strip_prefix('v')
        .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
}

fn parts_of(stem: &str) -> Vec<&str> {
    stem.split(|c: char| !c.is_alphanumeric())
        .filter(|p| !p.is_empty())
        .take(MAX_PARTS)
        .collect()
}

/// The first part of names with three or more parts that several different names share (`companya` in
/// `CompanyA.Graphic.Builder.exe` and `CompanyA.Tray.Agent.exe`): vendor prefixes.
pub fn vendor_prefixes<'a>(names: impl IntoIterator<Item = &'a str>) -> HashSet<String> {
    let mut seen: HashMap<String, HashSet<String>> = HashMap::new();
    for n in names {
        let stem = strip_extension(file_name(n));
        let parts = parts_of(stem);
        if parts.len() >= 3 && !parts[0].chars().all(|c| c.is_ascii_digit()) {
            seen.entry(parts[0].to_lowercase()).or_default().insert(stem.to_lowercase());
        }
    }
    seen.into_iter().filter(|(_, v)| v.len() >= 2).map(|(k, _)| k).collect()
}

/// Normalises `name`. `vendors` are the prefixes [`vendor_prefixes`] found; a vendor part is dropped only when
/// words remain. Never empty for a name with a letter or digit in it.
pub fn normalise(name: &str, vocab: &Vocabulary, vendors: &HashSet<String>) -> NormName {
    let stem = strip_extension(file_name(name));
    let mut parts = parts_of(stem);
    let all_parts = parts.clone();
    while parts.len() > 1 && parts.last().is_some_and(|p| is_version_part(p)) {
        parts.pop();
    }
    if parts.len() > 2 && vendors.contains(&parts[0].to_lowercase()) {
        parts.remove(0);
    }
    let mut tokens: Vec<String> = Vec::new();
    for p in &parts {
        tokens.extend(pieces(p, vocab));
    }
    while tokens.len() > 1 && tokens.last().is_some_and(|t| is_version_part(t)) {
        tokens.pop();
    }
    if tokens.is_empty() {
        for p in &all_parts {
            tokens.extend(pieces(p, vocab));
        }
    }
    let display = tokens.iter().map(|t| title(t)).collect::<Vec<_>>().join(" ");
    NormName { display, tokens }
}

fn title(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(n: &str) -> String {
        normalise(n, &Vocabulary::builtin(), &HashSet::new()).key()
    }

    #[test]
    fn spellings_of_one_name_agree() {
        for n in [
            "graphicbuilder.exe",
            "GraphicBuilder.exe",
            "graphic-builder",
            "C:\\Program Files\\App\\GraphicBuilder_x64_1.2.3.exe",
            "/opt/app/bin/graphic_builder",
            "GraphicBuilder.v2.EXE",
        ] {
            assert_eq!(norm(n), "graphic builder", "{n}");
        }
    }

    #[test]
    fn vendor_prefix_is_dropped_when_repeated() {
        let names = ["CompanyA.Graphic.Builder.exe", "CompanyA.Tray.Agent.exe", "other.exe"];
        let vendors = vendor_prefixes(names);
        assert!(vendors.contains("companya"));
        assert_eq!(vendors.len(), 1);
        let n = normalise("CompanyA.Graphic.Builder.exe", &Vocabulary::builtin(), &vendors);
        assert_eq!(n.key(), "graphic builder");
        assert_eq!(n.display, "Graphic Builder");
        assert_eq!(n.squashed(), "graphicbuilder");
        // One name alone: nothing is shared, so the prefix stays.
        assert!(vendor_prefixes(["CompanyA.Graphic.Builder.exe"]).is_empty());
    }

    #[test]
    fn versions_digits_and_acronyms() {
        assert_eq!(norm("XMLParser64.dll"), "xml parser");
        assert_eq!(norm("7zip.exe"), "7 zip");
        assert_eq!(norm("tool_v12"), "tool");
        assert_eq!(norm("64"), "64");
        assert_eq!(norm("x64"), "x");
        assert_eq!(norm("..."), "");
    }

    #[test]
    fn extension_and_path_helpers() {
        assert_eq!(file_name("a/b\\c.txt/"), "c.txt");
        assert_eq!(strip_extension("a.so"), "a");
        assert_eq!(strip_extension(".exe"), ".exe");
        assert_eq!(strip_extension("a.b"), "a.b");
    }

    #[test]
    fn non_english_names_normalise() {
        assert_eq!(norm("Über-Dienst.exe"), "über dienst");
    }
}
