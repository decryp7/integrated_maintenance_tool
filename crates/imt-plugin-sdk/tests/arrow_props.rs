//! Record ↔ Arrow batch is lossless for every value a record can hold: all levels, missing and extreme
//! times, any text, and fields of every JSON type (nested objects and arrays included, order kept).

use imt_core::model::{Fields, Level, Record};
use imt_plugin_sdk::arrow::{decode, encode, WithExtra};
use proptest::prelude::*;
use serde_json::Value;

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>().prop_filter("finite", |f| f.is_finite()).prop_map(Value::from),
        ".{0,12}".prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::vec(("[a-z@ ._-]{0,6}", inner), 0..4).prop_map(|kv| Value::Object(kv.into_iter().collect())),
        ]
    })
}

fn fields() -> impl Strategy<Value = Fields> {
    prop::collection::vec(("\\PC{0,10}", json_value()), 0..6).prop_map(|kv| kv.into_iter().collect())
}

fn record() -> impl Strategy<Value = Record> {
    (
        (
            any::<Option<i64>>(),
            0..6usize,
            "\\PC{0,40}",
            any::<Option<String>>(),
            any::<Option<String>>(),
        ),
        (fields(), any::<u32>(), any::<u32>(), any::<u32>(), any::<u64>()),
    )
        .prop_map(
            |((ts, level, message, template, exception), (fields, line, id, source, markers))| Record {
                id,
                ts,
                level: Level::ALL[level],
                message,
                template,
                exception,
                fields,
                source,
                line,
                markers,
            },
        )
}

fn same(a: &Record, b: &Record) -> bool {
    a.id == b.id
        && a.ts == b.ts
        && a.level == b.level
        && a.message == b.message
        && a.template == b.template
        && a.exception == b.exception
        && a.source == b.source
        && a.line == b.line
        && a.markers == b.markers
        && a.fields == b.fields
        && a.fields.keys().eq(b.fields.keys())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn records_survive_the_round_trip(records in prop::collection::vec(record(), 0..20)) {
        let decoded = decode(encode(&records, &[]).unwrap(), &[]).unwrap();
        prop_assert_eq!(decoded.records.len(), records.len());
        for (a, b) in records.iter().zip(&decoded.records) {
            prop_assert!(same(a, b), "{:?} != {:?}", a, b);
        }
    }

    #[test]
    fn extra_columns_survive_the_round_trip(records in prop::collection::vec(record(), 1..10), names in prop::collection::vec("\\PC{0,20}", 10)) {
        let rows: Vec<WithExtra<'_, 2>> = records
            .iter()
            .enumerate()
            .map(|(i, r)| WithExtra { record: r, extra: [names[i].as_str(), names[9 - i].as_str()] })
            .collect();
        let d = decode(encode(&rows, &["source_name", "group"]).unwrap(), &["source_name", "group"]).unwrap();
        for (i, r) in records.iter().enumerate() {
            prop_assert!(same(r, &d.records[i]));
            prop_assert_eq!(&d.extra[0][i], &names[i]);
            prop_assert_eq!(&d.extra[1][i], &names[9 - i]);
        }
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = decode(bytes::Bytes::from(bytes), &[]);
    }
}
