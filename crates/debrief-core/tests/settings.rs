//! Settings schema, validation and the plugin settings extension point.

use anyhow::{anyhow, Result};
use debrief_core::guard::QUARANTINE_AFTER;
use debrief_core::plugin::PluginStatus;
use debrief_core::settings::*;
use debrief_core::{Plugin, PluginRegistry};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn kinds_section() -> SettingsSection {
    SettingsSection::new("demo", "Demo", SectionOwner::Core)
        .describe("every kind")
        .field(SettingField::new("on", "On", FieldKind::Bool, true))
        .field(SettingField::new(
            "count",
            "Count",
            FieldKind::Int {
                min: 1,
                max: 9,
                step: 2,
                unit: Some("files".into()),
            },
            3,
        ))
        .field(SettingField::new(
            "ratio",
            "Ratio",
            FieldKind::Float {
                min: 0.0,
                max: 1.0,
                step: 0.1,
                unit: None,
            },
            0.5,
        ))
        .field(SettingField::new(
            "name",
            "Name",
            FieldKind::Text {
                max_len: 5,
                pattern: Some("[a-z]+".into()),
            },
            "abc",
        ))
        .field(SettingField::new(
            "free",
            "Free text",
            FieldKind::Text { max_len: 3, pattern: None },
            "",
        ))
        .field(SettingField::new(
            "level",
            "Level",
            FieldKind::Enum {
                options: vec![EnumOption::new("info", "Info"), EnumOption::new("debug", "Debug")],
            },
            "info",
        ))
        .field(SettingField::new("dir", "Folder", FieldKind::Path { dir: true }, ""))
        .field(SettingField::new(
            "timeout",
            "Timeout",
            FieldKind::Duration {
                min_ms: 1000,
                max_ms: 60_000,
            },
            5000,
        ))
        .field(SettingField::new(
            "size",
            "Size",
            FieldKind::Size {
                min_bytes: 1024,
                max_bytes: 1 << 20,
            },
            4096,
        ))
        .field(SettingField::new("tags", "Tags", FieldKind::StringList { max_items: 2 }, json!([])).as_advanced())
}

fn errors(values: Value) -> Vec<(String, String)> {
    validate(&kinds_section(), &values)
        .unwrap_err()
        .into_iter()
        .map(|e| (e.key, e.message))
        .collect()
}

fn one_error(values: Value) -> String {
    let e = errors(values);
    assert_eq!(e.len(), 1, "{e:?}");
    e[0].1.clone()
}

#[test]
fn the_demo_schema_is_valid_and_defaults_round_trip() {
    let s = kinds_section();
    assert_eq!(s.check(), Ok(()));
    let d = validate(&s, &json!({})).unwrap();
    assert_eq!(d, s.defaults());
    assert_eq!(validate(&s, &Value::Null).unwrap(), s.defaults());
    assert_eq!(d.to_json()["count"], 3);
    assert!(s.find("tags").unwrap().advanced);
    assert!(s.find("missing").is_none());
}

#[test]
fn unknown_keys_are_dropped_and_null_means_default() {
    let v = validate(&kinds_section(), &json!({ "bogus": 1, "count": null, "on": false })).unwrap();
    assert!(v.get("bogus").is_none());
    assert_eq!(v.int("count"), Some(3));
    assert_eq!(v.bool("on"), Some(false));
    assert_eq!(v.iter().count(), kinds_section().fields.len());
}

#[test]
fn non_objects_are_rejected_and_sanitized_to_defaults() {
    let e = validate(&kinds_section(), &json!([1, 2])).unwrap_err();
    assert_eq!(e[0].key, "");
    assert!(e[0].message.contains("object"));
    let (v, e) = sanitize(&kinds_section(), &json!("x"));
    assert_eq!((v, e.len()), (kinds_section().defaults(), 1));
}

#[test]
fn sanitize_keeps_good_values_and_defaults_bad_ones() {
    let (v, e) = sanitize(&kinds_section(), &json!({ "count": 5, "ratio": "x" }));
    assert_eq!(v.int("count"), Some(5));
    assert_eq!(v.float("ratio"), Some(0.5));
    assert_eq!(
        e,
        vec![FieldError {
            key: "ratio".into(),
            message: "Must be a number.".into()
        }]
    );
}

