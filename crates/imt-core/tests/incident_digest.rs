//! The plain-text digest of a report that a model writes up: only the report's facts go in, the notes stay out,
//! the cited event ids are collected, and the digest is cut at its budget.

use imt_core::incident::digest;
use imt_core::report::{AnnotationItem, Block, BlockItem, Report, Section, StoredBlock};
use serde_json::json;

fn item(id: &str, block: Block) -> StoredBlock {
    StoredBlock::Known(BlockItem { id: id.into(), block })
}

fn report(sections: Vec<Section>) -> Report {
    Report {
        version: 1,
        id: "r".into(),
        title: "t".into(),
        summary: String::new(),
        created_at: 0,
        updated_at: 0,
        sections,
    }
}

fn section(title: &str, blocks: Vec<StoredBlock>) -> Section {
    Section {
        id: title.to_lowercase(),
        title: title.into(),
        blocks,
    }
}

fn sample() -> Report {
    report(vec![
        section(
            "Summary",
            vec![
                item(
                    "a",
                    Block::Text {
                        markdown: "**Events:** 12".into(),
                    },
                ),
                item(
                    "b",
                    Block::Finding {
                        severity: "warning".into(),
                        title: "Slow".into(),
                        detail: "p95 is 3 s".into(),
                    },
                ),
            ],
        ),
        section(
            "Top problems",
            vec![
                item(
                    "c",
                    Block::Table {
                        caption: "Most frequent".into(),
                        query: "q".into(),
                        columns: vec!["template".into(), "n".into()],
                        rows: vec![vec![json!("timeout"), json!(7)], vec![json!("refused"), json!(2)]],
                    },
                ),
                item(
                    "d",
                    Block::Events {
                        caption: "Evidence".into(),
                        ids: (1..=20).rev().chain([5, 5]).collect(),
                    },
                ),
                item(
                    "e",
                    Block::Annotations {
                        caption: "Marks".into(),
                        items: vec![AnnotationItem {
                            from: 1_791_972_000_000,
                            to: None,
                            label: "deploy".into(),
                            kind: "deploy".into(),
                        }],
                    },
                ),
                // Blocks without facts (charts, timelines) and blocks of a newer version add nothing.
                item(
                    "f",
                    Block::Timeline {
                        caption: "tl".into(),
                        svg: None,
                        from: None,
                        to: None,
                    },
                ),
                StoredBlock::Unknown(json!({"type": "hologram"})),
            ],
        ),
        section(
            "Notes",
            vec![item(
                "g",
                Block::Text {
                    markdown: "Customer says it began at 09:35.".into(),
                },
            )],
        ),
    ])
}

#[test]
fn the_digest_holds_the_facts_and_the_cited_event_ids() {
    let (text, ids) = digest(&sample(), 10_000);
    assert!(
        text.contains("## Summary\n**Events:** 12\n- [warning] Slow: p95 is 3 s\n"),
        "{text}"
    );
    assert!(
        text.contains("Most frequent (template | n):\n- timeout | 7\n- refused | 2\n"),
        "{text}"
    );
    assert!(
        text.contains("Evidence: events 20, 19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9\n"),
        "only the first 12 are listed"
    );
    assert!(text.contains("- 2026-10-14 10:00:00 UTC: deploy\n"), "{text}");
    assert!(!text.contains("Customer says"), "the investigator's notes are not sent as text");
    assert!(text.contains("(the investigator's notes are in the report)"));
    assert!(!text.contains("hologram"));
    assert_eq!(ids, (1..=20).collect::<Vec<u32>>(), "all cited ids, sorted and unique");
}

#[test]
fn the_digest_is_cut_at_its_budget_and_says_so() {
    let (text, ids) = digest(&sample(), 30);
    assert!(text.ends_with("(cut: the report is longer)\n"), "{text}");
    assert!(text.contains("## Summary"), "{text}");
    assert!(!text.contains("Top problems"), "nothing after the cut");
    assert!(ids.is_empty(), "ids after the cut are not cited");
    let (empty, none) = digest(&report(vec![]), 100);
    assert!(empty.is_empty() && none.is_empty());
}

#[test]
fn an_unrepresentable_time_is_shown_as_the_raw_number() {
    let r = report(vec![section(
        "Marks",
        vec![item(
            "a",
            Block::Annotations {
                caption: "c".into(),
                items: vec![AnnotationItem {
                    from: i64::MAX,
                    to: None,
                    label: "far future".into(),
                    kind: String::new(),
                }],
            },
        )],
    )]);
    let (text, _) = digest(&r, 1000);
    assert!(text.contains(&format!("- {}: far future", i64::MAX)), "{text}");
}
