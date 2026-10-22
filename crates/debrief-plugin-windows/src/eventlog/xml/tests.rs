use super::*;
use debrief_plugin_api::model::Level;
use evtx::{EvtxParser, ParserSettings};
use serde_json::json;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!("../../../tests/fixtures/eventlog/", $name))
    };
}

const SECURITY_4672: &str = fixture!("security-4672.xml");
const WER_1001: &str = fixture!("wer-1001.xml");
const SCM_7026: &str = fixture!("scm-7026.xml");
const UNNAMED: &str = fixture!("classic-unnamed.xml");
const MULTI: &str = fixture!("classic-multi.xml");
const POWER: &str = fixture!("kernel-power-566.xml");
const TIME: &str = fixture!("kernel-general-1.xml");
const USERDATA: &str = fixture!("userdata-104.xml");

#[test]
fn recorded_events_map_to_the_importer_fields() {
    let r = record_from_xml(SCM_7026, Some(fixture!("scm-7026.message.txt")), 3).unwrap();
    assert_eq!(r.line, 3);
    assert_eq!(r.fields["Provider"], "Service Control Manager");
    assert_eq!(r.fields["EventId"], 7026);
    assert_eq!(r.fields["RecordId"], 14591);
    assert_eq!(r.fields["Channel"], "System");
    assert_eq!(r.fields["Computer"], "WIN-FIXTURE");
    assert_eq!(r.fields["ProcessId"], 1116);
    assert_eq!(r.fields["ThreadId"], 1120);
    assert_eq!(r.fields["Task"], 0);
    assert_eq!(r.fields["Keywords"], "0x8080000000000000");
    assert!(!r.fields.contains_key("UserId"), "<Security/> has no user");
    assert_eq!(r.fields["param1"], "\r\ndam\r\nuiomap", "CR LF kept as rendered");
    assert_eq!(r.level, Level::Information);
    assert_eq!(
        r.ts,
        Some(
            chrono::DateTime::parse_from_rfc3339("2026-10-17T13:26:31.172Z")
                .unwrap()
                .timestamp_millis()
        )
    );
    assert!(r
        .message
        .starts_with("The following boot-start or system-start driver(s) did not load:"));
    assert!(r.message.ends_with("uiomap"), "trailing CR LF trimmed: {:?}", r.message);

    let r = record_from_xml(SECURITY_4672, None, 1).unwrap();
    assert_eq!(r.fields["Channel"], "Security");
    assert_eq!(r.fields["Provider"], "Microsoft-Windows-Security-Auditing");
    assert_eq!(r.fields["EventId"], 4672);
    assert_eq!(r.fields["SubjectUserName"], "SYSTEM");
    assert_eq!(r.fields["SubjectLogonId"], "0x3e7");
    assert!(r.fields["PrivilegeList"].as_str().unwrap().contains("SeDebugPrivilege"));
    // Level 0 ("log always") is information, as in the importer.
    assert_eq!(r.level, Level::Information);
    // No message table: the importer's message from the event data.
    assert!(
        r.message
            .starts_with("Microsoft-Windows-Security-Auditing 4672: SubjectUserSid=S-1-5-18"),
        "{}",
        r.message
    );

    let r = record_from_xml(WER_1001, Some(fixture!("wer-1001.message.txt")), 1).unwrap();
    assert_eq!(r.fields["Bucket"], "", "empty named Data stays an empty value");
    assert_eq!(r.fields["P1"], "setup.exe");
    assert_eq!(r.fields["ReportStatus"], 268697600);
    assert_eq!(r.fields["UserId"], "S-1-5-18");
    assert!(r.message.starts_with("Fault bucket , type 0"));

    let r = record_from_xml(POWER, None, 1).unwrap();
    assert_eq!(
        (r.fields["Provider"].as_str(), r.fields["EventId"].as_i64()),
        (Some("Microsoft-Windows-Kernel-Power"), Some(566))
    );
    assert_eq!(r.fields["BootId"], 84);
    assert_eq!(r.fields["Task"], 268);

    let r = record_from_xml(TIME, None, 1).unwrap();
    assert_eq!(r.fields["EventId"], 1);
    // EventData.ProcessID differs in case from the System ProcessId, so both stay.
    assert_eq!(r.fields["ProcessID"], 11700);
    assert_eq!(r.fields["TimeZoneBias"], -480);
}