#[test]
fn every_error_is_reported() {
    let e = errors(json!({ "on": 1, "count": 100, "level": "x" }));
    let keys: Vec<&str> = e.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["on", "count", "level"]);
}

#[test]
fn bool_fields() {
    assert!(one_error(json!({ "on": "yes" })).contains("true or false"));
    assert_eq!(validate(&kinds_section(), &json!({ "on": false })).unwrap().bool("on"), Some(false));
}

#[test]
fn int_fields() {
    assert_eq!(one_error(json!({ "count": "3" })), "Must be a whole number.");
    assert_eq!(one_error(json!({ "count": 2.5 })), "Must be a whole number.");
    assert_eq!(one_error(json!({ "count": 0 })), "Must be between 1 files and 9 files.");
    assert_eq!(one_error(json!({ "count": 11 })), "Must be between 1 files and 9 files.");
    assert_eq!(one_error(json!({ "count": 4 })), "Must be 1 plus a multiple of 2.");
    let v = validate(&kinds_section(), &json!({ "count": 9.0 })).unwrap();
    assert_eq!(v.get("count"), Some(&json!(9)));
}

#[test]
fn float_fields() {
    assert_eq!(one_error(json!({ "ratio": true })), "Must be a number.");
    assert_eq!(one_error(json!({ "ratio": 1.5 })), "Must be between 0 and 1.");
    assert_eq!(one_error(json!({ "ratio": -0.1 })), "Must be between 0 and 1.");
    assert_eq!(
        validate(&kinds_section(), &json!({ "ratio": 0.25 })).unwrap().float("ratio"),
        Some(0.25)
    );
}

#[test]
fn text_fields() {
    assert_eq!(one_error(json!({ "name": 5 })), "Must be text.");
    assert_eq!(one_error(json!({ "name": "abcdef" })), "Must be at most 5 characters.");
    assert_eq!(one_error(json!({ "name": "AB" })), "Does not have the expected format ([a-z]+).");
    // The pattern must match the whole value, not a part of it.
    assert!(one_error(json!({ "name": "ab1" })).contains("expected format"));
    assert_eq!(one_error(json!({ "free": "four" })), "Must be at most 3 characters.");
    // Characters, not bytes.
    let v = validate(&kinds_section(), &json!({ "free": "äöü", "name": "xyz" })).unwrap();
    assert_eq!((v.text("free"), v.text("name")), (Some("äöü"), Some("xyz")));
}

#[test]
fn text_with_a_broken_pattern_reports_the_schema_problem() {
    let field = SettingField::new(
        "t",
        "T",
        FieldKind::Text {
            max_len: 10,
            pattern: Some("(".into()),
        },
        "",
    );
    assert!(validate_field(&field, &json!("a")).unwrap_err().contains("pattern is invalid"));
}

#[test]
fn enum_fields() {
    assert_eq!(one_error(json!({ "level": "trace" })), "Must be one of: info, debug.");
    assert_eq!(one_error(json!({ "level": 1 })), "Must be one of: info, debug.");
    assert_eq!(
        validate(&kinds_section(), &json!({ "level": "debug" })).unwrap().text("level"),
        Some("debug")
    );
}

#[test]
fn path_fields() {
    let abs = if cfg!(windows) { "C:\\logs" } else { "/var/log" };
    assert_eq!(validate(&kinds_section(), &json!({ "dir": abs })).unwrap().text("dir"), Some(abs));
    assert_eq!(one_error(json!({ "dir": 3 })), "Must be a path.");
    assert_eq!(one_error(json!({ "dir": "relative/dir" })), "Must be an absolute path (or empty).");
    assert_eq!(one_error(json!({ "dir": "/a\u{0}b" })), "Must not contain NUL characters.");
    assert!(one_error(json!({ "dir": format!("/{}", "a".repeat(MAX_PATH_LEN)) })).contains("at most"));
}

#[test]
fn duration_fields() {
    let v = validate(&kinds_section(), &json!({ "timeout": 2000 })).unwrap();
    assert_eq!(v.duration("timeout"), Some(Duration::from_secs(2)));
    assert_eq!(one_error(json!({ "timeout": -5 })), "Must be a duration in whole milliseconds.");
    assert_eq!(one_error(json!({ "timeout": "1s" })), "Must be a duration in whole milliseconds.");
    assert_eq!(one_error(json!({ "timeout": 999 })), "Must be between 1 s and 1 min.");
    assert_eq!(one_error(json!({ "timeout": 60_001 })), "Must be between 1 s and 1 min.");
}

