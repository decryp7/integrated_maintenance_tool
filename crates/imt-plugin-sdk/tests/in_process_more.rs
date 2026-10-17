//! More of the plugin service in this process: transforms (marks, panics), exporters that fail, settings,
//! services, events, permissions and the error paths of malformed requests.

use bytes::Bytes;
use imt_core::importers::{ImportCtx, Imported, Importer};
use imt_core::model::Record;
use imt_core::plugin::markers::PluginMarkerKind;
use imt_core::plugin::{RecordTransform, TransformCtx, Verdict};
use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use imt_core::{Plugin, PluginRegistry};
use imt_ipc::endpoint::{new_token, PrivateEndpoint};
use imt_ipc::IpcError;
use imt_plugin_sdk::client::{self, Client};
use imt_plugin_sdk::server::PluginService;
use imt_plugin_sdk::{arrow, code, pb, ByteService, Permission, PermissionRequest, ServeOptions};
use std::sync::Arc;
use std::time::Duration;

struct Imp;
impl Importer for Imp {
    fn id(&self) -> &'static str {
        "imp"
    }
    fn sniff(&self, _: &ImportCtx, head: &str) -> u8 {
        if head == "panic" {
            panic!("sniff panicked");
        }
        1
    }
    fn import(&self, ctx: &ImportCtx, content: &str) -> anyhow::Result<Imported> {
        match content {
            "panic" => panic!("import panicked"),
            "fail" => anyhow::bail!("bad input"),
            _ => {
                let mut r = Record::new(1);
                r.message = ctx.path.display().to_string();
                Ok(Imported {
                    records: vec![r],
                    skipped: 2,
                })
            }
        }
    }
    fn import_tail(&self, ctx: &ImportCtx, _head: &str, tail: &str) -> Option<anyhow::Result<Imported>> {
        match tail {
            "panic" => panic!("tail panicked"),
            "fail" => Some(Err(anyhow::anyhow!("bad tail"))),
            _ => Some(self.import(ctx, tail)),
        }
    }
}

struct Tr;
impl RecordTransform for Tr {
    fn id(&self) -> &str {
        "tr"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        if r.message == "panic" {
            panic!("transform panicked");
        }
        if r.message == "mark" {
            ctx.mark("k");
            ctx.mark("foreign");
        }
        if r.message == "drop" {
            Verdict::Drop
        } else {
            Verdict::Keep
        }
    }
}

struct Exp;
impl imt_core::export::Exporter for Exp {
    fn id(&self) -> &'static str {
        "exp"
    }
    fn name(&self) -> &'static str {
        "Exp"
    }
    fn extension(&self) -> &'static str {
        "x"
    }
    fn export(
        &self,
        rows: &mut dyn Iterator<Item = imt_core::export::ExportRow<'_>>,
        out: &mut dyn std::io::Write,
    ) -> anyhow::Result<usize> {
        let mut n = 0;
        for r in rows {
            match r.record.message.as_str() {
                "fail" => anyhow::bail!("cannot export this"),
                "panic" => panic!("export panicked"),
                m => writeln!(out, "{m}")?,
            }
            n += 1;
        }
        Ok(n)
    }
}

struct Set;
impl Configurable for Set {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new("x", "S", SectionOwner::Core).field(SettingField::new(
            "n",
            "N",
            FieldKind::Int {
                min: 0,
                max: 5,
                step: 1,
                unit: None,
            },
            1,
        ))
    }
    fn apply_settings(&self, v: &ValidValues) -> anyhow::Result<()> {
        if v.int("n") == Some(5) {
            anyhow::bail!("five is not allowed");
        }
        Ok(())
    }
}

struct Svc;
impl ByteService for Svc {
    fn call(&self, method: &str, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
        match method {
            "panic" => panic!("service panicked"),
            "fail" => anyhow::bail!("service failed"),
            _ => Ok(payload.to_vec()),
        }
    }
}

struct P;
impl Plugin for P {
    fn id(&self) -> &'static str {
        "test.more"
    }
    fn name(&self) -> &'static str {
        "More"
    }
    fn version(&self) -> &'static str {
        "2.0.0"
    }
    fn manifest(&self) -> imt_core::plugin::Manifest {
        imt_core::plugin::Manifest::new("test.more", "2.0.0").optionally("other.plugin", "^1")
    }
    fn register(&self, r: &mut PluginRegistry) -> anyhow::Result<()> {
        r.add_importer(Box::new(Imp));
        r.add_transform(Arc::new(Tr))?;
        r.add_exporter(Arc::new(Exp))?;
        r.add_marker_kinds(vec![PluginMarkerKind::new("k", "K")])?;
        Ok(())
    }
    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        Some(Arc::new(Set))
    }
}

