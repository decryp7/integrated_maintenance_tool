use debrief_dump::model::Document;
use debrief_dump::{DumpDocs, Limits};

#[test]
fn a_snapshot_is_a_copy_of_the_open_document() {
    let mut docs = DumpDocs::new(Vec::new(), Limits::default());
    let body = Document::Dump(debrief_dump::testing::sample_dump());
    let id = docs.insert("a.dmp", Some("/a.dmp"), body.clone()).unwrap().id;
    assert_eq!(docs.snapshot(id), Some(body));
    assert!(docs.close(id));
    assert_eq!(docs.snapshot(id), None);
}