#[test]
fn size_fields() {
    assert_eq!(
        validate(&kinds_section(), &json!({ "size": 2048 })).unwrap().bytes("size"),
        Some(2048)
    );
    assert_eq!(one_error(json!({ "size": -1 })), "Must be a size in whole bytes.");
    assert_eq!(one_error(json!({ "size": 1023 })), "Must be between 1 KB and 1 MB.");
    assert_eq!(one_error(json!({ "size": (1 << 20) + 1 })), "Must be between 1 KB and 1 MB.");
}

#[test]
fn string_list_fields() {
    let v = validate(&kinds_section(), &json!({ "tags": ["a", "b"] })).unwrap();
    assert_eq!(v.strings("tags"), Some(vec!["a".to_string(), "b".to_string()]));
    assert_eq!(one_error(json!({ "tags": "a" })), "Must be a list of text.");
    assert_eq!(one_error(json!({ "tags": ["a", "b", "c"] })), "Must have at most 2 items.");
    assert_eq!(one_error(json!({ "tags": ["a", 2] })), "Item 2 must be text.");
    assert!(one_error(json!({ "tags": ["x".repeat(MAX_ITEM_LEN + 1)] })).starts_with("Item 1 must be at most"));
}

#[test]
fn typed_accessors_return_none_for_other_types_and_missing_keys() {
    let v = kinds_section().defaults();
    assert_eq!(v.bool("count"), None);
    assert_eq!(v.int("missing"), None);
    assert_eq!(v.text("count"), None);
    assert_eq!(v.duration("name"), None);
    assert_eq!(v.bytes("name"), None);
    assert_eq!(v.strings("name"), None);
    assert_eq!(v.float("missing"), None);
}

fn with_field(kind: FieldKind, default: Value) -> SettingsSection {
    SettingsSection::new("s", "S", SectionOwner::App).field(SettingField::new("k", "K", kind, default))
}

#[test]
fn schema_checks_catch_inconsistent_declarations() {
    let bad = |s: SettingsSection| s.check().unwrap_err();
    assert!(bad(SettingsSection::new("", "x", SectionOwner::Core)).contains("section id"));
    assert!(bad(SettingsSection::new("a b", "x", SectionOwner::Core)).contains("section id"));
    assert!(bad(SettingsSection::new("x".repeat(200), "x", SectionOwner::Core)).contains("section id"));
    let dup = SettingsSection::new("s", "S", SectionOwner::Core)
        .field(SettingField::new("a", "A", FieldKind::Bool, true))
        .field(SettingField::new("a", "A", FieldKind::Bool, true));
    assert!(bad(dup).contains("declared twice"));
    assert!(
        bad(SettingsSection::new("s", "S", SectionOwner::Core).field(SettingField::new("a/b", "A", FieldKind::Bool, true)))
            .contains("field key")
    );
    let mut many = SettingsSection::new("s", "S", SectionOwner::Core);
    for i in 0..=MAX_FIELDS {
        many = many.field(SettingField::new(format!("f{i}"), "F", FieldKind::Bool, true));
    }
    assert!(bad(many).contains("the limit is"));
    let int = |min, max, step| FieldKind::Int {
        min,
        max,
        step,
        unit: None,
    };
    assert!(bad(with_field(int(5, 1, 1), json!(1))).contains("min <= max"));
    assert!(bad(with_field(int(1, 5, 0), json!(1))).contains("step >= 1"));
    let float = |min: f64, max: f64, step: f64| FieldKind::Float {
        min,
        max,
        step,
        unit: None,
    };
    assert!(bad(with_field(float(1.0, 0.0, 0.1), json!(0.5))).contains("finite"));
    assert!(bad(with_field(float(0.0, f64::INFINITY, 0.1), json!(0.5))).contains("finite"));
    assert!(bad(with_field(float(0.0, 1.0, 0.0), json!(0.5))).contains("step > 0"));
    let text = |max_len, pattern: Option<&str>| FieldKind::Text {
        max_len,
        pattern: pattern.map(String::from),
    };
    assert!(bad(with_field(text(0, None), json!(""))).contains("max_len"));
    assert!(bad(with_field(text(MAX_TEXT_LEN + 1, None), json!(""))).contains("max_len"));
    assert!(bad(with_field(text(5, Some("(")), json!(""))).contains("pattern"));
    assert!(bad(with_field(FieldKind::Enum { options: vec![] }, json!("a"))).contains("at least one option"));
    let twice = vec![EnumOption::new("a", "A"), EnumOption::new("a", "B")];
    assert!(bad(with_field(FieldKind::Enum { options: twice }, json!("a"))).contains("unique"));
    assert!(bad(with_field(FieldKind::Duration { min_ms: 2, max_ms: 1 }, json!(1))).contains("min_ms"));
    assert!(bad(with_field(
        FieldKind::Size {
            min_bytes: 2,
            max_bytes: 1
        },
        json!(1)
    ))
    .contains("min_bytes"));
    assert!(bad(with_field(FieldKind::StringList { max_items: 0 }, json!([]))).contains("max_items"));
    assert!(bad(with_field(
        FieldKind::StringList {
            max_items: MAX_LIST_ITEMS + 1
        },
        json!([])
    ))
    .contains("max_items"));
    // Defaults must pass their own validation.
    assert!(bad(with_field(int(1, 5, 1), json!(9))).contains("default 9"));
    assert_eq!(with_field(FieldKind::Path { dir: false }, json!("")).check(), Ok(()));
}

