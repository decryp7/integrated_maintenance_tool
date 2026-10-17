use super::html::{escape, render, HtmlRenderer};
use super::*;
use serde_json::json;

fn sample() -> Value {
    json!({
        "version": 1,
        "id": "r1",
        "title": "Checkout outage",
        "summary": "Payments **failed**.",
        "createdAt": 1, "updatedAt": 2,
        "sections": [{
            "id": "s1", "title": "What happened",
            "blocks": [
                {"id": "b1", "type": "text", "markdown": "# Hi"},
                {"id": "b2", "type": "events", "caption": "Bookmarks", "ids": [1, 2]},
                {"id": "b3", "type": "chart", "title": "Errors", "spec": {"id": "c", "title": "C", "y": "count()"}, "svg": "<svg><script>alert(1)</script><rect width=\"1\"/></svg>"},
                {"id": "b4", "type": "timeline", "svg": "<svg/>", "from": 1, "to": 2},
                {"id": "b4b", "type": "chart", "title": "No snapshot"},
                {"id": "b5", "type": "table", "query": "| stats count()", "columns": ["count()"], "rows": [[3]]},
                {"id": "b6", "type": "finding", "severity": "critical", "title": "Spike", "detail": "x"},
                {"id": "b7", "type": "annotations", "items": [{"from": 1, "label": "reboot"}]},
                {"id": "b8", "type": "hologram", "beam": 3},
                {"id": "b9", "type": "events"}
            ]
        }]
    })
}

#[test]
fn round_trips_and_keeps_unknown_blocks() {
    let r = Report::parse(sample()).unwrap();
    let blocks = &r.sections[0].blocks;
    assert_eq!(blocks.len(), 10);
    let kinds: Vec<&str> = blocks
        .iter()
        .filter_map(|b| match b {
            StoredBlock::Known(i) => Some(i.block.kind()),
            StoredBlock::Unknown(_) => None,
        })
        .collect();
    assert_eq!(
        kinds,
        ["text", "events", "chart", "timeline", "chart", "table", "finding", "annotations"]
    );
    // A newer block type and a damaged one are kept verbatim.
    assert!(matches!(&blocks[8], StoredBlock::Unknown(v) if v["beam"] == 3));
    assert!(matches!(&blocks[9], StoredBlock::Unknown(v) if v["type"] == "events"));
    // SVG snapshots are sanitised on the way in.
    let StoredBlock::Known(chart) = &blocks[2] else { panic!() };
    let Block::Chart { svg: Some(svg), .. } = &chart.block else {
        panic!()
    };
    assert!(!svg.contains("script") && svg.contains("<rect"));

    let back: Report = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
    assert_eq!(back, r);
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["sections"][0]["blocks"][8], json!({"id": "b8", "type": "hologram", "beam": 3}));
    assert_eq!(
        v["sections"][0]["blocks"][0],
        json!({"id": "b1", "type": "text", "markdown": "# Hi"})
    );
}