struct Server {
    _endpoint: PrivateEndpoint,
    _rt: tokio::runtime::Runtime,
    c: Client,
    rt: tokio::runtime::Runtime,
}

fn serve() -> Server {
    let endpoint = PrivateEndpoint::create_named("imt-sdk-more").unwrap();
    let token = new_token().unwrap();
    let srv = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let opts = ServeOptions {
        permissions: vec![PermissionRequest {
            permission: Permission::ReadOpenedFiles,
            reason: "test".into(),
        }],
        services: vec![("svc".into(), "1.0.0".into(), Arc::new(Svc) as Arc<dyn ByteService>)],
        events: vec!["recordsAppended".into()],
        annotator_interest: Vec::new(),
    };
    let routes = PluginService::new(Arc::new(P), opts).unwrap().routes();
    let (address, t) = (endpoint.address.clone(), token.clone());
    srv.spawn(async move {
        let _ = imt_ipc::server::serve(routes, &address, &t, Default::default(), std::future::pending()).await;
    });
    std::thread::sleep(Duration::from_millis(100));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let ch = rt.block_on(imt_ipc::client::connect_channel(endpoint.address.clone())).unwrap();
    let c = client::client(imt_ipc::client::authenticated(ch, &token).unwrap());
    Server {
        _endpoint: endpoint,
        _rt: srv,
        c,
        rt,
    }
}

const T: Option<Duration> = Some(Duration::from_secs(10));

fn code_of(e: &IpcError) -> i32 {
    match e {
        IpcError::Remote(r) => r.code,
        other => panic!("not a typed error: {other:?}"),
    }
}

fn ctx(path: &str, profile: &str) -> pb::ImportContext {
    pb::ImportContext {
        file_name: "f.log".into(),
        path: path.into(),
        file_date: "2024-02-03".into(),
        profile_json: profile.into(),
    }
}

fn start(id: &str, c: pb::ImportContext) -> pb::ImportStart {
    pb::ImportStart {
        importer_id: id.into(),
        ctx: Some(c),
        total_bytes: 0,
        batch_rows: 7,
    }
}

fn content(s: &'static str) -> Vec<(pb::ChunkPart, Bytes)> {
    vec![(pb::ChunkPart::Content, Bytes::from_static(s.as_bytes()))]
}