#[test]
fn sections_serialize_in_the_wire_shape() {
    let section = SettingsSection::new("p", "P", SectionOwner::Plugin { id: "x.y".into() }).field(
        SettingField::new("t", "T", FieldKind::Duration { min_ms: 1, max_ms: 2 }, 1)
            .describe("help")
            .needs_restart(),
    );
    let v = serde_json::to_value(&section).unwrap();
    assert_eq!(v["owner"], json!({ "kind": "plugin", "id": "x.y" }));
    assert_eq!(v["fields"][0]["kind"], json!({ "type": "duration", "minMs": 1, "maxMs": 2 }));
    assert_eq!(v["fields"][0]["restartRequired"], true);
    assert_eq!(v["fields"][0]["description"], "help");
    assert_eq!(serde_json::to_value(SectionOwner::App).unwrap(), json!({ "kind": "app" }));
    let list = serde_json::to_value(FieldKind::StringList { max_items: 3 }).unwrap();
    assert_eq!(list, json!({ "type": "stringList", "maxItems": 3 }));
    let fe = serde_json::to_value(FieldError {
        key: "k".into(),
        message: "m".into(),
    })
    .unwrap();
    assert_eq!(fe, json!({ "key": "k", "message": "m" }));
}

// ---- the extension point ----

/// Settings shared by a plugin's configurable and its contributions.
#[derive(Default)]
struct Knobs {
    timeout_ms: AtomicU64,
}

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Fine,
    PanicOnApply,
    PanicOnDescribe,
    PanicOnConfigurable,
    InvalidSchema,
    NoSettings,
    Untitled,
}

struct TestPlugin {
    id: &'static str,
    behaviour: Behaviour,
    knobs: Arc<Knobs>,
}

struct TestSettings {
    behaviour: Behaviour,
    knobs: Arc<Knobs>,
}

impl Configurable for TestSettings {
    fn settings_section(&self) -> SettingsSection {
        if self.behaviour == Behaviour::PanicOnDescribe {
            panic!("cannot describe");
        }
        let default = if self.behaviour == Behaviour::InvalidSchema { 1 } else { 5000 };
        let title = if self.behaviour == Behaviour::Untitled {
            " "
        } else {
            "Test plugin"
        };
        // The id and owner given here are replaced by the plugin's.
        SettingsSection::new("spoofed", title, SectionOwner::Core).field(SettingField::new(
            "timeout",
            "Timeout",
            FieldKind::Duration {
                min_ms: 1000,
                max_ms: 10_000,
            },
            default,
        ))
    }

    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        if self.behaviour == Behaviour::PanicOnApply {
            panic!("apply exploded");
        }
        let ms = values.duration("timeout").ok_or_else(|| anyhow!("no timeout"))?.as_millis() as u64;
        if ms == 9999 {
            return Err(anyhow!("refused 9999"));
        }
        self.knobs.timeout_ms.store(ms, Ordering::SeqCst);
        Ok(())
    }
}

