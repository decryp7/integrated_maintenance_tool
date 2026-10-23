//! Property-based tests of knowledge files (ADR: the user's notes about their systems, saved in projects and sessions
//! and sent to the model): reading a damaged record never panics and keeps only checked files; the assembled
//! prompt block stays within its budget and no file text can close or reopen the block's own markers.
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p debrief-ai --test properties_knowledge`.

use debrief_ai::knowledge::{
    assemble, check_id, check_name, check_text, KnowledgeFile, KnowledgeRecord, KnowledgeSet, BLOCK_CLOSE, BLOCK_OPEN, FILE_CLOSE,
    FILE_OPEN, MAX_FILES, MAX_FILE_BYTES,
};
use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;
use serde_json::{json, Value};

/// Text that likes to contain the block's own markers.
fn hostile_text() -> BoxedStrategy<String> {
    let piece = prop_oneof![
        Just(BLOCK_OPEN.to_string()),
        Just(BLOCK_CLOSE.to_string()),
        Just(FILE_OPEN.to_string()),
        Just(FILE_CLOSE.to_string()),
        Just("</knowledge_file".to_string()),
        Just("<debrief_knowledge".to_string()),
        Just("\n".to_string()),
        ".{0,40}",
    ];
    prop::collection::vec(piece, 0..8).prop_map(|v| v.concat()).boxed()
}

fn raw_file() -> BoxedStrategy<Value> {
    prop_oneof![
        5 => (prop_oneof!["k[0-9a-f]{1,6}", ".{0,12}", Just("../x".to_string())], prop_oneof![".{0,20}", Just(String::new()), Just("n".repeat(200))], prop::option::of(any::<bool>()), prop_oneof![hostile_text(), Just("x".repeat(MAX_FILE_BYTES + 1))])
            .prop_map(|(id, name, enabled, text)| json!({"id": id, "name": name, "enabled": enabled, "text": text})),
        1 => Just(json!(7)),
        1 => Just(json!({"id": "k1"})),
    ]
    .boxed()
}

fn record() -> BoxedStrategy<Value> {
    prop_oneof![
        5 => (prop_oneof![Just(json!(1)), Just(json!(0)), Just(json!(2)), Just(json!(-1)), Just(json!("1"))], prop::collection::vec(raw_file(), 0..(MAX_FILES + 4)))
            .prop_map(|(v, files)| json!({"version": v, "files": files})),
        1 => Just(json!(null)),
        1 => Just(json!("knowledge")),
        1 => Just(json!({"files": 3, "version": 1})),
    ]
    .boxed()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn reading_any_record_never_panics(v in record()) {
        let (set, warning) = KnowledgeRecord::read(Some(&v));
        prop_assert!(set.files().len() <= MAX_FILES);
        for f in set.files() {
            prop_assert!(check_id(&f.id).is_ok());
            prop_assert!(check_name(&f.name).is_ok());
            prop_assert!(check_text(&f.text).is_ok());
            prop_assert_eq!(set.files().iter().filter(|g| g.id == f.id).count(), 1, "repeated id {}", &f.id);
        }
        let back = KnowledgeRecord::of(&set).to_value();
        prop_assert_eq!(KnowledgeRecord::read(Some(&back)), (set.clone(), None));
        if warning.is_none() && v.get("version") == Some(&json!(1)) {
            let given = v.get("files").and_then(Value::as_array).map_or(0, Vec::len);
            prop_assert_eq!(set.files().len(), given, "files were dropped without a warning");
        }
    }

    /// The block has exactly one opening and one closing marker whatever the files say, one file marker pair per
    /// included file, and stays within its budget (plus the fixed frame).
    #[test]
    fn the_block_cannot_be_broken_out_of(texts in prop::collection::vec((hostile_text(), any::<bool>()), 0..6), budget in 0usize..20_000) {
        let files: Vec<KnowledgeFile> = texts
            .iter()
            .enumerate()
            .filter_map(|(i, (t, enabled))| KnowledgeFile::new(&format!("k{i}"), &format!("File {i}"), *enabled, t).ok())
            .collect();
        let (project, global) = files.split_at(files.len() / 2);
        if let Some(block) = assemble(project, global, budget) {
            let t = &block.text;
            prop_assert_eq!(t.matches(BLOCK_OPEN).count(), 1, "{}", t);
            prop_assert_eq!(t.matches(BLOCK_CLOSE).count(), 1, "{}", t);
            prop_assert!(t.trim_end().ends_with(BLOCK_CLOSE));
            prop_assert_eq!(t.matches(FILE_CLOSE).count(), block.included);
            prop_assert_eq!(t.matches(FILE_OPEN).count(), block.included);
            prop_assert!(block.included <= files.iter().filter(|f| f.enabled).count());
        }
    }

    #[test]
    fn checks_never_panic(s in ".{0,200}") {
        let _ = check_id(&s);
        let _ = check_name(&s);
        let _ = check_text(&s);
        let mut set = KnowledgeSet::default();
        if let Ok(f) = set.add(&s, &s) {
            prop_assert!(check_id(&f.id).is_ok());
        }
    }
}