#[test]
fn imports_report_failures_crashes_and_paths() {
    let mut s = serve();
    let granted = [PermissionRequest {
        permission: Permission::ReadOpenedFiles,
        reason: String::new(),
    }];
    let m = s.rt.block_on(client::handshake(&mut s.c, &granted, T)).unwrap();
    assert_eq!(m.permissions.len(), 1);
    assert_eq!(m.requires.len(), 1);
    assert_eq!(m.provides, [("svc".to_string(), "1.0.0".to_string())]);
    assert_eq!(m.capabilities.events, ["recordsAppended"]);
    // Granted: the full path; an invalid profile is refused; a valid one is used (and cached).
    let profile = serde_json::to_string(&imt_core::Profile::default_profile()).unwrap();
    let mut msgs = Vec::new();
    for _ in 0..2 {
        let out = s.rt.block_on(client::import(
            &mut s.c,
            false,
            start("imp", ctx("/a/f.log", &profile)),
            content("ok"),
            T,
            |d| {
                msgs.extend(d.records.into_iter().map(|r| r.message));
                Ok(())
            },
        ));
        assert_eq!(out.unwrap().skipped, 2);
    }
    assert_eq!(msgs, ["/a/f.log", "/a/f.log"]);
    let e =
        s.rt.block_on(client::import(
            &mut s.c,
            false,
            start("imp", ctx("", "{")),
            content("ok"),
            T,
            |_| Ok(()),
        ))
        .unwrap_err();
    assert_eq!(code_of(&e), code::INVALID);
    for (input, want) in [("fail", code::FAILED), ("panic", code::CRASHED)] {
        let e =
            s.rt.block_on(client::import(
                &mut s.c,
                false,
                start("imp", ctx("", "")),
                content(input),
                T,
                |_| Ok(()),
            ))
            .unwrap_err();
        assert_eq!(code_of(&e), want, "{input}");
        let tail = vec![
            (pb::ChunkPart::Head, Bytes::from_static(b"h")),
            (pb::ChunkPart::Tail, Bytes::from_static(input.as_bytes())),
        ];
        let e =
            s.rt.block_on(client::import(&mut s.c, true, start("imp", ctx("", "")), tail, T, |_| Ok(())))
                .unwrap_err();
        assert_eq!(code_of(&e), want, "tail {input}");
    }
    let tail = vec![(pb::ChunkPart::Tail, Bytes::from_static(b"ok"))];
    let mut n = 0;
    s.rt.block_on(client::import(&mut s.c, true, start("imp", ctx("", "")), tail, T, |d| {
        n += d.records.len();
        Ok(())
    }))
    .unwrap();
    assert_eq!(n, 1);
    // A sniff that panics is reported as a crash, though the guard answered 0.
    let e =
        s.rt.block_on(client::sniff(
            &mut s.c,
            pb::SniffRequest {
                importer_id: "imp".into(),
                ctx: Some(ctx("", "")),
                head: Bytes::from_static(b"panic"),
            },
            T,
        ))
        .unwrap_err();
    assert_eq!(code_of(&e), code::CRASHED);
    // A second ImportStart in the content is refused; an empty context too.
    let bad = s.rt.block_on(async {
        let msgs = vec![
            pb::ImportChunk {
                item: Some(pb::import_chunk::Item::Start(start("imp", ctx("", "")))),
            },
            pb::ImportChunk {
                item: Some(pb::import_chunk::Item::Start(start("imp", ctx("", "")))),
            },
        ];
        s.c.import(tokio_stream::iter(msgs)).await.err().unwrap()
    });
    assert_eq!(imt_ipc::error::remote_error(&bad).unwrap().code, code::INVALID);
    let mut no_ctx = start("imp", ctx("", ""));
    no_ctx.ctx = None;
    let e =
        s.rt.block_on(client::import(&mut s.c, false, no_ctx, content("ok"), T, |_| Ok(())))
            .unwrap_err();
    assert_eq!(code_of(&e), code::INVALID);
}

#[test]
fn transforms_mark_drop_and_crash() {
    let mut s = serve();
    s.rt.block_on(client::handshake(&mut s.c, &[], T)).unwrap();
    let recs: Vec<Record> = ["keep", "drop", "mark", "x", "y", "z"]
        .iter()
        .map(|m| {
            let mut r = Record::new(1);
            r.message = m.to_string();
            r.markers = 9;
            r
        })
        .collect();
    let batches: Vec<(Bytes, usize)> = (0..6).map(|_| (arrow::encode(&recs, &[]).unwrap(), recs.len())).collect();
    let out = s.rt.block_on(client::transform(&mut s.c, "tr", "f.log", batches, T)).unwrap();
    assert_eq!(out.len(), 6, "every batch answered, in order");
    let t = &out[0];
    assert_eq!(t.verdicts[1], Verdict::Drop);
    assert_eq!(t.marks, vec![(2, vec!["k".to_string()])]);
    assert_eq!(t.records[0].markers, 9, "IMT's markers come back unchanged");
    // A panic in the transform fails the call as a crash.
    let mut bad = recs.clone();
    bad[0].message = "panic".into();
    let e =
        s.rt.block_on(client::transform(
            &mut s.c,
            "tr",
            "f.log",
            vec![(arrow::encode(&bad, &[]).unwrap(), bad.len())],
            T,
        ))
        .unwrap_err();
    assert_eq!(code_of(&e), code::CRASHED);
    // Garbage, and a second start, are refused.
    let e =
        s.rt.block_on(client::transform(
            &mut s.c,
            "tr",
            "f.log",
            vec![(Bytes::from_static(b"junk"), 1)],
            T,
        ))
        .unwrap_err();
    assert_eq!(code_of(&e), code::INVALID);
    let bad = s.rt.block_on(async {
        let start = || pb::TransformIn {
            item: Some(pb::transform_in::Item::Start(pb::TransformStart {
                transform_id: "tr".into(),
                source_name: "s".into(),
            })),
        };
        let mut stream =
            s.c.transform(tokio_stream::iter(vec![start(), start()]))
                .await
                .unwrap()
                .into_inner();
        tokio_stream::StreamExt::next(&mut stream).await.unwrap().err().unwrap()
    });
    assert_eq!(imt_ipc::error::remote_error(&bad).unwrap().code, code::INVALID);
}

