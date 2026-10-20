//! Saved messages: the project's (UI state key `notifications`) and the app's (`notifications.json` in
//! the app data folder). Versioned; a missing, damaged or newer record is an empty list (with a warning),
//! a damaged message is skipped.

use super::centre::Notice;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

pub const VERSION: u32 = 1;
/// The app file is never read past this size (it holds at most about a hundred short messages).
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticesRecord {
    pub version: u32,
    pub notices: Vec<Notice>,
}

impl NoticesRecord {
    pub fn new(notices: Vec<Notice>) -> NoticesRecord {
        NoticesRecord { version: VERSION, notices }
    }

    pub fn from_value(value: Option<&Value>) -> (Vec<Notice>, Option<String>) {
        let Some(v) = value.filter(|v| !v.is_null()) else {
            return (Vec::new(), None);
        };
        let damaged = || {
            (
                Vec::new(),
                Some("the saved notifications are damaged and were not loaded".to_string()),
            )
        };
        match v.get("version").and_then(Value::as_u64) {
            Some(n) if n == u64::from(VERSION) => {}
            Some(n) if n > u64::from(VERSION) => {
                return (
                    Vec::new(),
                    Some(format!("the notifications were saved by a newer IMT (version {n})")),
                );
            }
            _ => return damaged(),
        }
        let Some(list) = v.get("notices").and_then(Value::as_array) else {
            return damaged();
        };
        let notices: Vec<Notice> = list.iter().filter_map(|n| serde_json::from_value(n.clone()).ok()).collect();
        let warning = (notices.len() < list.len()).then(|| format!("{} damaged notifications were skipped", list.len() - notices.len()));
        (notices, warning)
    }
}

/// Reads the app file; missing, oversized or damaged gives an empty list (logged).
pub fn load_file(path: &Path) -> Vec<Notice> {
    let read = || -> Result<Value, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if meta.len() > MAX_FILE_BYTES {
            return Err("the file is too large".into());
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    };
    match read() {
        Ok(v) => {
            let (notices, warning) = NoticesRecord::from_value(Some(&v));
            if let Some(w) = warning {
                log::warn!("Notifications file: {w}");
            }
            notices
        }
        Err(e) if path.exists() => {
            log::warn!("The notifications file could not be read and starts empty: {e}");
            Vec::new()
        }
        Err(_) => Vec::new(),
    }
}

/// Writes the app file atomically (temporary file, then rename).
pub fn save_file(path: &Path, notices: &[Notice]) -> std::io::Result<()> {
    let text = serde_json::to_vec(&NoticesRecord::new(notices.to_vec())).map_err(std::io::Error::other)?;
    crate::config_store::write_atomic(path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification_centre::centre::{Centre, NewNotice, NoticeId, NoticeScope, NoticeSeverity, NoticeSource};
    use serde_json::json;

    fn notices() -> Vec<Notice> {
        let mut c = Centre::new(10);
        c.post(
            NewNotice::new(NoticeSource::Update, NoticeSeverity::Info, "IMT 1.2 is available", NoticeScope::App),
            NoticeId(1),
            5,
        );
        c.post(
            NewNotice::new(
                NoticeSource::Session,
                NoticeSeverity::Warning,
                "IMT did not close properly",
                NoticeScope::App,
            ),
            NoticeId(2),
            6,
        );
        c.notices().to_vec()
    }

    #[test]
    fn records_round_trip_and_damage_falls_back_to_empty() {
        let v = serde_json::to_value(NoticesRecord::new(notices())).unwrap();
        assert_eq!(NoticesRecord::from_value(Some(&v)), (notices(), None));
        assert_eq!(NoticesRecord::from_value(None), (vec![], None));
        for bad in [json!(1), json!({"version": 1}), json!({"notices": []})] {
            assert!(NoticesRecord::from_value(Some(&bad)).1.unwrap().contains("damaged"));
        }
        assert!(NoticesRecord::from_value(Some(&json!({"version": 3, "notices": []})))
            .1
            .unwrap()
            .contains("newer"));
        let mut partly = v.clone();
        partly["notices"].as_array_mut().unwrap().push(json!({"id": 1}));
        let (list, w) = NoticesRecord::from_value(Some(&partly));
        assert_eq!((list.len(), w.as_deref()), (2, Some("1 damaged notifications were skipped")));
    }

    #[test]
    fn the_app_file_round_trips_and_a_corrupt_file_is_empty() {
        let tmp = crate::test_support::TempDir::new("notices-file");
        let path = tmp.0.join("data/notifications.json");
        assert!(load_file(&path).is_empty(), "missing");
        save_file(&path, &notices()).unwrap();
        assert_eq!(load_file(&path), notices());
        std::fs::write(&path, "{ not json").unwrap();
        assert!(load_file(&path).is_empty(), "corrupt");
        std::fs::write(&path, json!({"version": 99, "notices": []}).to_string()).unwrap();
        assert!(load_file(&path).is_empty(), "newer");
        // Writing into a file where the folder should be fails cleanly.
        let blocked = tmp.0.join("data/notifications.json/x.json");
        assert!(save_file(&blocked, &notices()).is_err());
    }
}
