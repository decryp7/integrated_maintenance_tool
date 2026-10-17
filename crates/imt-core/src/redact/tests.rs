use super::*;
use serde_json::json;

fn only(d: Detector) -> Redactor {
    Redactor::new(
        vec![Rule::detector(d, Replacement::Mode(Mode::Mask))],
        FieldScope::All,
        Salt::fixed([1; 16]),
    )
}

fn masked(d: Detector, text: &str) -> String {
    only(d).apply(text, &mut Counts::new()).into_owned()
}

/// Positives are masked (`<label>`), negatives stay untouched.
fn check(d: Detector, positives: &[(&str, &str)], negatives: &[&str]) {
    for (input, want) in positives {
        assert_eq!(masked(d, input), *want, "{} on {input:?}", d.name());
    }
    for input in negatives {
        assert_eq!(masked(d, input), *input, "{} must not match {input:?}", d.name());
    }
}

#[test]
fn bearer_tokens() {
    check(
        Detector::BearerToken,
        &[
            ("Authorization: Bearer abcdefgh1234", "Authorization: Bearer <token>"),
            ("bearer A.b-c_d~e+f/g=", "bearer <token>"),
        ],
        &["Bearer short", "the bearer of news"],
    );
}

#[test]
fn jwts() {
    check(
        Detector::Jwt,
        &[("t=eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.sig-_x end", "t=<token> end")],
        &["eyJshort.x.y", "eyJ is a prefix"],
    );
}

