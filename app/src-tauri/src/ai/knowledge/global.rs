//! The global knowledge files (ADR 0029): `<config>/ai/knowledge/`, one `<id>.md` per file and `index.json`
//! (versioned: names, order, enabled flags). Reading never fails: a missing index is no knowledge, a damaged one is
//! rebuilt from the `.md` files found, an index from a newer IMT is read but never rewritten. Every change is
//! written (text first, then the index, each atomically) before it is visible.

use crate::config_store::write_atomic;
use crate::sync::lock;
use imt_ai::knowledge::{check_id, KnowledgeChange, KnowledgeFile, KnowledgeSet, MAX_FILES, MAX_FILE_BYTES};
use imt_ai::{AiError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const INDEX_FILE: &str = "index.json";
pub const INDEX_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Index {
    version: u32,
    #[serde(default)]
    files: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    id: String,
    name: String,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

struct State {
    set: Arc<KnowledgeSet>,
    warnings: Vec<String>,
    /// The index was written by a newer IMT: shown, never rewritten.
    read_only: bool,
}

/// The global knowledge files. Without a folder (tests, no configuration folder) they live in memory only.
pub struct GlobalKnowledge {
    dir: Option<PathBuf>,
    state: Mutex<State>,
}

impl GlobalKnowledge {
    /// Reads the folder (never fails; problems become warnings).
    pub fn open(dir: Option<PathBuf>) -> GlobalKnowledge {
        let state = match &dir {
            Some(d) => load(d),
            None => State {
                set: Arc::default(),
                warnings: vec![],
                read_only: false,
            },
        };
        for w in &state.warnings {
            log::warn!("AI knowledge: {w}");
        }
        GlobalKnowledge {
            dir,
            state: Mutex::new(state),
        }
    }

    /// The files as they are now (cheap: shared).
    pub fn snapshot(&self) -> Arc<KnowledgeSet> {
        lock(&self.state).set.clone()
    }

    pub fn warnings(&self) -> Vec<String> {
        lock(&self.state).warnings.clone()
    }

    pub fn read_only(&self) -> bool {
        lock(&self.state).read_only
    }

    pub fn add(&self, name: &str, text: &str) -> Result<KnowledgeFile> {
        self.change(|set| set.add(name, text).cloned(), None)
    }

    pub fn update(&self, id: &str, change: &KnowledgeChange) -> Result<KnowledgeFile> {
        self.change(|set| set.update(id, change).cloned(), None)
    }

    pub fn remove(&self, id: &str) -> Result<KnowledgeFile> {
        check_id(id)?;
        self.change(|set| set.remove(id), Some(id))
    }

    /// Applies `f` to a copy, writes the result and only then makes it current. `removed` names a file to delete.
    fn change(&self, f: impl FnOnce(&mut KnowledgeSet) -> Result<KnowledgeFile>, removed: Option<&str>) -> Result<KnowledgeFile> {
        let mut st = lock(&self.state);
        if st.read_only {
            return Err(AiError::Refused(
                "the knowledge files were saved by a newer IMT and cannot be changed here: update IMT".into(),
            ));
        }
        let mut next = (*st.set).clone();
        let file = f(&mut next)?;
        if let Some(dir) = &self.dir {
            if removed.is_none() {
                write_atomic(&dir.join(format!("{}.md", file.id)), file.text.as_bytes())
                    .map_err(|e| AiError::Io(format!("cannot save the knowledge file: {e}")))?;
            }
            save_index(dir, &next)?;
            if let Some(id) = removed {
                let _ = std::fs::remove_file(dir.join(format!("{id}.md")));
            }
        }
        st.set = Arc::new(next);
        Ok(file)
    }
}

fn save_index(dir: &Path, set: &KnowledgeSet) -> Result<()> {
    let index = Index {
        version: INDEX_VERSION,
        files: set
            .files()
            .iter()
            .map(|f| Entry {
                id: f.id.clone(),
                name: f.name.clone(),
                enabled: f.enabled,
            })
            .collect(),
    };
    let json = serde_json::to_vec_pretty(&index).map_err(|e| AiError::Io(e.to_string()))?;
    write_atomic(&dir.join(INDEX_FILE), &json).map_err(|e| AiError::Io(format!("cannot save the knowledge index: {e}")))
}

/// The text of `<dir>/<id>.md`, if it is a usable knowledge file.
fn read_text(dir: &Path, id: &str) -> std::result::Result<String, String> {
    let path = dir.join(format!("{id}.md"));
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_FILE_BYTES as u64 {
        return Err(format!("larger than {} KiB", MAX_FILE_BYTES / 1024));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|_| "not UTF-8 text".to_string())
}

fn load(dir: &Path) -> State {
    let mut warnings = Vec::new();
    let (entries, read_only) = match std::fs::read_to_string(dir.join(INDEX_FILE)) {
        Ok(text) => match serde_json::from_str::<Index>(&text) {
            Ok(i) if i.version <= INDEX_VERSION => (i.files, false),
            Ok(i) => {
                warnings.push("The knowledge files were saved by a newer IMT: they are used but cannot be changed here.".into());
                (i.files, true)
            }
            Err(_) => {
                warnings.push("The list of knowledge files was damaged; it was rebuilt from the files in the folder.".into());
                (rebuild(dir), false)
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (rebuild(dir), false),
        Err(e) => {
            warnings.push(format!("The knowledge files could not be read ({e})."));
            (vec![], false)
        }
    };
    let mut files = Vec::new();
    for e in entries {
        if check_id(&e.id).is_err() {
            continue;
        }
        let text = match read_text(dir, &e.id) {
            Ok(t) => t,
            Err(why) => {
                warnings.push(format!("The knowledge file \"{}\" could not be read ({why}).", e.name));
                continue;
            }
        };
        match KnowledgeFile::new(&e.id, &e.name, e.enabled, &text) {
            Ok(f) => files.push(f),
            Err(why) => warnings.push(format!("The knowledge file \"{}\" was skipped: {why}.", e.name)),
        }
    }
    State {
        set: Arc::new(KnowledgeSet::from_files(files)),
        warnings,
        read_only,
    }
}

/// Entries for the `<id>.md` files of the folder (enabled, named after the file), in id order.
fn rebuild(dir: &Path) -> Vec<Entry> {
    let mut ids: Vec<(u32, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".md")?.to_string();
            let n = check_id(&id).ok()?[1..].parse().ok()?;
            Some((n, id))
        })
        .collect();
    ids.sort();
    ids.into_iter()
        .take(MAX_FILES)
        .map(|(_, id)| Entry {
            name: id.clone(),
            id,
            enabled: true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn dir(tag: &str) -> (TempDir, PathBuf) {
        let t = TempDir::new(&format!("knowledge-{tag}"));
        let d = t.0.join("knowledge");
        (t, d)
    }

    #[test]
    fn files_are_written_and_read_back() {
        let (_t, d) = dir("round-trip");
        let g = GlobalKnowledge::open(Some(d.clone()));
        assert!(g.snapshot().files().is_empty() && g.warnings().is_empty());
        let a = g.add("Glossary", "ACME-GW: the payment gateway").unwrap();
        g.add("Environments", "PROD: eu-west").unwrap();
        g.update(
            &a.id,
            &KnowledgeChange {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        g.remove("k2").unwrap();
        assert!(!d.join("k2.md").exists());
        let back = GlobalKnowledge::open(Some(d.clone()));
        assert_eq!(back.snapshot(), g.snapshot());
        assert_eq!(back.snapshot().files()[0].text, "ACME-GW: the payment gateway");
        assert!(!back.snapshot().files()[0].enabled);
        assert_eq!(std::fs::read_to_string(d.join("k1.md")).unwrap(), "ACME-GW: the payment gateway");
        assert!(matches!(g.remove("../x"), Err(AiError::Invalid(_))));
    }

    #[test]
    fn a_damaged_index_is_rebuilt_from_the_files() {
        let (_t, d) = dir("damaged");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("k2.md"), "two").unwrap();
        std::fs::write(d.join("k1.md"), "one").unwrap();
        std::fs::write(d.join("notes.md"), "not ours").unwrap();
        std::fs::write(d.join(INDEX_FILE), "{ broken").unwrap();
        let g = GlobalKnowledge::open(Some(d.clone()));
        let s = g.snapshot();
        assert_eq!(
            s.files().iter().map(|f| (f.name.as_str(), f.text.as_str())).collect::<Vec<_>>(),
            [("k1", "one"), ("k2", "two")]
        );
        assert!(g.warnings()[0].contains("rebuilt"));
        // A missing index is rebuilt silently.
        std::fs::remove_file(d.join(INDEX_FILE)).unwrap();
        let g = GlobalKnowledge::open(Some(d.clone()));
        assert_eq!((g.snapshot().files().len(), g.warnings().len()), (2, 0));
        // The next change writes a good index again.
        g.add("New", "three").unwrap();
        assert!(GlobalKnowledge::open(Some(d)).warnings().is_empty());
    }

    #[test]
    fn missing_unreadable_and_oversized_files_are_skipped_with_a_warning() {
        let (_t, d) = dir("skipped");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("k2.md"), "x".repeat(MAX_FILE_BYTES + 1)).unwrap();
        std::fs::write(d.join("k3.md"), [0xff, 0xfe, 0x00]).unwrap();
        std::fs::write(d.join("k4.md"), "ok").unwrap();
        let index = r#"{"version": 1, "files": [
            {"id": "k1", "name": "Missing"}, {"id": "k2", "name": "Huge"}, {"id": "k3", "name": "Binary"},
            {"id": "../../etc/passwd", "name": "Escape"}, {"id": "k4", "name": ""}]}"#;
        std::fs::write(d.join(INDEX_FILE), index).unwrap();
        let g = GlobalKnowledge::open(Some(d));
        assert!(g.snapshot().files().is_empty());
        let w = g.warnings().join("\n");
        for part in [
            "\"Missing\" could not be read",
            "\"Huge\" could not be read (larger than 64 KiB)",
            "not UTF-8",
            "was skipped",
        ] {
            assert!(w.contains(part), "{part}: {w}");
        }
        assert!(!w.contains("Escape"), "unsafe ids are ignored");
    }

    #[test]
    fn a_newer_index_is_used_but_never_rewritten() {
        let (_t, d) = dir("newer");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("k1.md"), "fact").unwrap();
        let index = r#"{"version": 9, "files": [{"id": "k1", "name": "Future", "enabled": true, "extra": 1}]}"#;
        std::fs::write(d.join(INDEX_FILE), index).unwrap();
        let g = GlobalKnowledge::open(Some(d.clone()));
        assert!(g.read_only() && g.warnings()[0].contains("newer"));
        assert_eq!(g.snapshot().files()[0].text, "fact");
        assert!(matches!(g.add("x", ""), Err(AiError::Refused(_))));
        assert_eq!(std::fs::read_to_string(d.join(INDEX_FILE)).unwrap(), index);
    }

    #[test]
    fn a_folder_that_cannot_be_written_keeps_the_files_as_they_were() {
        let (t, _) = dir("unwritable");
        // The "folder" is a file: nothing can be created in it.
        let blocker = t.0.join("blocker");
        std::fs::write(&blocker, "").unwrap();
        let g = GlobalKnowledge::open(Some(blocker.clone()));
        assert!(g.snapshot().files().is_empty());
        assert!(matches!(g.add("Glossary", "x"), Err(AiError::Io(_))));
        assert!(g.snapshot().files().is_empty());
        // In memory only (no folder): changes work and are not written anywhere.
        let m = GlobalKnowledge::open(None);
        m.add("Glossary", "x").unwrap();
        assert_eq!(m.snapshot().files().len(), 1);
    }
}
