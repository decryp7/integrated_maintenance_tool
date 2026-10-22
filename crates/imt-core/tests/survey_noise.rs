//! The folder survey on a folder with routine noise: heartbeat lines become a "drop" proposal and a constant
//! periodic message a "collapse repeats" one, each previewed on the sampled events (kept, dropped, changed and
//! examples) before it is offered.

use imt_core::survey::{self, ProposalGroup, ProposalPreview, SurveyControl, SurveyRequest};
use imt_core::workspace::WorkspaceStore;
use imt_core::{Profile, Workspace, DEFAULT_PROFILE_TOML};
use std::fmt::Write as _;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn noisy_folder() -> TempDir {
    let d = std::env::temp_dir().join(format!("imt-survey-noise-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut text = String::new();
    for i in 0..240u32 {
        let (m, s) = (i / 60, i % 60);
        // A heartbeat every second, a constant cache sweep every 4 s, and varied work in between.
        writeln!(text, "2026-10-14 10:{m:02}:{s:02}.000 INFO Heartbeat from agent").unwrap();
        if i % 4 == 0 {
            writeln!(text, "2026-10-14 10:{m:02}:{s:02}.500 INFO Cache sweep finished").unwrap();
        }
        if i % 3 == 0 {
            writeln!(
                text,
                "2026-10-14 10:{m:02}:{s:02}.700 INFO Order {} shipped to customer {}",
                1000 + i,
                i % 17
            )
            .unwrap();
        }
    }
    std::fs::write(d.join("app.log"), text).unwrap();
    TempDir(d)
}

#[test]
fn routine_noise_is_proposed_with_a_preview_of_what_it_drops_or_collapses() {
    let dir = noisy_folder();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir.0)).unwrap();
    let store = WorkspaceStore::new(ws);
    let setup = survey::run(&store, DEFAULT_PROFILE_TOML, &SurveyRequest::default(), &SurveyControl::new()).unwrap();
    assert!(!setup.survey.noise.is_empty(), "{:?}", setup.survey.noise);

    let noise: Vec<_> = setup.proposals.iter().filter(|p| p.group == ProposalGroup::Noise).collect();
    let drop = noise
        .iter()
        .find(|p| p.title.starts_with("Drop") && p.title.contains("Heartbeat"))
        .unwrap_or_else(|| panic!("a drop proposal for the heartbeats: {noise:#?}\nskipped: {:#?}", setup.skipped));
    match &drop.preview {
        ProposalPreview::Pipeline {
            sampled,
            kept,
            dropped,
            examples,
            ..
        } => {
            assert_eq!(kept + dropped, *sampled);
            assert_eq!(*dropped, 240, "every heartbeat and nothing else");
            assert!(!examples.is_empty() && examples.iter().all(|e| e.after.is_empty() && e.before.contains("Heartbeat")));
        }
        other => panic!("{other:?}"),
    }
    assert!(drop.summary.contains("routine"), "{}", drop.summary);
    assert!(drop.toml.contains("drop"), "{}", drop.toml);

    if let Some(collapse) = noise
        .iter()
        .find(|p| p.title.starts_with("Collapse") && p.title.contains("Cache sweep"))
    {
        match &collapse.preview {
            ProposalPreview::Pipeline { kept, dropped, .. } => assert!(*kept > 0 && *dropped > 0),
            other => panic!("{other:?}"),
        }
        assert!(collapse.recommended);
    }

    // The proposals compose into a profile the store accepts, and applying the drop removes the heartbeats.
    let composed = survey::compose(&store, DEFAULT_PROFILE_TOML, &drop.fragments).unwrap();
    let profile = Profile::from_toml(&composed.toml).unwrap();
    use imt_core::ports::{Profiles, Query};
    let before = store.summary().unwrap().total;
    store.set_profile(profile).unwrap().wait().unwrap();
    assert_eq!(store.summary().unwrap().total, before - 240);
}
