//! The `models/` folder in the app data directory: list, add (copy or link), validate and delete. A model's
//! image projector (`mmproj`, ADR 0025) sits next to it as `<model stem>.mmproj.gguf` and is listed with it,
//! never as a model of its own.

use crate::catalog::{is_projector_file_name, is_safe_file_name, projector_file_name};
use crate::error::{AiError, Result};
use crate::gguf::{self, GgufInfo, ProjectorInfo};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub file_name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub info: Option<GgufInfo>,
    /// Why the file cannot be used, if it cannot.
    pub problem: Option<String>,
    /// The model's image projector, when one is installed next to it.
    pub projector: Option<InstalledProjector>,
}

impl InstalledModel {
    /// Whether the model can read images: its projector is installed and fits it.
    pub fn vision(&self) -> bool {
        self.projector.as_ref().is_some_and(|p| p.problem.is_none())
    }
}

/// A model's image projector in the models folder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledProjector {
    pub file_name: String,
    pub size_bytes: u64,
    pub info: Option<ProjectorInfo>,
    /// Why it cannot be used with its model (damaged, or made for another model), if so.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddMode {
    Copy,
    /// Hard link when possible (no extra disk space), otherwise copy.
    Link,
}

pub struct ModelStore {
    dir: PathBuf,
}

/// Turns any user file name into one that is safe inside the models folder.
pub fn sanitize_file_name(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim_start_matches('.').replace("..", "_");
    let stem = stem.strip_suffix(".gguf").or_else(|| stem.strip_suffix(".GGUF")).unwrap_or(&stem);
    let stem: String = stem.chars().take(150).collect();
    format!("{}.gguf", if stem.is_empty() { "model" } else { &stem })
}

impl ModelStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        ModelStore { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Path of an installed file, refusing names that could escape the folder.
    pub fn path_of(&self, file_name: &str) -> Result<PathBuf> {
        if !is_safe_file_name(file_name) {
            return Err(AiError::Refused(format!("'{file_name}' is not a model file name")));
        }
        Ok(self.dir.join(file_name))
    }

