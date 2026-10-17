use imt_collect::*;
use std::collections::BTreeMap;

fn redactor(cfg: RedactConfig) -> Redactor {
    Redactor::new(&cfg).unwrap()
}

fn run(text: &str) -> (String, Counts) {
    let mut c = Counts::new();
    let out = redactor(RedactConfig::default()).apply(text, &mut c);
    (out, c)
}

#[test]
fn default_plan_is_valid_and_roundtrips() {
    let json = CollectPlan::default_json();
    let plan = CollectPlan::parse(&json).unwrap();
    assert_eq!(plan.name, "imt-diagnostics");
    assert_eq!(plan.event_logs, vec!["System", "Application"]);
    assert!(plan.environment && plan.redact.built_in);
    assert_eq!(plan.logs.len(), 1);
    assert!(CollectPlan::default().validate().is_ok());
    assert_eq!(CollectPlan::parse("{}").unwrap(), CollectPlan::default());
}

#[test]
fn plan_parses_toml_and_json() {
    let toml = r#"
name = "contoso"
registry = ["HKLM\\SOFTWARE\\Contoso"]
maxBundleMb = 10
[[logs]]
path = "/tmp/x"
include = ["*.log"]
maxAgeDays = 3
[redact]
builtIn = false
[[redact.patterns]]
name = "ticket"
regex = "TCK-\\d+"
"#;
    let p = CollectPlan::parse(toml).unwrap();
    assert_eq!(p.name, "contoso");
    assert_eq!(p.logs[0].max_age_days, Some(3));
    assert!(!p.redact.built_in);
    assert_eq!(p.redact.patterns[0].replace, "[REDACTED]");
    let j = CollectPlan::parse(r#"{"name":"j","logs":[{"path":"/a"}]}"#).unwrap();
    assert!(j.logs[0].include.contains(&"*.log".to_string()));
}

#[test]
fn plan_errors_are_specific() {
    let cases: Vec<(&str, &str)> = vec![
        ("{not json", "not valid JSON"),
        ("name = [", "not valid TOML"),
        (r#"{"bogus":1}"#, "unknown field"),
        (r#"{"name":" "}"#, "name must not be empty"),
        (r#"{"maxBundleMb":0}"#, "maxBundleMb"),
        (r#"{"commandTimeoutSecs":0}"#, "commandTimeoutSecs"),
        (r#"{"logs":[{"path":""}]}"#, "empty path"),
        (r#"{"logs":[{"path":"/a","include":["[bad"]}]}"#, "invalid glob"),
        (r#"{"eventLogs":["-x"]}"#, "invalid event log"),
        (r#"{"eventLogs":[""]}"#, "invalid event log"),
        (r#"{"registry":["SOFTWARE\\X"]}"#, "hive"),
        (r#"{"redact":{"patterns":[{"name":"","regex":"a"}]}}"#, "no name"),
        (r#"{"redact":{"patterns":[{"name":"x","regex":"("}]}}"#, "redaction pattern 'x'"),
    ];
    for (text, want) in cases {
        let e = CollectPlan::parse(text).unwrap_err().to_string();
        assert!(e.contains(want), "{text} -> {e}");
    }
}

#[test]
fn builtin_email_tokens_passwords() {
    let (o, c) = run("mail bob.smith+x@example.co.uk now");
    assert_eq!(o, "mail [email] now");
    assert_eq!(c["email"], 1);

    let (o, c) = run("Authorization: Bearer abcDEF123456.xyz");
    assert_eq!(o, "Authorization: Bearer [REDACTED]");
    assert_eq!(c["bearer-token"], 1);

    let jwt = "tok eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4 end";
    let (o, c) = run(jwt);
    assert_eq!(o, "tok [REDACTED] end");
    assert_eq!(c["jwt"], 1);

    let (o, c) = run(r#"{"apiKey": "sk-1234567890abcd", "x": 1} api_key=ABCDEFGH12345"#);
    assert_eq!(o, r#"{"apiKey": "[REDACTED]", "x": 1} api_key=[REDACTED]"#);
    assert_eq!(c["api-key"], 2);

    let (o, c) = run("login password=hunter22 pwd: secret1 \"password\":\"abc\" ok");
    assert_eq!(o, "login password=[REDACTED] pwd: [REDACTED] \"password\":\"[REDACTED]\" ok");
    assert_eq!(c["password"], 3);

    let (o, c) = run("Server=db;User Id=sa;Password=p@ss;AccountKey=abcd+/==;SharedAccessKey=zzz;");
    assert_eq!(
        o,
        "Server=db;User Id=sa;Password=[REDACTED];AccountKey=[REDACTED];SharedAccessKey=[REDACTED];"
    );
    assert_eq!(c["password"], 1);
    assert_eq!(c["connection-string"], 2);

    let (o, c) = run("nothing to see 12:30:45 std::string");
    assert_eq!(o, "nothing to see 12:30:45 std::string");
    assert!(c.is_empty());
}

#[test]
fn builtin_credit_cards_use_luhn() {
    let (o, c) = run("card 4111 1111 1111 1111 and 4111-1111-1111-1111 and 5500005555555559");
    assert_eq!(o, "card [REDACTED] and [REDACTED] and [REDACTED]");
    assert_eq!(c["credit-card"], 3);
    // Fails Luhn: left alone.
    let (o, c) = run("order 1234 5678 9012 3456 id 12345678901234");
    assert_eq!(o, "order 1234 5678 9012 3456 id 12345678901234");
    assert!(c.is_empty());
    assert!(luhn_ok("4111111111111111"));
    assert!(!luhn_ok("4111111111111112"));
    assert!(!luhn_ok("411111"), "too short");
    assert!(!luhn_ok(&"1".repeat(25)), "too long");
}

#[test]
fn builtin_ips_and_toggle() {
    let (o, c) = run("from 192.168.1.20 to 10.0.0.1 via 2001:db8:85a3:0:0:8a2e:370:7334 fe80::1 ::1 v 999.1.1.1");
    assert_eq!(o, "from [ipv4] to [ipv4] via [ipv6] [ipv6] [ipv6] v 999.1.1.1");
    assert_eq!(c["ipv4"], 2);
    assert_eq!(c["ipv6"], 3);

    let mut c = Counts::new();
    let r = redactor(RedactConfig {
        ip_addresses: false,
        ..RedactConfig::default()
    });
    assert_eq!(r.apply("host 10.0.0.1", &mut c), "host 10.0.0.1");
}

#[test]
fn custom_patterns_and_counts_accumulate() {
    let cfg = RedactConfig {
        built_in: false,
        ip_addresses: true,
        patterns: vec![
            CustomPattern {
                name: "ticket".into(),
                regex: r"TCK-(\d+)".into(),
                replace: "TCK-#$1".into(),
            },
            CustomPattern {
                name: "name".into(),
                regex: "Alice".into(),
                replace: "[user]".into(),
            },
        ],
    };
    let r = redactor(cfg);
    assert!(!r.is_empty());
    let mut c = Counts::new();
    assert_eq!(r.apply("TCK-12 by Alice, TCK-13", &mut c), "TCK-#12 by [user], TCK-#13");
    assert_eq!(c["ticket"], 2);
    r.apply("Alice", &mut c);
    assert_eq!(c["name"], 2);
    let none = redactor(RedactConfig {
        built_in: false,
        ip_addresses: true,
        patterns: vec![],
    });
    assert!(none.is_empty());
}

#[test]
fn bad_custom_pattern_is_an_error() {
    let cfg = RedactConfig {
        built_in: false,
        ip_addresses: true,
        patterns: vec![CustomPattern {
            name: "x".into(),
            regex: "(".into(),
            replace: String::new(),
        }],
    };
    assert!(Redactor::new(&cfg).is_err());
}

#[test]
fn safe_segment_cleans_names() {
    assert_eq!(safe_segment("a/b:c!d"), "a_b_c_d");
    assert_eq!(safe_segment("..."), "_");
    assert_eq!(safe_segment("My App"), "My App");
    let _: BTreeMap<String, usize> = Counts::new();
}

#[test]
fn json_redaction_keeps_numbers_and_structure() {
    let r = redactor(RedactConfig::default());
    let mut c = Counts::new();
    let v = serde_json::json!({"n": 4000577273856u64, "ok": true, "nil": null, "a": ["bob@x.org", {"k": "pwd=abcdefgh"}]});
    let out = r.apply_json(&v, &mut c);
    assert_eq!(out["n"], 4000577273856u64);
    assert_eq!(out["a"][0], "[email]");
    assert_eq!(out["a"][1]["k"], "pwd=[REDACTED]");
    assert_eq!(out["nil"], serde_json::Value::Null);
    assert_eq!(c.len(), 2);
}