impl Plugin for TestPlugin {
    fn id(&self) -> &'static str {
        self.id
    }
    fn name(&self) -> &'static str {
        "Test"
    }
    fn version(&self) -> &'static str {
        "1.0"
    }
    fn register(&self, _registry: &mut PluginRegistry) -> Result<()> {
        Ok(())
    }
    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        match self.behaviour {
            Behaviour::NoSettings => None,
            Behaviour::PanicOnConfigurable => panic!("no settings today"),
            b => Some(Arc::new(TestSettings {
                behaviour: b,
                knobs: self.knobs.clone(),
            })),
        }
    }
}

fn plugin(id: &'static str, behaviour: Behaviour) -> TestPlugin {
    TestPlugin {
        id,
        behaviour,
        knobs: Arc::default(),
    }
}

fn values(section: &SettingsSection, v: Value) -> ValidValues {
    validate(section, &v).unwrap()
}

#[test]
fn plugins_without_settings_contribute_none() {
    let mut r = PluginRegistry::empty();
    r.install(&plugin("t.none", Behaviour::NoSettings)).unwrap();
    assert!(r.plugin_settings().is_empty());
    assert!(r.plugins()[0].contributes.is_empty());
}

#[test]
fn an_active_plugin_applies_its_settings() {
    let p = plugin("t.fine", Behaviour::Fine);
    let mut r = PluginRegistry::empty();
    r.install(&p).unwrap();
    assert_eq!(r.plugins()[0].contributes, ["settings"]);
    let all = r.plugin_settings();
    assert_eq!(all.len(), 1);
    let s = &all[0];
    let section = s.configurable.settings_section();
    assert_eq!(section.id, "t.fine");
    assert_eq!(section.owner, SectionOwner::Plugin { id: "t.fine".into() });
    assert_eq!(section.title, "Test plugin");
    assert!(!s.read_only());
    assert_eq!(s.note(), None);
    s.apply(&values(&section, json!({ "timeout": 2000 }))).unwrap();
    assert_eq!(p.knobs.timeout_ms.load(Ordering::SeqCst), 2000);
    // An ordinary error is passed through and is not a crash.
    let e = s.apply(&values(&section, json!({ "timeout": 9999 }))).unwrap_err();
    assert_eq!(e.to_string(), "refused 9999");
    assert_eq!(r.plugins()[0].failures, 0);
}

#[test]
fn an_untitled_section_takes_the_plugin_name() {
    let mut r = PluginRegistry::empty();
    r.install(&plugin("t.untitled", Behaviour::Untitled)).unwrap();
    assert_eq!(r.plugin_settings()[0].configurable.settings_section().title, "Test");
}

#[test]
fn a_panicking_apply_is_contained_counted_and_leads_to_quarantine() {
    let mut r = PluginRegistry::empty();
    r.install(&plugin("t.boom", Behaviour::PanicOnApply)).unwrap();
    let s = r.plugin_settings().remove(0);
    let section = s.configurable.settings_section();
    let v = section.defaults();
    for i in 1..=QUARANTINE_AFTER {
        let e = s.configurable.apply_settings(&v).unwrap_err().to_string();
        assert_eq!(e, "plugin \"t.boom\" (apply_settings) crashed: apply exploded");
        assert_eq!(r.plugins()[0].failures, i);
    }
    assert_eq!(r.plugins()[0].status, PluginStatus::Quarantined);
    // Quarantined: the section is still listed, read-only, and plugin code is not called.
    let s = r.plugin_settings().remove(0);
    assert!(s.read_only());
    assert!(s.note().unwrap().contains("repeated crashes"));
    assert_eq!(s.configurable.settings_section(), section);
    assert!(s.apply(&v).unwrap_err().to_string().contains("repeated crashes"));
    let e = s.configurable.apply_settings(&v).unwrap_err().to_string();
    assert!(e.contains("disabled after repeated crashes"), "{e}");
    assert_eq!(r.plugins()[0].failures, QUARANTINE_AFTER);
    // Lifting the quarantine makes it editable again.
    r.reset_plugin("t.boom");
    assert!(!r.plugin_settings()[0].read_only());
}

