//! Small behaviours of the public API that the larger suites do not reach: coverage arithmetic, how a link explains
//! itself, the playbook library on a hostile folder.

use imt_core::playbook::{list_library, remove_from_library, save_to_library, Action, Playbook, Step};
use imt_core::ports::Coverage;
use imt_core::related::{ClueKind, Reason, TitleTarget};
use std::path::PathBuf;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("imt-gaps-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn coverage_is_complete_only_when_everything_was_analysed_within_limits() {
    let full = Coverage {
        matched: 10,
        analysed: 10,
        limits: vec![],
    };
    assert!(full.complete());
    assert!(!Coverage {
        analysed: 9,
        ..full.clone()
    }
    .complete());
    assert!(!Coverage {
        limits: vec!["pattern table is full".into()],
        ..full.clone()
    }
    .complete());
    let both = full.clone().and(Coverage {
        matched: 5,
        analysed: 3,
        limits: vec!["sampled".into()],
    });
    assert_eq!(
        (both.matched, both.analysed, both.limits.as_slice()),
        (15, 13, ["sampled".to_string()].as_slice())
    );
    assert!(!both.complete());
}

#[test]
fn a_link_says_why_it_was_made() {
    let title = |target| Reason::Title {
        text: "Orders".into(),
        target,
    };
    assert_eq!(
        title(TitleTarget::Source { name: "orders.log".into() }).label(),
        "title 'Orders' matches source orders.log"
    );
    assert_eq!(
        title(TitleTarget::Lane { lane: "WEB01".into() }).label(),
        "title 'Orders' matches lane WEB01"
    );
    assert_eq!(
        title(TitleTarget::Field {
            field: "Host".into(),
            value: "WEB01".into()
        })
        .label(),
        "title 'Orders' matches Host WEB01"
    );
    assert!(Reason::Time { delta_ms: -600 }.label().starts_with("time"));
    let text = Reason::Text {
        clue: "timed out".into(),
        clue_kind: ClueKind::ErrorText,
        partial: false,
    };
    assert_eq!(text.label(), "text 'timed out'");
}

fn playbook(id: &str) -> Playbook {
    Playbook {
        version: 1,
        id: id.into(),
        title: "Check".into(),
        description: String::new(),
        tags: vec![],
        steps: vec![Step {
            id: "s".into(),
            title: "Look".into(),
            instruction: String::new(),
            action: Action::Manual,
        }],
    }
}

#[test]
fn the_playbook_library_refuses_bad_ids_and_reports_what_it_cannot_do() {
    let dir = Dir::new("library");
    save_to_library(&dir.0, &playbook("outage")).unwrap();
    assert!(list_library(&dir.0).playbooks.iter().any(|p| p.id == "outage"));
    // Removing is reported: once true, then false; an id that could leave the folder is an error.
    assert!(remove_from_library(&dir.0, "outage").unwrap());
    assert!(!remove_from_library(&dir.0, "outage").unwrap());
    assert!(remove_from_library(&dir.0, "../outside").is_err());
    assert!(save_to_library(&dir.0, &playbook("Not Valid!")).is_err());
    // A folder where the file should be cannot be removed as a file: the error says what failed.
    std::fs::create_dir_all(dir.0.join("blocked.json")).unwrap();
    let err = remove_from_library(&dir.0, "blocked").unwrap_err();
    assert!(format!("{err:#}").contains("cannot be removed"), "{err:#}");
    // The library folder itself missing: saving fails with a message, leaving no temporary file behind.
    let missing = dir.0.join("no-such-dir").join("deeper");
    std::fs::write(dir.0.join("no-such-dir"), b"a file, not a folder").unwrap();
    assert!(save_to_library(&missing, &playbook("x")).is_err());
}