#[test]
fn unnamed_data_and_user_data() {
    let r = record_from_xml(UNNAMED, Some(fixture!("classic-unnamed.message.txt")), 1).unwrap();
    assert_eq!(r.fields["Data"], "guest-ping called");
    assert_eq!(r.message, "guest-ping called");

    let r = record_from_xml(MULTI, None, 1).unwrap();
    assert_eq!(r.fields["Provider"], "Debrief Test & Co");
    assert_eq!(r.level, Level::Error);
    assert_eq!(r.fields["EventId"], 1000);
    // Several unnamed values: one comma-joined Data value, like the importer.
    assert_eq!(r.fields["Data"], "first <one>,second,");
    assert_eq!(r.fields["Binary"], "DEADBEEF");
    assert_eq!(
        r.ts,
        Some(
            chrono::DateTime::parse_from_rfc3339("2026-10-17T09:00:01.5Z")
                .unwrap()
                .timestamp_millis()
        )
    );

    let r = record_from_xml(USERDATA, None, 1).unwrap();
    assert_eq!(r.fields["EventId"], 104);
    assert_eq!(r.fields["SubjectUserName"], "admin");
    assert_eq!(r.fields["Channel"], "System");
    assert_eq!(r.fields["EventData.Channel"], "System", "a UserData name that clashes is prefixed");
    assert!(!r.fields.contains_key("BackupPath"), "empty elements carry no value");
}

#[test]
fn json_shape_matches_the_evtx_crate() {
    let j = event_json(SCM_7026).unwrap();
    let system = &j["Event"]["System"];
    assert_eq!(system["EventID"], json!({"#attributes": {"Qualifiers": 49152}, "#text": 7026}));
    assert_eq!(system["Correlation"], Value::Null);
    assert_eq!(system["Execution"], json!({"#attributes": {"ProcessID": 1116, "ThreadID": 1120}}));
    assert_eq!(
        system["Provider"]["#attributes"]["Name"], "Service Control Manager",
        "xmlns attributes are dropped, the rest kept"
    );
    assert_eq!(record_id(&j), Some(14591));
    let j = event_json(MULTI).unwrap();
    assert_eq!(j["Event"]["EventData"]["Data"], json!({"#text": ["first <one>", "second", ""]}));
    // Repeated elements get `_N` suffixes.
    let j = event_json("<Event><System/><UserData><A>1</A><A>2</A><A x='1'/></UserData></Event>").unwrap();
    assert_eq!(j["Event"]["UserData"], json!({"A": 1, "A_1": 2, "A_2": {"#attributes": {"x": 1}}}));
    assert_eq!(record_id(&json!({"Event": {"System": {"EventRecordID": "12"}}})), Some(12));
    assert_eq!(record_id(&json!({})), None);
}

