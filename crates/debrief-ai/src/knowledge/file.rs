//! One knowledge file and the files of one scope, validated when they are made: every [`KnowledgeFile`] has a
//! safe id (it names the file on disk), a usable name and text within [`MAX_FILE_BYTES`].

use super::{MAX_FILES, MAX_FILE_BYTES, MAX_NAME_CHARS};
use crate::error::{AiError, Result};
use serde::{Deserialize, Serialize};

/// Where a knowledge file lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KnowledgeScope {
    /// Saved with the project (and the session): a shared project carries it.
    Project,
    /// Debrief's configuration folder: applies to every project.
    Global,
}

impl KnowledgeScope {
    pub fn label(self) -> &'static str {
        match self {
            KnowledgeScope::Project => "project",
            KnowledgeScope::Global => "global",
        }
    }
}

/// A knowledge file: its id (`k1`, `k2`, ...), name, whether the AI reads it, and its Markdown text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeFile {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub text: String,
}

impl KnowledgeFile {
    /// A checked file (see [`check_id`], [`check_name`], [`check_text`]).
    pub fn new(id: &str, name: &str, enabled: bool, text: &str) -> Result<KnowledgeFile> {
        Ok(KnowledgeFile {
            id: check_id(id)?.to_string(),
            name: check_name(name)?,
            enabled,
            text: check_text(text)?,
        })
    }

    /// What a list shows (no text).
    pub fn info(&self, scope: KnowledgeScope) -> KnowledgeInfo {
        KnowledgeInfo {
            id: self.id.clone(),
            scope,
            name: self.name.clone(),
            enabled: self.enabled,
            bytes: self.text.len(),
        }
    }
}

/// A knowledge file in a list: everything but its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeInfo {
    pub id: String,
    pub scope: KnowledgeScope,
    pub name: String,
    pub enabled: bool,
    /// Size of the text in bytes.
    pub bytes: usize,
}

/// A change to a file: only the fields given change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeChange {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub text: Option<String>,
}

/// An id: `k` and 1 to 6 digits. Ids name files on disk, so nothing else is accepted.
pub fn check_id(id: &str) -> Result<&str> {
    let digits = id.strip_prefix('k').unwrap_or("");
    if digits.is_empty() || digits.len() > 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AiError::Invalid("unknown knowledge file".into()));
    }
    Ok(id)
}

/// A name: trimmed, 1 to [`MAX_NAME_CHARS`] characters, no control characters.
pub fn check_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(AiError::Invalid("give the knowledge file a name".into()));
    }
    if n.chars().count() > MAX_NAME_CHARS {
        return Err(AiError::Invalid(format!(
            "the name is too long (at most {MAX_NAME_CHARS} characters)"
        )));
    }
    if n.chars().any(char::is_control) {
        return Err(AiError::Invalid("the name must not contain control characters".into()));
    }
    Ok(n.to_string())
}

/// Text: at most [`MAX_FILE_BYTES`], no NUL characters; line endings become `\n`.
pub fn check_text(text: &str) -> Result<String> {
    if text.len() > MAX_FILE_BYTES {
        return Err(AiError::Invalid(format!(
            "the knowledge file is too large ({} KiB; at most {} KiB): split it or shorten it",
            text.len().div_ceil(1024),
            MAX_FILE_BYTES / 1024
        )));
    }
    if text.contains('\0') {
        return Err(AiError::Invalid(
            "the knowledge file is not text (it contains NUL characters)".into(),
        ));
    }
    Ok(text.replace("\r\n", "\n"))
}

/// The files of one scope, in the user's order, at most [`MAX_FILES`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnowledgeSet {
    files: Vec<KnowledgeFile>,
}

impl KnowledgeSet {
    /// The first [`MAX_FILES`] files, without repeated ids (the first wins).
    pub fn from_files(files: Vec<KnowledgeFile>) -> KnowledgeSet {
        let mut set = KnowledgeSet::default();
        for f in files {
            if set.files.len() < MAX_FILES && set.get(&f.id).is_none() {
                set.files.push(f);
            }
        }
        set
    }

    pub fn files(&self) -> &[KnowledgeFile] {
        &self.files
    }

