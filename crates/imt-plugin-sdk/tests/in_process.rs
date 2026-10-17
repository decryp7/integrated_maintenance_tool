//! The plugin service served in this process (so every handler is exercised here), checked with the
//! conformance kit and with direct calls: typed errors, malformed requests and limits.

use imt_core::importers::{ImportCtx, Imported, Importer};
use imt_core::model::Record;
use imt_core::{Plugin, PluginRegistry};
use imt_ipc::endpoint::{new_token, PrivateEndpoint};
use imt_plugin_sdk::client;
use imt_plugin_sdk::conformance::{run, Endpoint, Options};
use imt_plugin_sdk::server::PluginService;
use imt_plugin_sdk::{code, pb, ServeOptions};
use std::sync::Arc;
use std::time::Duration;

struct Lines;

impl Importer for Lines {
    fn id(&self) -> &'static str {
        "lines"
    }
    fn sniff(&self, _: &ImportCtx, head: &str) -> u8 {
        if head.is_empty() {
            0
        } else {
            10
        }
    }
    fn import(&self, ctx: &ImportCtx, content: &str) -> anyhow::Result<Imported> {
        let mut out = Imported::default();
        for (i, l) in content.lines().enumerate() {
            let mut r = Record::new(i as u32 + 1);
            r.message = format!("{}:{l}", ctx.path.display());
            out.records.push(r);
        }
        Ok(out)
    }
}

struct Bin;

impl imt_core::importers::BinaryImporter for Bin {
    fn id(&self) -> &'static str {
        "bin"
    }
    fn sniff(&self, _: &ImportCtx, head: &[u8]) -> u8 {
        if head.starts_with(b"BIN") {
            100
        } else {
            0
        }
    }
    fn import(&self, _: &ImportCtx, bytes: &[u8]) -> anyhow::Result<Imported> {
        if bytes.starts_with(b"BAD") {
            anyhow::bail!("corrupt");
        }
        Ok(Imported {
            records: vec![Record::new(bytes.len() as u32)],
            skipped: 0,
        })
    }
}

struct Csv;

impl imt_core::export::Exporter for Csv {
    fn id(&self) -> &'static str {
        "count"
    }
    fn name(&self) -> &'static str {
        "Count"
    }
    fn extension(&self) -> &'static str {
        "txt"
    }
    fn export(
        &self,
        rows: &mut dyn Iterator<Item = imt_core::export::ExportRow<'_>>,
        out: &mut dyn std::io::Write,
    ) -> anyhow::Result<usize> {
        let mut n = 0;
        for r in rows {
            out.write_all(&vec![b'x'; 300_000])?;
            writeln!(out, "{}", r.source_name)?;
            n += 1;
        }
        Ok(n)
    }
}

struct Annot;

impl imt_core::plugin::annotate::Annotator for Annot {
    fn id(&self) -> &str {
        "annot"
    }
    fn annotate(
        &self,
        view: &dyn imt_core::plugin::annotate::AnnotationView,
        since: Option<imt_core::plugin::annotate::RecordCursor>,
    ) -> anyhow::Result<Vec<imt_core::plugin::annotate::Annotation>> {
        // Out of process, queries are refused with a hint; select works over the records sent.
        assert!(view.query(&Default::default(), 1).is_err());
        let newest = view.select(
            &imt_core::plugin::annotate::ViewFilter {
                newest_first: true,
                from: Some(i64::MIN),
                to: Some(i64::MAX),
                ..Default::default()
            },
            1,
        )?;
        let (from, to) = view.time_range().unwrap_or((0, 0));
        Ok(vec![imt_core::plugin::annotate::Annotation {
            kind: "k".into(),
            from,
            to,
            instance: view.instances().first().cloned(),
            on: None,
            label: format!("{} newest, since {:?}", newest.len(), since.map(|s| s.id)),
            severity: imt_core::api::Severity::Info,
            evidence_query: Default::default(),
        }])
    }
}

struct P;

impl Plugin for P {
    fn id(&self) -> &'static str {
        "test.inproc"
    }
    fn name(&self) -> &'static str {
        "In process"
    }
    fn version(&self) -> &'static str {
        "0.1.0"
    }
    fn register(&self, r: &mut PluginRegistry) -> anyhow::Result<()> {
        r.add_importer(Box::new(Lines));
        r.add_binary_importer(Box::new(Bin));
        r.add_exporter(Arc::new(Csv))?;
        r.add_annotator(Arc::new(Annot))?;
        Ok(())
    }
}

struct Server {
    endpoint: PrivateEndpoint,
    token: String,
    _rt: tokio::runtime::Runtime,
}

fn serve(plugin: Arc<dyn Plugin>, opts: ServeOptions) -> Server {
    let endpoint = PrivateEndpoint::create_named("imt-sdk-test").unwrap();
    let token = new_token().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let routes = PluginService::new(plugin, opts).unwrap().routes();
    let (address, t) = (endpoint.address.clone(), token.clone());
    rt.spawn(async move {
        let _ = imt_ipc::server::serve(routes, &address, &t, Default::default(), std::future::pending()).await;
    });
    std::thread::sleep(Duration::from_millis(100));
    Server { endpoint, token, _rt: rt }
}