/// The evtx crate renders the same events as XML and as JSON: mapping its
/// XML through this module gives the importer's records exactly.
#[test]
fn parity_with_the_evtx_importer_on_the_sample_log() {
    let bytes = include_bytes!("../../../tests/fixtures/security.evtx").to_vec();
    let settings = ParserSettings::new().validate_checksums(false);
    let mut xml_parser = EvtxParser::from_buffer(bytes.clone()).unwrap().with_configuration(settings.clone());
    let mut json_parser = EvtxParser::from_buffer(bytes).unwrap().with_configuration(settings);
    let xml: Vec<_> = xml_parser.records().flatten().collect();
    let json: Vec<_> = json_parser.records_json_value().flatten().collect();
    assert_eq!(xml.len(), json.len());
    assert!(!xml.is_empty());
    for (x, j) in xml.iter().zip(&json) {
        let ours = record_from_xml(&x.data, None, 1).unwrap();
        // The sample's record headers are numbered apart from `EventRecordID`; live events carry only the latter.
        let theirs = to_record(&j.data, record_id(&j.data).unwrap_or(j.event_record_id), 1);
        assert_eq!(ours.ts, theirs.ts);
        assert_eq!(ours.level, theirs.level);
        assert_eq!(ours.message, theirs.message, "record {}", j.event_record_id);
        assert_eq!(ours.fields, theirs.fields, "record {}", j.event_record_id);
    }
}

#[test]
fn provider_names_and_messages() {
    assert_eq!(provider_name(SECURITY_4672).as_deref(), Some("Microsoft-Windows-Security-Auditing"));
    assert_eq!(provider_name(MULTI).as_deref(), Some("Debrief Test & Co"));
    assert_eq!(provider_name("<Event><System/></Event>"), None);
    assert_eq!(provider_name("<<<"), None);
    assert_eq!(provider_name(&"<a>".repeat(100)), None);
    assert_eq!(clean_message("\r\nHello.\r\n\r\n\0").as_deref(), Some("Hello."));
    assert_eq!(clean_message(" \r\n"), None);
    let long = clean_message(&"x".repeat(MAX_MESSAGE_CHARS + 5)).unwrap();
    assert_eq!(long.chars().count(), MAX_MESSAGE_CHARS);
    assert!(long.ends_with('\u{2026}'));
    // A blank formatted message falls back to the event data.
    let r = record_from_xml(UNNAMED, Some("  "), 1).unwrap();
    assert_eq!(r.message, "qemu-ga 1: Data=guest-ping called");
}

#[test]
fn bad_xml_is_an_error_not_a_panic() {
    for (bad, why) in [
        ("", "ends before"),
        ("<Event>", "ends before"),
        ("<Event></Other>", "malformed"),
        ("</Event>", "malformed"),
        ("<Other/>", "not an event"),
        ("<Event><System><a></System></Event>", "malformed"),
    ] {
        let e = event_json(bad).unwrap_err().to_string();
        assert!(e.contains(why), "{bad:?}: {e}");
    }
    let deep = format!("<Event>{}{}</Event>", "<a>".repeat(40), "</a>".repeat(40));
    assert!(event_json(&deep).unwrap_err().to_string().contains("deeper"));
    let deep_empty = format!("<Event>{}<b/>{}</Event>", "<a>".repeat(31), "</a>".repeat(31));
    assert!(event_json(&deep_empty).unwrap_err().to_string().contains("deeper"));
    let many = format!("<Event>{}</Event>", "<a/>".repeat(MAX_ELEMENTS + 1));
    assert!(event_json(&many).unwrap_err().to_string().contains("more than"));
    let many_open = format!("<Event>{}</Event>", "<a></a>".repeat(MAX_ELEMENTS + 1));
    assert!(event_json(&many_open).unwrap_err().to_string().contains("more than"));
    let huge = format!("<Event>{}</Event>", "x".repeat(MAX_XML_BYTES));
    assert!(event_json(&huge).unwrap_err().to_string().contains("larger"));
    // Entities, character references, CDATA and namespaced names.
    let j = event_json(
        "<e:Event xmlns:e='x'><System/><EventData><Data Name='a'>&#65;&unknown;<![CDATA[<b>]]>&gt;</Data></EventData></e:Event>",
    )
    .unwrap();
    assert_eq!(j["Event"]["EventData"]["a"], "A<b>>");
    // Truncated real events.
    for cut in (0..SCM_7026.len()).step_by(37) {
        let _ = record_from_xml(&SCM_7026[..cut], None, 1);
    }
}
