//! The playbook library: one `<id>.json` file per playbook in a folder (the app's configuration folder), so sharing
//! one is copying a file. Reads tolerate damage (a bad file is skipped with a warning); writes are atomic.

use super::{valid_id, Playbook, MAX_FILE_BYTES};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Most playbooks listed from a library folder.
pub const MAX_LIBRARY: usize = 200;

/// What a folder holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Library {
    pub playbooks: Vec<Playbook>,
    /// Files that could not be read, each with what to fix.
    pub warnings: Vec<String>,
}

fn file_of(dir: &Path, id: &str) -> Result<PathBuf> {
    if !valid_id(id) {
        bail!("\"{}\" is not a playbook id", id.chars().take(40).collect::<String>());
    }
    Ok(dir.join(format!("{id}.json")))
}

fn read_file(path: &Path) -> Result<Playbook> {
    let meta = std::fs::metadata(path).context("the file cannot be read")?;
    if meta.len() > MAX_FILE_BYTES as u64 {
        bail!("the file is larger than {} KB", MAX_FILE_BYTES / 1024);
    }
    Playbook::parse_str(&std::fs::read_to_string(path).context("the file cannot be read as text")?)
}

/// The playbooks in `dir` (a missing folder is an empty library), by title.
pub fn list_library(dir: &Path) -> Library {
    let mut lib = Library::default();
    let Ok(rd) = std::fs::read_dir(dir) else { return lib };
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    for p in paths.into_iter().take(MAX_LIBRARY) {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match read_file(&p) {
            // The file name is the id: a file whose content says another id is not trusted to overwrite others.
            Ok(pb) if p.file_stem().is_some_and(|s| *s == *pb.id) => lib.playbooks.push(pb),
            Ok(_) => lib.warnings.push(format!(
                "{name}: the id inside does not match the file name; rename the file or fix the id"
            )),
            Err(e) => lib.warnings.push(format!("{name}: {e:#}")),
        }
    }
    lib.playbooks
        .sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    lib
}

/// Saves (or replaces) a playbook in the library, atomically.
pub fn save_to_library(dir: &Path, playbook: &Playbook) -> Result<()> {
    playbook.check()?;
    let target = file_of(dir, &playbook.id)?;
    std::fs::create_dir_all(dir).context("the playbook folder cannot be created")?;
    let tmp = dir.join(format!(".{}.tmp", playbook.id));
    std::fs::write(&tmp, playbook.to_json()).context("the playbook cannot be written")?;
    std::fs::rename(&tmp, &target)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
        .context("the playbook cannot be saved")
}

/// Removes a playbook from the library; `false` when there was none.
pub fn remove_from_library(dir: &Path, id: &str) -> Result<bool> {
    let p = file_of(dir, id)?;
    match std::fs::remove_file(&p) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).context("the playbook cannot be removed"),
    }
}

/// Reads a playbook file the user chose (any name) without adding it to a library.
pub fn import_from(path: &Path) -> Result<Playbook> {
    read_file(path)
}

/// Writes a playbook to a file the user chose.
pub fn export_to(path: &Path, playbook: &Playbook) -> Result<()> {
    playbook.check()?;
    std::fs::write(path, playbook.to_json()).context("the playbook cannot be written")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playbook::templates;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("imt-playbooks-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn save_list_and_remove() {
        let d = dir("lib");
        assert_eq!(list_library(&d), Library::default());
        let t = templates();
        for p in &t {
            save_to_library(&d, p).unwrap();
        }
        let lib = list_library(&d);
        assert_eq!(lib.playbooks.len(), t.len());
        assert!(lib.warnings.is_empty());
        let titles: Vec<_> = lib.playbooks.iter().map(|p| p.title.to_lowercase()).collect();
        let mut sorted = titles.clone();
        sorted.sort();
        assert_eq!(titles, sorted);
        // Saving again replaces; nothing temporary is left behind.
        save_to_library(&d, &t[0]).unwrap();
        assert!(std::fs::read_dir(&d)
            .unwrap()
            .all(|e| !e.unwrap().file_name().to_string_lossy().starts_with('.')));
        assert!(remove_from_library(&d, &t[0].id).unwrap());
        assert!(!remove_from_library(&d, &t[0].id).unwrap());
        assert!(remove_from_library(&d, "../x").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn damaged_and_misnamed_files_are_skipped_with_a_warning() {
        let d = dir("bad");
        std::fs::create_dir_all(&d).unwrap();
        save_to_library(&d, &templates()[0]).unwrap();
        std::fs::write(d.join("broken.json"), "{ nope").unwrap();
        std::fs::write(d.join("notes.txt"), "ignored").unwrap();
        let mut other = templates()[1].clone();
        other.id = "someone-else".into();
        std::fs::write(d.join("misnamed.json"), other.to_json()).unwrap();
        std::fs::write(d.join("huge.json"), vec![b' '; MAX_FILE_BYTES + 1]).unwrap();
        let lib = list_library(&d);
        assert_eq!(lib.playbooks.len(), 1);
        assert_eq!(lib.warnings.len(), 3, "{:?}", lib.warnings);
        assert!(lib.warnings.iter().any(|w| w.starts_with("broken.json: ")));
        assert!(lib
            .warnings
            .iter()
            .any(|w| w.starts_with("misnamed.json: the id inside does not match")));
        assert!(lib.warnings.iter().any(|w| w.starts_with("huge.json: the file is larger")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn import_and_export_round_trip_any_file_name() {
        let d = dir("io");
        std::fs::create_dir_all(&d).unwrap();
        let p = &templates()[2];
        let f = d.join("shared with colleague.json");
        export_to(&f, p).unwrap();
        assert_eq!(&import_from(&f).unwrap(), p);
        assert!(import_from(&d.join("missing.json")).is_err());
        let mut bad = p.clone();
        bad.id = "Bad".into();
        assert!(export_to(&f, &bad).is_err());
        assert!(save_to_library(&d, &bad).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_playbook_that_cannot_replace_what_is_in_its_place_is_reported_and_leaves_no_temporary_file() {
        let d = dir("blocked");
        let p = templates().remove(0);
        // A non-empty folder where the playbook's file belongs cannot be renamed over.
        let target = d.join(format!("{}.json", p.id));
        std::fs::create_dir_all(target.join("inner")).unwrap();
        let e = save_to_library(&d, &p).unwrap_err();
        assert!(e.to_string().contains("cannot be saved"), "{e:#}");
        assert!(!d.join(format!(".{}.tmp", p.id)).exists());
        std::fs::remove_dir_all(&d).ok();
    }
}