#[test]
fn api_keys() {
    check(
        Detector::ApiKey,
        &[
            ("api_key=abcd1234efgh", "api_key=<secret>"),
            (r#"{"apiKey": "ABCDEFGH12"}"#, r#"{"apiKey": "<secret>"}"#),
            ("client-secret: s3cr3t-value", "client-secret: <secret>"),
        ],
        &["api_key=short", "token count 5"],
    );
}

#[test]
fn connection_strings() {
    check(
        Detector::ConnectionString,
        &[(
            "DefaultEndpointsProtocol=https;AccountKey=abc+/=;SharedAccessKey=xyz",
            "DefaultEndpointsProtocol=https;AccountKey=<secret>;SharedAccessKey=<secret>",
        )],
        &["Server=db;Database=orders"],
    );
}

#[test]
fn passwords() {
    check(
        Detector::Password,
        &[
            ("login password=hunter2 ok", "login password=<password> ok"),
            ("Pwd: x;", "Pwd: <password>;"),
            (r#""passwd":"p@ss""#, r#""passwd":"<password>""#),
        ],
        &["password reset requested", "forgot your password?"],
    );
}

#[test]
fn user_paths() {
    check(
        Detector::UserPath,
        &[
            (r"C:\Users\alice\AppData\x.log", r"C:\Users\<user>\AppData\x.log"),
            (r"d:\documents and settings\bob\x", r"d:\documents and settings\<user>\x"),
            ("/home/carol/.config", "/home/<user>/.config"),
            ("open /Users/dave/Library", "open /Users/<user>/Library"),
        ],
        &["/var/log/home/x", "/opt/app/Users/x", r"C:\Program Files\x"],
    );
}

#[test]
fn emails() {
    check(
        Detector::Email,
        &[("mail jane.doe+x@mail.example.co.uk now", "mail <email> now")],
        &["user@localhost", "@handle", "a@b"],
    );
}

#[test]
fn credit_cards_are_luhn_checked() {
    check(
        Detector::CreditCard,
        &[
            ("card 4111 1111 1111 1111 ok", "card <card> ok"),
            ("5500-0000-0000-0004", "<card>"),
            ("378282246310005", "<card>"),
        ],
        &["4111 1111 1111 1112", "order 1234567890123", "123"],
    );
    assert!(!luhn_ok("79927398713"), "too short for a card");
    assert!(!luhn_ok(&"1".repeat(20)));
}

#[test]
fn ip_addresses() {
    check(
        Detector::Ipv4,
        &[("from 10.0.0.1 to 192.168.255.254", "from <ipv4> to <ipv4>")],
        &["999.1.1.1", "version 1.2.3", "256.10.10.10"],
    );
    check(
        Detector::Ipv6,
        &[
            ("via fe80:0:0:0:0:0:0:1 x", "via <ipv6> x"),
            ("2001:db8::ff00:42:8329", "<ipv6>"),
            ("loop ::1 end", "loop <ipv6> end"),
        ],
        &["10:30:00", "std::string"],
    );
}

#[test]
fn guids() {
    check(
        Detector::Guid,
        &[("id {3F2504E0-4F89-11D3-9A0C-0305E82C3301}", "id {<guid>}")],
        &["3F2504E0-4F89-11D3-9A0C", "not-a-guid"],
    );
}

#[test]
fn phones() {
    check(
        Detector::Phone,
        &[
            ("call +1 555 123 4567 now", "call <phone> now"),
            ("+44 (20) 7946 0958", "<phone>"),
            ("(555) 123-4567", "<phone>"),
            ("tel 555-123-4567.", "tel <phone>."),
        ],
        &["2026-10-14 10:00:00", "+1 2", "v1.2.3", "12-34"],
    );
}

#[test]
fn hostnames() {
    check(
        Detector::Hostname,
        &[
            ("connect db01.corp.contoso.com:1433", "connect <host>:1433"),
            ("ping web.eu.example.de", "ping <host>"),
        ],
        &[
            "System.Net.Http failed",
            "appsettings.json",
            "example.com",
            "Orders.Api.Controllers",
        ],
    );
}

#[test]
fn every_detector_has_a_name_label_and_description_and_compiles() {
    for d in Detector::ALL {
        assert!(!d.name().is_empty() && !d.label().is_empty() && !d.description().is_empty());
        let r = Rule::detector(d, Replacement::Mode(Mode::Mask));
        assert_eq!(r.name(), d.name());
        let parsed: Detector = serde_json::from_value(json!(d.name())).unwrap();
        assert_eq!(parsed, d);
    }
}

#[test]
fn custom_rules_use_the_value_group_templates_or_the_whole_match() {
    let cfg = RedactCfg {
        detectors: vec![],
        mode: Mode::Mask,
        fields: vec!["*".into()],
        on_export: false,
        rule: vec![
            RedactRuleCfg {
                name: "customer".into(),
                regex: r"Customer (?P<value>\w+)".into(),
                replace: None,
            },
            RedactRuleCfg {
                name: "order".into(),
                regex: r"order-(\d+)".into(),
                replace: Some("order-#$1#".into()),
            },
            RedactRuleCfg {
                name: "ticket".into(),
                regex: r"TCK\d+".into(),
                replace: None,
            },
        ],
    };
    let r = Redactor::from_config(&cfg, Salt::fixed([0; 16])).unwrap();
    let mut c = Counts::new();
    let out = r.apply("Customer acme placed order-42 (TCK7, TCK8)", &mut c);
    assert_eq!(out, "Customer <customer> placed order-#42# (<ticket>, <ticket>)");
    assert_eq!((c["customer"], c["order"], c["ticket"]), (1, 1, 2));
    assert_eq!(r.rule_names(), ["customer", "order", "ticket"]);

    let bad = |name: &str, regex: &str| RedactCfg {
        rule: vec![RedactRuleCfg {
            name: name.into(),
            regex: regex.into(),
            replace: None,
        }],
        ..RedactCfg::default()
    };
    let e = Redactor::from_config(&bad("x", "("), Salt::random()).err().unwrap();
    assert!(format!("{e:#}").contains("redaction rule \"x\""), "{e:#}");
    let e = Redactor::from_config(&bad(" ", "a"), Salt::random()).err().unwrap();
    assert!(e.to_string().contains("needs a name"));
    let huge = "a".repeat(crate::user_regex::MAX_PATTERN_LEN + 1);
    assert!(Redactor::from_config(&bad("big", &huge), Salt::random()).is_err());
}

#[test]
fn literal_replacements_keep_the_rest_of_the_match() {
    let r = Redactor::new(
        vec![Rule::detector(Detector::Password, Replacement::Literal("[REDACTED]".into()))],
        FieldScope::All,
        Salt::fixed([0; 16]),
    );
    assert_eq!(r.apply("pwd=abc rest", &mut Counts::new()), "pwd=[REDACTED] rest");
}

#[test]
fn modes_mask_hash_and_drop() {
    let cfg = |mode| RedactCfg {
        detectors: vec![Detector::Email],
        mode,
        ..RedactCfg::default()
    };
    let text = "a@example.com b@example.com a@example.com";
    let salt = Salt::fixed([3; 16]);
    let drop = Redactor::from_config(&cfg(Mode::Drop), salt).unwrap();
    assert_eq!(drop.apply(text, &mut Counts::new()), "  ");

    // Equal values hash equally within one export (salt) ...
    let hash = Redactor::from_config(&cfg(Mode::Hash), salt).unwrap();
    let out = hash.apply(text, &mut Counts::new()).into_owned();
    let tokens: Vec<&str> = out.split(' ').collect();
    assert_eq!(tokens.len(), 3);
    assert!(tokens[0].starts_with("<email:") && tokens[0].ends_with('>') && tokens[0].len() == "<email:>".len() + 10);
    assert_eq!(tokens[0], tokens[2]);
    assert_ne!(tokens[0], tokens[1]);
    // ... are stable for the same salt ...
    let again = Redactor::from_config(&cfg(Mode::Hash), salt).unwrap();
    assert_eq!(again.apply(text, &mut Counts::new()), out);
    // ... and differ across exports.
    let other = Redactor::from_config(&cfg(Mode::Hash), Salt::random()).unwrap();
    assert_ne!(other.apply(text, &mut Counts::new()), out);
    assert_ne!(Salt::random(), Salt::random());
    assert_eq!(format!("{:?}", Salt::random()), "Salt(..)");
    assert_ne!(fallback_entropy(), [0u8; 16]);
}

#[test]
fn scopes_select_message_exception_template_and_fields() {
    let scope = FieldScope::parse(&["@message".into(), " ".into(), "User".into(), "@template".into()]);
    assert!(scope.covers("@message") && scope.covers("User") && scope.covers("@template"));
    assert!(!scope.covers("@exception") && !scope.covers("Other"));
    assert!(FieldScope::parse(&["@message".into()]).covers("@template"));
    assert!(!FieldScope::parse(&["@exception".into()]).covers("@template"));
    assert_eq!(FieldScope::parse(&["@exception".into(), "*".into()]), FieldScope::All);

    let r = Redactor::new(
        vec![Rule::detector(Detector::Email, Replacement::Mode(Mode::Mask))],
        FieldScope::parse(&["@message".into(), "User".into()]),
        Salt::fixed([0; 16]),
    );
    assert_eq!(r.scope(), &FieldScope::parse(&["@message".into(), "User".into()]));
    let mut rec = Record::new(1);
    rec.message = "from a@x.io".into();
    rec.exception = Some("b@x.io failed".into());
    rec.template = Some("from {User}".into());
    rec.fields.insert("User".into(), json!({"mail": ["c@x.io"], "n": 5}));
    rec.fields.insert("Other".into(), json!("d@x.io"));
    let mut c = Counts::new();
    let out = r.record(&rec, &mut c);
    assert!(matches!(out, Cow::Owned(_)));
    assert_eq!(out.message, "from <email>");
    assert_eq!(out.exception.as_deref(), Some("b@x.io failed"));
    assert_eq!(out.fields["User"], json!({"mail": ["<email>"], "n": 5}));
    assert_eq!(out.fields["Other"], "d@x.io");
    assert_eq!(c["email"], 2);
    assert_eq!(r.field("Other", Cow::Borrowed("e@x.io"), &mut c), "e@x.io");
    assert_eq!(r.field("User", Cow::Owned("e@x.io".to_string()), &mut c), "<email>");

    let mut clean = Record::new(2);
    clean.message = "nothing here".into();
    clean.fields.insert("User".into(), json!("bob"));
    assert!(matches!(r.record(&clean, &mut Counts::new()), Cow::Borrowed(_)));
    assert!(matches!(r.fields(&clean.fields, &mut Counts::new()), Cow::Borrowed(_)));

    let all = Redactor::new(
        vec![Rule::detector(Detector::Email, Replacement::Mode(Mode::Mask))],
        FieldScope::All,
        Salt::fixed([0; 16]),
    );
    let out = all.record(&rec, &mut Counts::new());
    assert_eq!(out.exception.as_deref(), Some("<email> failed"));
    assert_eq!(out.fields["Other"], "<email>");
}

#[test]
fn json_redaction_keeps_numbers_and_structure() {
    let r = only(Detector::Email);
    let v = json!({"a": ["x@y.io", {"k": "z@y.io"}], "n": 1, "b": true, "z": null});
    let out = r.apply_json(&v, &mut Counts::new());
    assert_eq!(out, json!({"a": ["<email>", {"k": "<email>"}], "n": 1, "b": true, "z": null}));
}

#[test]
fn previews_show_before_after_and_hits_per_rule() {
    let cfg = RedactCfg {
        detectors: vec![Detector::Email, Detector::Ipv4],
        ..RedactCfg::default()
    };
    let p = preview(&cfg, "mail a@b.io");
    assert_eq!((p.before.as_str(), p.after.as_str()), ("mail a@b.io", "mail <email>"));
    assert_eq!(
        p.hits,
        [
            RuleHits {
                rule: "email".into(),
                count: 1
            },
            RuleHits {
                rule: "ipv4".into(),
                count: 0
            }
        ]
    );
    assert!(p.error.is_none());

    let bad = RedactCfg {
        rule: vec![RedactRuleCfg {
            name: "x".into(),
            regex: "(".into(),
            replace: None,
        }],
        ..RedactCfg::default()
    };
    let p = preview(&bad, "same");
    assert_eq!(p.after, "same");
    assert!(p.error.unwrap().contains("redaction rule"));
}

#[test]
fn config_parses_from_toml_with_defaults() {
    let cfg: RedactCfg = toml::from_str("detectors = ['email', 'user-path']\nmode = 'hash'\n[[rule]]\nname='n'\nregex='r'").unwrap();
    assert_eq!(cfg.detectors, [Detector::Email, Detector::UserPath]);
    assert_eq!(cfg.mode, Mode::Hash);
    assert_eq!(cfg.fields, ["@message", "@exception"]);
    assert!(!cfg.on_export);
    assert!(toml::from_str::<RedactCfg>("detectors = ['nope']").is_err());
    assert!(toml::from_str::<RedactCfg>("mode = 'blur'").is_err());
    assert_eq!(RedactCfg::default().mode, Mode::Mask);
}

#[test]
fn empty_redactor_changes_nothing() {
    let r = Redactor::new(Vec::new(), FieldScope::All, Salt::random());
    assert!(r.is_empty());
    assert!(matches!(r.apply("a@b.io", &mut Counts::new()), Cow::Borrowed(_)));
}