#[test]
fn defaults_and_limits() {
    let r = Report::parse(json!({"id": "x"})).unwrap();
    assert_eq!((r.version, r.title.as_str(), r.sections.len()), (REPORT_VERSION, "", 0));
    let err = |v: Value| format!("{:#}", Report::parse(v).unwrap_err());
    assert!(err(json!({"id": " "})).contains("needs an id"));
    assert!(err(json!({"title": "no id"})).contains("not a valid report"));
    assert!(err(json!({"id": "x", "title": "a".repeat(MAX_TEXT + 1)})).contains("title"));
    assert!(err(json!({"id": "x", "summary": "a".repeat(MAX_TEXT + 1)})).contains("summary"));
    let sections: Vec<Value> = (0..=MAX_SECTIONS).map(|i| json!({"id": i.to_string()})).collect();
    assert!(err(json!({"id": "x", "sections": sections})).contains("sections"));
    let blocks: Vec<Value> = (0..=MAX_BLOCKS).map(|i| json!({"id": i.to_string(), "type": "text"})).collect();
    assert!(err(json!({"id": "x", "sections": [{"id": "s", "blocks": blocks}]})).contains("blocks"));
    let one = |b: Value| json!({"id": "x", "sections": [{"id": "s", "title": "", "blocks": [b]}]});
    let long = "a".repeat(MAX_TEXT + 1);
    assert!(err(one(json!({"id": "b", "type": "text", "markdown": long}))).contains("block \"b\""));
    assert!(err(one(json!({"id": "b", "type": "events", "ids": vec![1; MAX_EVENT_IDS + 1]}))).contains("events per block"));
    assert!(err(one(json!({"id": "b", "type": "events", "caption": long, "ids": []}))).contains("caption"));
    assert!(err(one(json!({"id": "b", "type": "chart", "caption": long}))).contains("caption"));
    assert!(err(one(json!({"id": "b", "type": "chart", "title": long}))).contains("title"));
    assert!(err(one(json!({"id": "b", "type": "chart", "svg": "<p/>"}))).contains("<svg> root"));
    assert!(err(one(json!({"id": "b", "type": "timeline", "caption": long}))).contains("caption"));
    assert!(err(one(
        json!({"id": "b", "type": "table", "columns": [], "rows": vec![json!([]); MAX_TABLE_ROWS + 1]})
    ))
    .contains("rows"));
    assert!(err(one(json!({"id": "b", "type": "table", "caption": long, "columns": [], "rows": []}))).contains("caption"));
    assert!(err(one(json!({"id": "b", "type": "table", "query": long, "columns": [], "rows": []}))).contains("query"));
    assert!(err(one(json!({"id": "b", "type": "finding", "title": long}))).contains("title"));
    assert!(err(one(json!({"id": "b", "type": "finding", "title": "t", "detail": long}))).contains("detail"));
    assert!(err(one(json!({"id": "b", "type": "annotations", "caption": long, "items": []}))).contains("caption"));
    let items = vec![json!({"from": 1, "label": "x"}); MAX_EVENT_IDS + 1];
    assert!(err(one(json!({"id": "b", "type": "annotations", "items": items}))).contains("annotations per block"));
    let mut s = Report::parse(json!({"id": "x"})).unwrap();
    s.sections.push(Section {
        id: "s".into(),
        title: long,
        blocks: vec![],
    });
    assert!(s.check().is_err());
}

#[test]
fn projects_skip_unreadable_reports() {
    #[derive(Deserialize)]
    struct P {
        #[serde(default, deserialize_with = "tolerant_reports")]
        reports: Vec<Report>,
    }
    let p: P = serde_json::from_value(json!({"reports": [{"id": "ok"}, {"title": "no id"}, 5]})).unwrap();
    assert_eq!(p.reports.len(), 1);
    let p: P = serde_json::from_value(json!({"reports": "nonsense"})).unwrap();
    assert!(p.reports.is_empty());
}