    pub fn list(&self) -> Vec<InstalledModel> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<InstalledModel> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if !is_safe_file_name(&name) || is_projector_file_name(&name) || !e.path().is_file() {
                    return None;
                }
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                let (info, problem) = match gguf::inspect(&e.path()) {
                    Ok(info) => (Some(info), None),
                    Err(err) => (None, Some(err.to_string())),
                };
                let projector = self.projector_of(&name, info.as_ref());
                Some(InstalledModel {
                    file_name: name,
                    path: e.path(),
                    size_bytes: size,
                    info,
                    problem,
                    projector,
                })
            })
            .collect();
        out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        out
    }

    /// The projector installed next to `model_file`, checked against the model's header when known.
    fn projector_of(&self, model_file: &str, model: Option<&GgufInfo>) -> Option<InstalledProjector> {
        let file_name = projector_file_name(model_file)?;
        let path = self.dir.join(&file_name);
        let size_bytes = std::fs::metadata(&path).ok().filter(|m| m.is_file())?.len();
        let checked = gguf::inspect_projector(&path).and_then(|p| match model {
            Some(m) => p.check_fits(m).map(|_| p),
            None => Ok(p),
        });
        Some(match checked {
            Ok(info) => InstalledProjector {
                file_name,
                size_bytes,
                info: Some(info),
                problem: None,
            },
            Err(e) => InstalledProjector {
                file_name,
                size_bytes,
                info: None,
                problem: Some(e.to_string()),
            },
        })
    }

    /// The usable projector of an installed model: `None` when it has none; an error when one is there but
    /// cannot be used (damaged, or made for another model).
    pub fn projector_path(&self, model_file: &str) -> Result<Option<PathBuf>> {
        let model = gguf::inspect(&self.path_of(model_file)?)?;
        match self.projector_of(model_file, Some(&model)) {
            None => Ok(None),
            Some(InstalledProjector { problem: Some(p), .. }) => Err(AiError::ModelFile(format!("the image projector: {p}"))),
            Some(p) => Ok(Some(self.dir.join(p.file_name))),
        }
    }

    /// Validates `src` as the image projector of the installed model `model_file` and puts it next to the model
    /// (replacing the one there). A file that is not a projector, or is one made for another model, is refused.
    pub fn add_projector(&self, model_file: &str, src: &Path, mode: AddMode) -> Result<InstalledProjector> {
        let model = gguf::inspect(&self.path_of(model_file)?)?;
        let info = gguf::inspect_projector(src)?;
        info.check_fits(&model)?;
        let name = projector_file_name(model_file).ok_or_else(|| AiError::Refused(format!("'{model_file}' cannot have a projector")))?;
        let dest = self.dir.join(&name);
        let part = self.dir.join(format!("{name}.part"));
        let _ = std::fs::remove_file(&part);
        let linked = mode == AddMode::Link && std::fs::hard_link(src, &part).is_ok();
        let placed = if linked { Ok(0) } else { std::fs::copy(src, &part) }.and_then(|_| std::fs::rename(&part, &dest));
        if let Err(e) = placed {
            let _ = std::fs::remove_file(&part);
            return Err(if e.kind() == std::io::ErrorKind::StorageFull {
                AiError::Disk("the disk is full".into())
            } else {
                AiError::Io(e.to_string())
            });
        }
        Ok(InstalledProjector {
            file_name: name,
            size_bytes: std::fs::metadata(&dest)?.len(),
            info: Some(info),
            problem: None,
        })
    }

    /// Removes the projector of `model_file`, if it has one. The model stays (text only).
    pub fn delete_projector(&self, model_file: &str) -> Result<bool> {
        let Some(name) = projector_file_name(model_file) else {
            return Ok(false);
        };
        let p = self.dir.join(&name);
        if !p.is_file() {
            return Ok(false);
        }
        std::fs::remove_file(&p)?;
        let _ = std::fs::remove_file(self.dir.join(format!("{name}.part")));
        Ok(true)
    }

    /// Validates `src` as a GGUF model and puts it in the folder.
    pub fn add_local(&self, src: &Path, mode: AddMode) -> Result<InstalledModel> {
        self.add_local_as(src, mode, None)
    }

    /// [`Self::add_local`] under `name` (e.g. the catalog file name of a verified download the browser
    /// renamed) instead of the source's own name. A name already taken gets a `-2`, `-3`, ... suffix.
    pub fn add_local_as(&self, src: &Path, mode: AddMode, name: Option<&str>) -> Result<InstalledModel> {
        let info = gguf::inspect(src)?;
        std::fs::create_dir_all(&self.dir)?;
        let original = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let base = sanitize_file_name(name.unwrap_or(&original));
        let mut name = base.clone();
        let mut n = 1;
        while self.dir.join(&name).exists() {
            n += 1;
            name = format!("{}-{n}.gguf", base.trim_end_matches(".gguf"));
        }
        let dest = self.dir.join(&name);
        let linked = mode == AddMode::Link && std::fs::hard_link(src, &dest).is_ok();
        if !linked {
            let part = self.dir.join(format!("{name}.part"));
            let result = std::fs::copy(src, &part).and_then(|_| std::fs::rename(&part, &dest));
            if let Err(e) = result {
                let _ = std::fs::remove_file(&part);
                return Err(if e.kind() == std::io::ErrorKind::StorageFull {
                    AiError::Disk("the disk is full".into())
                } else {
                    AiError::Io(e.to_string())
                });
            }
        }
        let size = std::fs::metadata(&dest)?.len();
        Ok(InstalledModel {
            file_name: name,
            path: dest,
            size_bytes: size,
            info: Some(info),
            problem: None,
            projector: None,
        })
    }

    /// Deletes a model and its projector.
    pub fn delete(&self, file_name: &str) -> Result<()> {
        let p = self.path_of(file_name)?;
        if !p.is_file() || is_projector_file_name(file_name) {
            return Err(AiError::NotFound(file_name.to_string()));
        }
        std::fs::remove_file(&p)?;
        let _ = std::fs::remove_file(self.dir.join(format!("{file_name}.part")));
        self.delete_projector(file_name)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::gguf::{projector_like, qwen_like};

    #[test]
    fn sanitizing_names() {
        assert_eq!(sanitize_file_name("My Model (v2).gguf"), "My_Model__v2_.gguf");
        assert_eq!(sanitize_file_name("../../evil.GGUF"), "___evil.gguf");
        assert_eq!(sanitize_file_name("..."), "model.gguf");
        assert_eq!(sanitize_file_name(""), "model.gguf");
        assert!(is_safe_file_name(&sanitize_file_name("日本語 モデル.gguf")));
    }

    #[test]
    fn add_list_delete_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("my model.gguf");
        std::fs::write(&src, qwen_like()).unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        assert!(store.list().is_empty());
        let a = store.add_local(&src, AddMode::Copy).unwrap();
        assert_eq!(a.file_name, "my_model.gguf");
        assert_eq!(a.info.as_ref().unwrap().architecture, "qwen3");
        let b = store.add_local(&src, AddMode::Link).unwrap();
        assert_eq!(b.file_name, "my_model-2.gguf");
        let c = store.add_local_as(&src, AddMode::Copy, Some("Catalog-Q4.gguf")).unwrap();
        assert_eq!(c.file_name, "Catalog-Q4.gguf");
        store.delete("Catalog-Q4.gguf").unwrap();
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|m| m.problem.is_none() && m.size_bytes > 0));
        store.delete("my_model.gguf").unwrap();
        assert_eq!(store.list().len(), 1);
        assert!(matches!(store.delete("my_model.gguf"), Err(AiError::NotFound(_))));
        assert!(matches!(store.delete("../x.gguf"), Err(AiError::Refused(_))));
        assert!(store.path_of("a/b.gguf").is_err());
    }

    #[test]
    fn a_projector_is_listed_with_its_model_and_deleted_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        let model = tmp.path().join("m.gguf");
        std::fs::write(&model, qwen_like()).unwrap();
        let proj = tmp.path().join("mmproj-F16.gguf");
        std::fs::write(&proj, projector_like(2560)).unwrap();
        let wrong = tmp.path().join("other-mmproj.gguf");
        std::fs::write(&wrong, projector_like(4096)).unwrap();
        // No model yet: nothing to pair with.
        assert!(store.add_projector("m.gguf", &proj, AddMode::Copy).is_err());
        store.add_local(&model, AddMode::Copy).unwrap();
        assert!(!store.list()[0].vision());
        assert_eq!(store.projector_path("m.gguf").unwrap(), None);
        // A model file or another model's projector is refused; the right one pairs.
        assert!(store
            .add_projector("m.gguf", &model, AddMode::Copy)
            .unwrap_err()
            .to_string()
            .contains("not an image projector"));
        assert!(store
            .add_projector("m.gguf", &wrong, AddMode::Copy)
            .unwrap_err()
            .to_string()
            .contains("another model"));
        let p = store.add_projector("m.gguf", &proj, AddMode::Link).unwrap();
        assert_eq!(p.file_name, "m.mmproj.gguf");
        let list = store.list();
        assert_eq!(list.len(), 1, "the projector is not a model of its own");
        assert!(list[0].vision());
        assert_eq!(
            list[0].projector.as_ref().unwrap().info.as_ref().unwrap().projection_dim,
            Some(2560)
        );
        assert_eq!(store.projector_path("m.gguf").unwrap(), Some(store.dir().join("m.mmproj.gguf")));
        // Replacing it keeps one file; a damaged one is listed with its problem and refused for loading.
        store.add_projector("m.gguf", &proj, AddMode::Copy).unwrap();
        std::fs::write(store.dir().join("m.mmproj.gguf"), b"GGUFbroken").unwrap();
        let l = store.list();
        assert!(!l[0].vision() && l[0].projector.as_ref().unwrap().problem.is_some());
        assert!(store.projector_path("m.gguf").unwrap_err().to_string().contains("image projector"));
        assert!(store.delete("m.mmproj.gguf").is_err());
        store.delete("m.gguf").unwrap();
        assert!(!store.dir().join("m.mmproj.gguf").exists());
        assert!(!store.delete_projector("m.gguf").unwrap());
        assert!(!store.delete_projector("../x").unwrap());
    }

    #[test]
    fn rejects_bad_files_and_lists_them_with_a_problem() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("bad.gguf");
        std::fs::write(&bad, b"<html>nope").unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        assert!(store.add_local(&bad, AddMode::Copy).unwrap_err().to_string().contains("web page"));
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.dir().join("broken.gguf"), b"xxxx").unwrap();
        std::fs::write(store.dir().join("notes.txt"), b"x").unwrap();
        std::fs::create_dir(store.dir().join("dir.gguf")).unwrap();
        let l = store.list();
        assert_eq!(l.len(), 1);
        assert!(l[0].problem.as_deref().unwrap().contains("not a GGUF"));
        assert!(ModelStore::new(tmp.path().join("nothing")).list().is_empty());
    }
}