#[test]
fn the_service_conforms_in_process() {
    let s = serve(Arc::new(P), ServeOptions::default());
    let report = run(
        &Endpoint {
            address: s.endpoint.address.clone(),
            token: s.token.clone(),
        },
        Options {
            fuzz_cases: 16,
            ..Options::default()
        },
    );
    assert!(report.passed(), "{report}");
    assert!(report.checks.iter().any(|c| c.name.contains("exporter \"count\"")));
    assert!(report
        .checks
        .iter()
        .any(|c| c.name.contains("annotator \"annot\"") && c.detail.contains("1 annotation")));
}

#[test]
fn direct_calls_cover_typed_errors_and_order() {
    let s = serve(Arc::new(P), ServeOptions::default());
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let channel = rt.block_on(imt_ipc::client::connect_channel(s.endpoint.address.clone())).unwrap();
    let mut c = client::client(imt_ipc::client::authenticated(channel, &s.token).unwrap());
    let t = Some(Duration::from_secs(5));
    let m = rt.block_on(client::handshake(&mut c, &[], t)).unwrap();
    assert_eq!(m.capabilities.importers.len(), 2);
    let ctx = pb::ImportContext {
        file_name: "f.bin".into(),
        path: "/secret/f.bin".into(),
        file_date: "nonsense".into(),
        profile_json: String::new(),
    };
    // A binary importer: bytes in, records out; a plugin error is FAILED.
    let start = |id: &str| pb::ImportStart {
        importer_id: id.into(),
        ctx: Some(ctx.clone()),
        total_bytes: 3,
        batch_rows: 0,
    };
    let mut n = 0;
    let out = rt.block_on(client::import(
        &mut c,
        false,
        start("bin"),
        vec![(pb::ChunkPart::Content, bytes::Bytes::from_static(b"BIN"))],
        t,
        |d| {
            n += d.records.len();
            Ok(())
        },
    ));
    assert_eq!((out.unwrap().skipped, n), (0, 1));
    let e = rt
        .block_on(client::import(
            &mut c,
            false,
            start("bin"),
            vec![(pb::ChunkPart::Content, bytes::Bytes::from_static(b"BAD"))],
            t,
            |_| Ok(()),
        ))
        .unwrap_err();
    assert!(
        matches!(&e, imt_ipc::IpcError::Remote(r) if r.code == code::FAILED && r.message.contains("corrupt")),
        "{e:?}"
    );
    let sniff = rt.block_on(client::sniff(
        &mut c,
        pb::SniffRequest {
            importer_id: "bin".into(),
            ctx: Some(ctx.clone()),
            head: bytes::Bytes::from_static(b"BIN"),
        },
        t,
    ));
    assert_eq!(sniff.unwrap(), 100);
    // The path is not passed on without the permission: only the name.
    let mut msgs = Vec::new();
    rt.block_on(client::import(
        &mut c,
        false,
        start("lines"),
        vec![(pb::ChunkPart::Content, bytes::Bytes::from_static(b"a\nb"))],
        t,
        |d| {
            msgs.extend(d.records.into_iter().map(|r| r.message));
            Ok(())
        },
    ))
    .unwrap();
    assert_eq!(msgs, ["f.bin:a", "f.bin:b"]);
    // Text importers have no tail of their own here: the default says "cannot append".
    let out = rt
        .block_on(client::import(
            &mut c,
            true,
            start("lines"),
            vec![(pb::ChunkPart::Tail, bytes::Bytes::from_static(b"x"))],
            t,
            |_| Ok(()),
        ))
        .unwrap();
    assert!(out.tail_unsupported);
    let e = rt
        .block_on(client::import(&mut c, true, start("bin"), vec![], t, |_| Ok(())))
        .unwrap_err();
    assert!(matches!(e, imt_ipc::IpcError::Remote(r) if r.code == code::INVALID));
    // Wrong message order and unknown ids are INVALID.
    let bad = rt.block_on(async { c.import(tokio_stream::iter(vec![pb::ImportChunk::default()])).await.err().unwrap() });
    assert_eq!(imt_ipc::error::remote_error(&bad).unwrap().code, code::INVALID);
    let bad = rt.block_on(async {
        c.transform(tokio_stream::iter(vec![pb::TransformIn::default()]))
            .await
            .err()
            .unwrap()
    });
    assert_eq!(imt_ipc::error::remote_error(&bad).unwrap().code, code::INVALID);
    let bad = rt.block_on(async { c.annotate(tokio_stream::iter(vec![pb::AnnotateIn::default()])).await.err().unwrap() });
    assert_eq!(imt_ipc::error::remote_error(&bad).unwrap().code, code::INVALID);
    let bad = rt.block_on(async { c.export(tokio_stream::iter(vec![pb::ExportIn::default()])).await.err().unwrap() });
    assert_eq!(imt_ipc::error::remote_error(&bad).unwrap().code, code::INVALID);
    let too_big = pb::ImportStart {
        importer_id: "lines".into(),
        ctx: Some(ctx.clone()),
        total_bytes: u64::MAX,
        batch_rows: 0,
    };
    let e = rt
        .block_on(client::import(&mut c, false, too_big, vec![], t, |_| Ok(())))
        .unwrap_err();
    assert!(matches!(e, imt_ipc::IpcError::Remote(r) if r.code == code::TOO_LARGE));
    let e = rt.block_on(client::transform(&mut c, "nope", "s", vec![], t)).unwrap_err();
    assert!(matches!(e, imt_ipc::IpcError::Remote(r) if r.code == code::INVALID));
    let e = rt.block_on(client::settings_section(&mut c, t)).unwrap_err();
    assert!(matches!(e, imt_ipc::IpcError::Remote(r) if r.code == code::UNIMPLEMENTED));
    let e = rt.block_on(client::apply_settings(&mut c, &serde_json::json!({}), t)).unwrap_err();
    assert!(matches!(e, imt_ipc::IpcError::Remote(r) if r.code == code::UNIMPLEMENTED));
    let e = rt
        .block_on(client::call_service(&mut c, pb::ServiceCall::default(), t))
        .unwrap_err();
    assert!(matches!(e, imt_ipc::IpcError::Remote(r) if r.code == code::UNIMPLEMENTED));
    // Events of known and unknown kinds are accepted.
    let evs = vec![
        pb::HostEvent {
            kind: "recordsAppended".into(),
            payload_json: r#"{"count":2,"sources":[1]}"#.into(),
        },
        pb::HostEvent {
            kind: "profileChanged".into(),
            payload_json: r#"{"name":"p"}"#.into(),
        },
        pb::HostEvent {
            kind: "workspaceCleared".into(),
            payload_json: String::new(),
        },
        pb::HostEvent {
            kind: "fromTheFuture".into(),
            payload_json: "{".into(),
        },
    ];
    assert_eq!(rt.block_on(client::events(&mut c, evs, t)).unwrap(), 4);
    // An annotation run with `since` and a time range.
    let rec = Record::new(1);
    let batch = imt_plugin_sdk::arrow::encode(
        &[imt_plugin_sdk::arrow::WithExtra {
            record: &rec,
            extra: ["s", "i"],
        }],
        &["source_name", "instance"],
    )
    .unwrap();
    let list = rt
        .block_on(client::annotate(
            &mut c,
            pb::AnnotateStart {
                annotator_id: "annot".into(),
                has_since: true,
                since_id: 4,
                since_has_ts: true,
                since_ts: 3,
                has_range: true,
                range_from: 1,
                range_to: 9,
                instances: vec!["i".into()],
            },
            vec![batch],
            t,
        ))
        .unwrap();
    assert_eq!(list[0].label, "0 newest, since Some(4)");
    assert_eq!((list[0].from, list[0].to), (1, 9));
    let e = rt
        .block_on(client::annotate(
            &mut c,
            pb::AnnotateStart {
                annotator_id: "nope".into(),
                ..Default::default()
            },
            vec![],
            t,
        ))
        .unwrap_err();
    assert!(matches!(e, imt_ipc::IpcError::Remote(r) if r.code == code::INVALID));
    let e = rt
        .block_on(client::annotate(
            &mut c,
            pb::AnnotateStart {
                annotator_id: "annot".into(),
                ..Default::default()
            },
            vec![bytes::Bytes::from_static(b"garbage")],
            t,
        ))
        .unwrap_err();
    assert!(matches!(e, imt_ipc::IpcError::Remote(r) if r.code == code::INVALID));
}

#[test]
fn a_plugin_that_cannot_run_alone_is_refused_at_start() {
    struct Needy;
    impl Plugin for Needy {
        fn id(&self) -> &'static str {
            "test.needy"
        }
        fn name(&self) -> &'static str {
            "Needy"
        }
        fn version(&self) -> &'static str {
            "1.0.0"
        }
        fn manifest(&self) -> imt_core::plugin::Manifest {
            imt_core::plugin::Manifest::new("test.needy", "1.0.0").requires("other.plugin", "^1")
        }
        fn register(&self, _: &mut PluginRegistry) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let e = PluginService::new(Arc::new(Needy), ServeOptions::default())
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("inactive") && e.contains("other.plugin"), "{e}");
    struct Broken;
    impl Plugin for Broken {
        fn id(&self) -> &'static str {
            "test.broken"
        }
        fn name(&self) -> &'static str {
            "Broken"
        }
        fn version(&self) -> &'static str {
            "1.0.0"
        }
        fn register(&self, _: &mut PluginRegistry) -> anyhow::Result<()> {
            anyhow::bail!("cannot register")
        }
    }
    assert!(PluginService::new(Arc::new(Broken), ServeOptions::default()).is_err());
}