    pub fn get(&self, id: &str) -> Option<&KnowledgeFile> {
        self.files.iter().find(|f| f.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut KnowledgeFile> {
        self.files
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or_else(|| AiError::NotFound("this knowledge file no longer exists".into()))
    }

    /// The smallest unused id.
    fn fresh_id(&self) -> String {
        (1..)
            .map(|n| format!("k{n}"))
            .find(|id| self.get(id).is_none())
            .unwrap_or_else(|| "k0".into())
    }

    /// Adds an enabled file at the end; returns it.
    pub fn add(&mut self, name: &str, text: &str) -> Result<&KnowledgeFile> {
        if self.files.len() >= MAX_FILES {
            return Err(AiError::Invalid(format!(
                "there are already {MAX_FILES} knowledge files here: remove one or merge them first"
            )));
        }
        let file = KnowledgeFile::new(&self.fresh_id(), name, true, text)?;
        self.files.push(file);
        Ok(&self.files[self.files.len() - 1])
    }

    /// Applies `change` (checked before anything changes); returns the file.
    pub fn update(&mut self, id: &str, change: &KnowledgeChange) -> Result<&KnowledgeFile> {
        let name = change.name.as_deref().map(check_name).transpose()?;
        let text = change.text.as_deref().map(check_text).transpose()?;
        let f = self.get_mut(id)?;
        if let Some(n) = name {
            f.name = n;
        }
        if let Some(t) = text {
            f.text = t;
        }
        if let Some(e) = change.enabled {
            f.enabled = e;
        }
        Ok(f)
    }

    /// Removes a file; returns it.
    pub fn remove(&mut self, id: &str) -> Result<KnowledgeFile> {
        let i = self
            .files
            .iter()
            .position(|f| f.id == id)
            .ok_or_else(|| AiError::NotFound("this knowledge file no longer exists".into()))?;
        Ok(self.files.remove(i))
    }

    pub fn infos(&self, scope: KnowledgeScope) -> Vec<KnowledgeInfo> {
        self.files.iter().map(|f| f.info(scope)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_checked_when_made() {
        assert!(KnowledgeFile::new("k1", " Glossary ", true, "a\r\nb").is_ok_and(|f| f.name == "Glossary" && f.text == "a\nb"));
        for bad in ["", "k", "x1", "k1234567", "k1/../x", "k-1", "../k1"] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
        assert!(check_name("   ").is_err());
        assert!(check_name(&"n".repeat(MAX_NAME_CHARS + 1)).is_err());
        assert!(check_name("a\nb").is_err());
        assert!(check_name(&"n".repeat(MAX_NAME_CHARS)).is_ok());
        assert!(check_text(&"x".repeat(MAX_FILE_BYTES)).is_ok());
        let e = check_text(&"x".repeat(MAX_FILE_BYTES + 1)).unwrap_err().to_string();
        assert!(e.contains("too large") && e.contains("64 KiB"), "{e}");
        assert!(check_text("a\0b").is_err());
        assert_eq!(KnowledgeScope::Global.label(), "global");
        assert_eq!(KnowledgeScope::Project.label(), "project");
    }

    #[test]
    fn a_set_adds_updates_and_removes_within_its_limit() {
        let mut s = KnowledgeSet::default();
        assert_eq!(s.add("A", "one").unwrap().id, "k1");
        assert_eq!(s.add("B", "two").unwrap().id, "k2");
        s.remove("k1").unwrap();
        assert_eq!(s.add("C", "three").unwrap().id, "k1", "the smallest free id");
        let f = s
            .update(
                "k2",
                &KnowledgeChange {
                    name: Some("B2".into()),
                    enabled: Some(false),
                    text: Some("new".into()),
                },
            )
            .unwrap();
        assert_eq!((f.name.as_str(), f.enabled, f.text.as_str()), ("B2", false, "new"));
        // A bad change changes nothing.
        let bad = KnowledgeChange {
            name: Some("ok".into()),
            text: Some("x".repeat(MAX_FILE_BYTES + 1)),
            ..Default::default()
        };
        assert!(s.update("k2", &bad).is_err());
        assert_eq!(s.get("k2").unwrap().name, "B2");
        assert!(matches!(s.update("k9", &KnowledgeChange::default()), Err(AiError::NotFound(_))));
        assert!(matches!(s.remove("k9"), Err(AiError::NotFound(_))));
        let infos = s.infos(KnowledgeScope::Project);
        assert_eq!(infos.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["B2", "C"]);
        assert_eq!(infos[1].bytes, 5);
        for i in 0..MAX_FILES {
            if s.add(&format!("f{i}"), "").is_err() {
                break;
            }
        }
        assert_eq!(s.files().len(), MAX_FILES);
        assert!(s.add("one more", "").unwrap_err().to_string().contains("already"));
    }

    #[test]
    fn a_set_from_files_drops_repeated_ids_and_extra_files() {
        let f = |id: &str| KnowledgeFile::new(id, id, true, "").unwrap();
        let s = KnowledgeSet::from_files(vec![f("k1"), f("k1"), f("k2")]);
        assert_eq!(s.files().len(), 2);
        let many = (1..=MAX_FILES + 5).map(|n| f(&format!("k{n}"))).collect();
        assert_eq!(KnowledgeSet::from_files(many).files().len(), MAX_FILES);
    }
}
