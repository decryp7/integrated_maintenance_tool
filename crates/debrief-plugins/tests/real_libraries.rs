//! log4net's XmlLayoutSchemaLog4J as the real library writes it (`samples/real-libraries`, from
//! `tools/dotnet-sample-gen`), read by the Formats plugin over the real IPC. The text and JSON files of that
//! folder are tested in debrief-core (`tests/real_libraries.rs`).

mod common;

use debrief_core::model::Level;
use debrief_core::{Profile, Workspace};
use std::path::Path;

#[test]
fn log4net_log4j_xml_keeps_levels_properties_japanese_text_and_inner_exceptions() {
    let registry = common::registry(&[common::FORMATS]);
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/real-libraries/log4net-log4j.xml");
    ws.open(&[file]).unwrap();
    let s = ws.summary();
    assert!(s.unrecognized.is_empty(), "{:?}", s.unrecognized);
    assert_eq!(
        (s.sources[0].importer.as_str(), s.sources[0].parser.as_deref()),
        ("log4j-xml", Some("log4net-xml"))
    );
    let recs = ws.records();
    let levels: Vec<Level> = recs.iter().map(|r| r.level).collect();
    assert_eq!(
        levels,
        [
            Level::Information,
            Level::Debug,
            Level::Information,
            Level::Warning,
            Level::Error,
            Level::Warning,
            Level::Fatal
        ]
    );
    // Epoch milliseconds: 2026-10-14 19:30:00.120 +09:00.
    assert_eq!(recs[0].ts, Some(1_791_973_800_120));
    assert_eq!(
        (&recs[0].fields["Logger"], &recs[0].fields["Thread"]),
        (&"Contoso.Orders.Program".into(), &"1".into())
    );
    assert_eq!(recs[2].message, "注文 88412 を受け付けました（顧客 C-20931、合計 12800 円）");
    assert!(
        recs.iter().all(|r| r.fields.get("Pid").is_some_and(|p| p == 4120 || p == "4120")),
        "log4j:data properties"
    );
    let capture = recs[4].exception.as_deref().unwrap();
    assert!(capture.starts_with("System.InvalidOperationException: Payment capture failed for order 88412"));
    assert!(capture.contains(" ---> System.TimeoutException: 決済ゲートウェイがタイムアウトしました"));
    let fatal = recs[6].exception.as_deref().unwrap();
    assert!(fatal.contains("(Inner Exception #1) System.IO.IOException: ディスクに空き容量がありません"));
    assert!(recs[5].exception.is_none());
}
