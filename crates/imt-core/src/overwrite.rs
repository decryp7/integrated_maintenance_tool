//! No silent overwrite: every command that writes a file the user chose
//! refuses to replace an existing file unless the user confirmed it.

use std::path::{Path, PathBuf};

/// Suffix of the error message; the UI recognises it and asks before retrying with `overwrite`.
pub const FILE_EXISTS_SUFFIX: &str = " already exists; confirm to replace it";

/// The target file exists and replacing it was not confirmed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}{FILE_EXISTS_SUFFIX}", .0.display())]
pub struct FileExists(pub PathBuf);

/// Ok when `path` may be written: it does not exist yet, or `overwrite` is set.
pub fn check_overwrite(path: &Path, overwrite: bool) -> Result<(), FileExists> {
    if !overwrite && std::fs::symlink_metadata(path).is_ok() {
        return Err(FileExists(path.to_path_buf()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_files_need_confirmation() {
        let dir = std::env::temp_dir().join(format!("imt-overwrite-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("out.csv");
        let _ = std::fs::remove_file(&file);
        assert!(check_overwrite(&file, false).is_ok(), "a new file is fine");
        std::fs::write(&file, "x").unwrap();
        let e = check_overwrite(&file, false).unwrap_err();
        assert_eq!(e.to_string(), format!("{} already exists; confirm to replace it", file.display()));
        assert_eq!(
            format!("{:#}", anyhow::Error::from(e)),
            format!("{}{FILE_EXISTS_SUFFIX}", file.display())
        );
        assert!(check_overwrite(&file, true).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