#[test]
fn a_disabled_plugin_shows_its_section_read_only() {
    let p = plugin("t.off", Behaviour::Fine);
    let mut r = PluginRegistry::empty();
    r.disable(["t.off"]);
    r.install(&p).unwrap();
    let s = r.plugin_settings().remove(0);
    assert_eq!(s.plugin.status, PluginStatus::Disabled);
    assert!(s.read_only());
    assert!(s.note().unwrap().contains("disabled"));
    assert_eq!(s.configurable.settings_section().fields.len(), 1);
    let v = s.configurable.settings_section().defaults();
    assert!(s.apply(&v).is_err());
    assert_eq!(p.knobs.timeout_ms.load(Ordering::SeqCst), 0);
}

#[test]
fn panics_while_describing_give_an_empty_section_that_says_why() {
    let mut r = PluginRegistry::empty();
    r.install(&plugin("t.describe", Behaviour::PanicOnDescribe)).unwrap();
    r.install(&plugin("t.configurable", Behaviour::PanicOnConfigurable)).unwrap();
    for (s, why) in r.plugin_settings().iter().zip(["cannot describe", "no settings today"]) {
        let section = s.configurable.settings_section();
        assert!(section.fields.is_empty());
        assert!(
            section.description.starts_with("Settings are unavailable"),
            "{}",
            section.description
        );
        assert!(section.description.contains(why));
        assert_eq!(section.title, "Test");
        assert_eq!(s.plugin.failures, 1);
        // Applying the (empty) defaults to the stand-in is harmless.
        assert!(s.apply(&section.defaults()).is_ok());
    }
}

#[test]
fn an_invalid_schema_is_reported_without_counting_a_crash() {
    let mut r = PluginRegistry::empty();
    r.install(&plugin("t.schema", Behaviour::InvalidSchema)).unwrap();
    let s = r.plugin_settings().remove(0);
    let section = s.configurable.settings_section();
    assert!(section.fields.is_empty());
    assert!(section.description.contains("invalid settings schema"), "{}", section.description);
    assert_eq!(s.plugin.failures, 0);
}

#[test]
fn a_rejected_plugin_contributes_no_settings() {
    struct Failing;
    impl Plugin for Failing {
        fn id(&self) -> &'static str {
            "t.failing"
        }
        fn name(&self) -> &'static str {
            "F"
        }
        fn version(&self) -> &'static str {
            "1"
        }
        fn register(&self, _r: &mut PluginRegistry) -> Result<()> {
            Err(anyhow!("nope"))
        }
        fn configurable(&self) -> Option<Arc<dyn Configurable>> {
            Some(Arc::new(TestSettings {
                behaviour: Behaviour::Fine,
                knobs: Arc::default(),
            }))
        }
    }
    let mut r = PluginRegistry::empty();
    assert!(r.install(&Failing).is_err());
    assert!(r.plugin_settings().is_empty());
}

#[test]
fn a_panicking_apply_quarantines_the_plugin_in_the_workspace_health() {
    use debrief_core::{LogStore, Profile, Workspace};
    let mut r = PluginRegistry::empty();
    r.install(&plugin("t.ws", Behaviour::PanicOnApply)).unwrap();
    let ws: Box<dyn LogStore> = Box::new(Workspace::with_registry(Profile::default_profile(), r).unwrap());
    let settings = ws.plugin_settings();
    assert_eq!(settings.len(), 1);
    let section = settings[0].configurable.settings_section();
    let v = values(&section, json!({}));
    for _ in 0..QUARANTINE_AFTER {
        assert!(settings[0].apply(&v).unwrap_err().to_string().contains("apply exploded"));
    }
    // The workspace itself (not a second registry) saw the crashes.
    let info = ws.summary().plugins.into_iter().find(|p| p.id == "t.ws").unwrap();
    assert_eq!((info.status, info.failures), (PluginStatus::Quarantined, QUARANTINE_AFTER));
    assert!(info.last_error.unwrap().contains("(apply_settings) crashed: apply exploded"));
    let again = ws.plugin_settings();
    assert!(again[0].read_only() && again[0].apply(&v).is_err());
    ws.reset_plugin("t.ws");
    assert_eq!(ws.summary().plugins[0].status, PluginStatus::Active);
}