fn resolved(message: &str) -> ResolvedReport {
    ResolvedReport {
        title: "<b>T</b>".into(),
        summary: "Hi **there** <script>x</script> [link](javascript:alert(1)) [ok](https://e.x/?a=1&b=\"2\")".into(),
        generated_at: 0,
        redacted: true,
        redaction: vec![
            RuleHits {
                rule: "email".into(),
                count: 1,
            },
            RuleHits {
                rule: "ipv4".into(),
                count: 0,
            },
        ],
        sections: vec![ResolvedSection {
            title: "S <i>".into(),
            blocks: vec![
                ResolvedBlock::Events {
                    caption: "c\"".into(),
                    events: vec![EventLine {
                        id: 1,
                        ts: Some(0),
                        level: "Error".into(),
                        source: "a<b".into(),
                        message: message.into(),
                        exception: Some("<x>".into()),
                    }],
                    missing: 2,
                },
                ResolvedBlock::Figure {
                    caption: String::new(),
                    title: "F".into(),
                    svg: Some("<svg><rect/></svg>".into()),
                },
                ResolvedBlock::Figure {
                    caption: "none".into(),
                    title: String::new(),
                    svg: None,
                },
                ResolvedBlock::Table {
                    caption: String::new(),
                    columns: vec!["<c>".into()],
                    rows: vec![vec!["<img src=x onerror=alert(1)>".into()]],
                },
                ResolvedBlock::Finding {
                    severity: "critical".into(),
                    title: "<t>".into(),
                    detail: "<d>".into(),
                },
                ResolvedBlock::Annotations {
                    caption: String::new(),
                    items: vec![
                        AnnotationItem {
                            from: 0,
                            to: Some(1),
                            label: "<l>".into(),
                            kind: "k".into(),
                        },
                        AnnotationItem {
                            from: 0,
                            to: None,
                            label: "m".into(),
                            kind: String::new(),
                        },
                    ],
                },
                ResolvedBlock::Text {
                    markdown: "## h\n- a\n1. b\n> q\n```\n<code>\n```\n---\n*e* `c`".into(),
                },
                ResolvedBlock::Unsupported { kind: "<k>".into() },
            ],
        }],
    }
}

#[test]
fn html_escapes_every_text_so_log_content_stays_text() {
    let attack = "<script>alert('x')</script><img src=x onerror=alert(1)> & \"quotes\"";
    let html = render(&resolved(attack), Theme::Light);
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("default-src 'none'"));
    // The only markup is ours: no script element, no event handler attribute.
    assert!(!html.contains("<script"), "{html}");
    assert!(!html.contains("<img"), "{html}");
    assert!(!html.contains("javascript:"));
    assert!(html.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;&lt;img src=x onerror=alert(1)&gt; &amp; &quot;quotes&quot;"));
    assert!(html.contains("<title>&lt;b&gt;T&lt;/b&gt;</title>"));
    assert!(html.contains("<a href=\"https://e.x/?a=1&amp;b=&quot;2&quot;\" rel=\"noopener noreferrer\">ok</a>"));
    assert!(html.contains("<strong>there</strong>"));
    assert!(html.contains("<svg><rect/></svg>"));
    assert!(html.contains("2 event(s) are no longer"));
    assert!(html.contains("Redacted: 1 value masked — email (1)."));
    for tag in [
        "<h3>h</h3>",
        "<ul><li>a</li></ul>",
        "<ol><li>b</li></ol>",
        "<blockquote>q</blockquote>",
        "<pre><code>&lt;code&gt;</code></pre>",
        "<hr>",
        "<em>e</em>",
        "<code>c</code>",
    ] {
        assert!(html.contains(tag), "{tag} missing");
    }
    let dark = render(&resolved("m"), Theme::Dark);
    assert!(dark.contains("--bg:#14171d"));
    let mut plain = resolved("m");
    plain.redacted = false;
    assert!(!render(&plain, Theme::Light).contains("Redacted:"));
    plain.redacted = true;
    plain.redaction = vec![];
    assert!(render(&plain, Theme::Light).contains("Redacted: 0 values masked."));
}

#[test]
fn the_html_renderer_writes_to_any_output() {
    let mut out = Vec::new();
    HtmlRenderer.render(&resolved("m"), Theme::Light, &mut out).unwrap();
    assert!(String::from_utf8(out).unwrap().ends_with("</html>\n"));
    assert_eq!((HtmlRenderer.id(), HtmlRenderer.extension()), ("html", "html"));
    assert_eq!(escape("<&>\"'"), "&lt;&amp;&gt;&quot;&#39;");
    assert_eq!(format_ts(None), "—");
    assert_eq!(format_ts(Some(1_791_972_000_123)), "2026-10-14 10:00:00.123Z");
    assert_eq!(cell_text(&Value::Null), "");
    assert_eq!(cell_text(&json!(3)), "3");
}
