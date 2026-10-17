//! In process vs. out of process (ADR 0002's decision rule: gRPC unless it is more than ~10% slower for a
//! real workload). Run with `cargo bench -p imt-plugin-host` (release build).
//!
//! 1. Import 1M access-log lines with the built-in web plugin: guarded in process vs. served by the
//!    `imt-plugin-host` binary (Arrow batches over gRPC on a private socket).
//! 2. A transform over 1M records: in process vs. the test plugin's identical transform out of process, at
//!    several batch sizes.
//! 3. Where the out-of-process time goes: Arrow encode / decode of the 1M records, and the protobuf
//!    envelope (prost encode + decode of the messages carrying those Arrow bytes).
//!
//! `IMT_BENCH_LINES` changes the size (default 1,000,000).

use bytes::Bytes;
use imt_core::importers::ImportCtx;
use imt_core::model::Record;
use imt_core::plugin::{RecordTransform, TransformCtx, Verdict};
use imt_core::profile::{CompiledProfile, Profile};
use imt_core::{Plugin, PluginRegistry};
use imt_plugin_host::{builtins, PluginSpec, RemotePlugin};
use imt_plugin_sdk::{arrow, pb};
use prost::Message;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn lines() -> usize {
    std::env::var("IMT_BENCH_LINES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000)
}

fn access_log(n: usize) -> String {
    let mut s = String::with_capacity(n * 140);
    for i in 0..n {
        s.push_str(&format!(
            "10.0.{}.{} - user{} [01/Jan/2024:10:{:02}:{:02} +0000] \"GET /api/orders/{i}?page={} HTTP/1.1\" {} {} \"https://example.com/\" \"Mozilla/5.0 (X11; Linux x86_64)\" {}.{:03}\n",
            i % 250,
            i % 200,
            i % 17,
            (i / 60) % 60,
            i % 60,
            i % 9,
            [200, 200, 200, 404, 500][i % 5],
            1000 + i % 5000,
            i % 3,
            i % 1000
        ));
    }
    s
}

fn best_of<T>(runs: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best = Duration::MAX;
    let mut last = None;
    for _ in 0..runs {
        let t = Instant::now();
        let v = f();
        best = best.min(t.elapsed());
        last = Some(v);
    }
    (best, last.unwrap_or_else(f))
}

fn ms(d: Duration) -> String {
    format!("{:>8.1} ms", d.as_secs_f64() * 1000.0)
}

fn overhead(base: Duration, other: Duration) -> String {
    format!("{:+.1}%", (other.as_secs_f64() / base.as_secs_f64() - 1.0) * 100.0)
}

fn import_with(r: &PluginRegistry, content: &str) -> usize {
    let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
    let ctx = ImportCtx {
        profile: &profile,
        path: Path::new("access.log"),
        file_date: None,
    };
    let i = r.hosted_importers().find(|i| i.id() == "access-log").unwrap();
    i.import(&ctx, content).unwrap().records.len()
}

/// The same transform as the test plugin's `fake.upper`.
struct Upper;

impl RecordTransform for Upper {
    fn id(&self) -> &str {
        "upper"
    }
    fn apply(&self, r: &mut Record, _ctx: &TransformCtx) -> Verdict {
        if r.message.contains("drop") {
            return Verdict::Drop;
        }
        r.message = r.message.to_uppercase();
        r.fields.insert("upper".into(), true.into());
        Verdict::Keep
    }
}

fn main() {
    let n = lines();
    let host = env!("CARGO_BIN_EXE_imt-plugin-host");
    let fake = env!("CARGO_BIN_EXE_imt-plugin-host-fake");
    println!("imt-plugin-host transport benchmark: {n} lines, best of 3\n");
    let content = access_log(n);
    println!("input: {:.1} MB of access log", content.len() as f64 / 1e6);

    // 1. Import.
    let mut local = PluginRegistry::empty();
    local.install(&imt_plugin_web::WebPlugin).unwrap();
    let (t_local, count) = best_of(3, || import_with(&local, &content));
    assert_eq!(count, n);
    let remote = RemotePlugin::start(builtins::spec(host, "net.decryptology.web", Vec::new())).unwrap();
    let mut reg = PluginRegistry::empty();
    reg.install_shared(remote.clone()).unwrap();
    println!("\nImport (built-in access-log importer):");
    println!("  in process                          {}", ms(t_local));
    for rows in [16 * 1024, 32 * 1024, 64 * 1024, 128 * 1024] {
        remote.set_batch_rows(rows);
        let (t_remote, count) = best_of(3, || import_with(&reg, &content));
        assert_eq!(count, n);
        println!(
            "  plugin host, {:>6} rows/batch    {}   {}",
            rows,
            ms(t_remote),
            overhead(t_local, t_remote)
        );
    }
    remote.set_batch_rows(0);

    // When the batches arrive (undecoded): the plugin's import, then its encoding and the transfer.
    {
        let conn = imt_plugin_host::PluginConnection::launch(&builtins::spec(host, "net.decryptology.web", Vec::new())).unwrap();
        let rt = imt_plugin_host::connection::runtime().unwrap();
        let mut c = conn.client.clone();
        let profile_json = serde_json::to_string(&Profile::default_profile()).unwrap();
        let start = pb::ImportStart {
            importer_id: "access-log".into(),
            ctx: Some(pb::ImportContext {
                file_name: "access.log".into(),
                path: String::new(),
                file_date: String::new(),
                profile_json,
            }),
            total_bytes: content.len() as u64,
            batch_rows: 0,
        };
        let t0 = Instant::now();
        let body = Bytes::copy_from_slice(content.as_bytes());
        let copied = t0.elapsed();
        let mut arrivals = Vec::new();
        let out = imt_plugin_sdk::client::block_on(
            rt.handle(),
            imt_plugin_sdk::client::import_raw(&mut c, false, start, vec![(pb::ChunkPart::Content, body)], None, |_, _| {
                arrivals.push(t0.elapsed());
                Ok(())
            }),
        )
        .unwrap();
        let _ = out;
        println!(
            "\nRaw import call: copy {} | first batch at {} | last batch at {} | done at {}",
            ms(copied),
            ms(arrivals[0]),
            ms(*arrivals.last().unwrap()),
            ms(t0.elapsed())
        );
    }

    // 3. Where the time goes: the records of the import, as they cross.
    let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
    let ctx = ImportCtx {
        profile: &profile,
        path: Path::new("access.log"),
        file_date: None,
    };
    let imported = local
        .hosted_importers()
        .find(|i| i.id() == "access-log")
        .unwrap()
        .import(&ctx, &content)
        .unwrap()
        .records;
    let ranges = arrow::batch_ranges(imported.len(), arrow::DEFAULT_BATCH_ROWS, |i| arrow::record_size(&imported[i]));
    let (t_enc, batches) = best_of(3, || {
        ranges
            .iter()
            .map(|r| arrow::encode(&imported[r.clone()], &[]).unwrap())
            .collect::<Vec<Bytes>>()
    });
    let arrow_bytes: usize = batches.iter().map(Bytes::len).sum();
    let (t_dec, decoded) = best_of(3, || {
        batches
            .iter()
            .map(|b| arrow::decode(b.clone(), &[]).unwrap().records.len())
            .sum::<usize>()
    });
    assert_eq!(decoded, n);
    let (t_env, wire) = best_of(3, || {
        let mut total = 0;
        for b in &batches {
            let msg = pb::ImportReply {
                item: Some(pb::import_reply::Item::Batch(pb::RecordBatch { arrow: b.clone(), rows: 1 })),
            };
            let bytes = Bytes::from(msg.encode_to_vec());
            total += bytes.len();
            let back = pb::ImportReply::decode(bytes).unwrap();
            assert!(back.item.is_some());
        }
        total
    });
    // The envelope without the payload copy: how much protobuf adds per message.
    let (t_env_small, _) = best_of(3, || {
        for _ in 0..batches.len() {
            let msg = pb::ImportReply {
                item: Some(pb::import_reply::Item::Done(pb::ImportDone {
                    skipped: 1,
                    tail_unsupported: false,
                })),
            };
            let _ = pb::ImportReply::decode(Bytes::from(msg.encode_to_vec())).unwrap();
        }
    });
    println!(
        "\nWhere the out-of-process time goes ({} batches, {:.1} MB of Arrow, {:.1} MB on the wire):",
        batches.len(),
        arrow_bytes as f64 / 1e6,
        wire as f64 / 1e6
    );
    println!("  Arrow encode (plugin)              {}", ms(t_enc));
    println!("  Arrow decode (IMT)                 {}", ms(t_dec));
    println!(
        "  protobuf envelope, encode+decode   {}   (incl. one copy of the payload into the message buffer)",
        ms(t_env)
    );
    println!("  protobuf envelope, without payload {}", ms(t_env_small));

    // 2. Transform.
    let mut base: Vec<Record> = imported.clone();
    for r in base.iter_mut().step_by(10) {
        r.message.push_str(" drop");
    }
    let (t_tr_local, kept) = best_of(3, || {
        let mut recs = base.clone();
        let ctx = TransformCtx::new("access.log");
        Upper.apply_batch(&mut recs, &ctx).iter().filter(|v| **v == Verdict::Keep).count()
    });
    let clone_only = best_of(3, || base.clone().len()).0;
    println!("\nTransform over {n} records (uppercase + one field):");
    println!(
        "  in process            {}   (plus {} to clone the input, not counted)",
        ms(t_tr_local),
        ms(clone_only)
    );
    for rows in [8 * 1024, 16 * 1024, 32 * 1024, 64 * 1024] {
        let mut spec = PluginSpec::new(fake, "fake");
        spec.supervisor.max_crashes = 100;
        let p = RemotePlugin::start(spec).unwrap();
        p.set_batch_rows(rows);
        let mut r = PluginRegistry::empty();
        r.set_budgets(imt_core::guard::Budgets::unlimited());
        r.install_shared(p.clone() as Arc<dyn Plugin>).unwrap();
        let t = r.hosted_transforms().into_iter().next().unwrap();
        let (dt, kept_remote) = best_of(3, || {
            let mut recs = base.clone();
            let ctx = TransformCtx::new("access.log");
            let v = t.apply_batch(&mut recs, &ctx);
            v.iter().filter(|v| **v == Verdict::Keep).count()
        });
        assert_eq!(kept_remote, kept);
        println!(
            "  plugin host, {:>6} rows/batch {}   {} (incl. the input clone)",
            rows,
            ms(dt),
            overhead(t_tr_local + clone_only, dt)
        );
        p.shutdown();
    }
    remote.shutdown();
}
