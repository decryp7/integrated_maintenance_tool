#![no_main]
//! The notes (ADR 0021) and knowledge-file records a project or session carries: any JSON through
//! `NoteRecord::read` / `KnowledgeRecord::read` (the first byte selects), then what was kept written back and read
//! again without a warning, and the AI blocks built from it (notes context, knowledge block markers intact).
use debrief_ai::knowledge::{assemble, KnowledgeRecord, BLOCK_CLOSE, BLOCK_OPEN};
use debrief_notes::context::{self, Focus};
use debrief_notes::NoteRecord;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(rest) else {
        return;
    };
    if pick % 2 == 0 {
        let (set, _) = NoteRecord::read(Some(&v));
        let back = serde_json::to_value(NoteRecord::of(&set)).expect("notes serialise");
        assert_eq!(NoteRecord::read(Some(&back)), (set.clone(), None));
        let _ = context::render(set.notes(), &Focus::default(), (pick as usize) << 6);
    } else {
        let (set, _) = KnowledgeRecord::read(Some(&v));
        assert_eq!(
            KnowledgeRecord::read(Some(&KnowledgeRecord::of(&set).to_value())),
            (set.clone(), None)
        );
        if let Some(block) = assemble(set.files(), &[], (pick as usize) << 8) {
            assert_eq!(block.text.matches(BLOCK_OPEN).count(), 1);
            assert_eq!(block.text.matches(BLOCK_CLOSE).count(), 1);
        }
    }
});