fn export(s: &mut Server, messages: &[&str], garbage: bool) -> Result<(u64, String), IpcError> {
    let recs: Vec<Record> = messages
        .iter()
        .map(|m| {
            let mut r = Record::new(1);
            r.message = m.to_string();
            r
        })
        .collect();
    let rows: Vec<arrow::WithExtra<'_, 2>> = recs
        .iter()
        .map(|r| arrow::WithExtra {
            record: r,
            extra: ["s", "g"],
        })
        .collect();
    let batch = if garbage {
        Bytes::from_static(b"junk")
    } else {
        arrow::encode(&rows, &["source_name", "group"]).unwrap()
    };
    let c = &mut s.c;
    s.rt.block_on(async {
        let mut call = client::start_export(c, "exp", T).await?;
        for _ in 0..3 {
            let _ = call
                .input
                .send(pb::ExportIn {
                    item: Some(pb::export_in::Item::Batch(pb::RecordBatch {
                        arrow: batch.clone(),
                        rows: recs.len() as u32,
                    })),
                })
                .await;
        }
        drop(call.input);
        let mut text = String::new();
        while let Some(m) = tokio_stream::StreamExt::next(&mut call.output).await {
            match m.map_err(|e| client::error(&e, String::new))?.item {
                Some(pb::export_out::Item::Data(d)) => text.push_str(&String::from_utf8_lossy(&d)),
                Some(pb::export_out::Item::Done(d)) => return Ok((d.rows, text)),
                None => {}
            }
        }
        Err(IpcError::Protocol("no done".into()))
    })
}

#[test]
fn exports_settings_services_and_events() {
    let mut s = serve();
    s.rt.block_on(client::handshake(&mut s.c, &[], T)).unwrap();
    let (rows, text) = export(&mut s, &["a", "b"], false).unwrap();
    assert_eq!((rows, text.as_str()), (6, "a\nb\na\nb\na\nb\n"));
    assert_eq!(code_of(&export(&mut s, &["fail"], false).unwrap_err()), code::FAILED);
    assert_eq!(code_of(&export(&mut s, &["panic"], false).unwrap_err()), code::CRASHED);
    assert_eq!(code_of(&export(&mut s, &["a"], true).unwrap_err()), code::INVALID);
    let e = s.rt.block_on(client::start_export(&mut s.c, "nope", T)).err().unwrap();
    assert_eq!(code_of(&e), code::INVALID);
    // Settings: schema, values validated again, the plugin's refusal.
    let section = s.rt.block_on(client::settings_section(&mut s.c, T)).unwrap();
    assert_eq!(section.fields[0].key, "n");
    s.rt.block_on(client::apply_settings(&mut s.c, &serde_json::json!({"n": 2}), T))
        .unwrap();
    for (v, want) in [
        (serde_json::json!({"n": 99}), code::INVALID),
        (serde_json::json!({"n": 5}), code::FAILED),
    ] {
        let e = s.rt.block_on(client::apply_settings(&mut s.c, &v, T)).unwrap_err();
        assert_eq!(code_of(&e), want, "{v}");
    }
    let bad = s.rt.block_on(async {
        s.c.apply_settings(pb::ApplySettingsRequest { values_json: "{".into() })
            .await
            .err()
            .unwrap()
    });
    assert_eq!(imt_ipc::error::remote_error(&bad).unwrap().code, code::INVALID);
    // Services.
    let call = |method: &str| pb::ServiceCall {
        service_id: "svc".into(),
        version: "1.0.0".into(),
        method: method.into(),
        payload: Bytes::from_static(b"hi"),
    };
    assert_eq!(
        s.rt.block_on(client::call_service(&mut s.c, call("echo"), T)).unwrap(),
        Bytes::from_static(b"hi")
    );
    assert_eq!(
        code_of(&s.rt.block_on(client::call_service(&mut s.c, call("fail"), T)).unwrap_err()),
        code::FAILED
    );
    assert_eq!(
        code_of(&s.rt.block_on(client::call_service(&mut s.c, call("panic"), T)).unwrap_err()),
        code::CRASHED
    );
    // Marker kinds.
    let kinds = s.rt.block_on(client::marker_kinds(&mut s.c, T)).unwrap();
    assert_eq!(kinds[0].kind, "k");
}
