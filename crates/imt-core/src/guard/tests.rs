use super::*;
use std::collections::HashMap;
use std::time::Duration;

#[test]
fn catch_reports_panics_and_passes_results() {
    assert_eq!(catch("x", || Ok(5)).unwrap(), 5);
    assert!(catch::<()>("x", || Err(anyhow!("plain"))).unwrap_err().to_string() == "plain");
    let e = catch::<()>("parser", || panic!("boom {}", 1)).unwrap_err().to_string();
    assert_eq!(e, "parser crashed: boom 1");
    let e = catch::<()>("p", || std::panic::panic_any(42u8)).unwrap_err().to_string();
    assert!(e.ends_with("unknown panic"));
}

#[test]
fn health_quarantines_after_repeated_crashes() {
    let h = Health::default();
    for _ in 0..QUARANTINE_AFTER - 1 {
        h.fail("p", "x");
    }
    assert!(!h.is_quarantined("p"));
    h.fail("p", &"long ".repeat(200));
    assert!(h.is_quarantined("p"));
    assert_eq!(h.get("p").failures, QUARANTINE_AFTER);
    assert_eq!(h.get("p").last_error.unwrap().chars().count(), 500);
    h.reset("p");
    assert_eq!(h.get("p"), PluginHealth::default());
}

#[test]
fn formatted_panic_messages_are_reported() {
    let n = std::hint::black_box(7);
    let e = catch::<()>("parser", || panic!("bad line {n}")).unwrap_err().to_string();
    assert_eq!(e, "parser crashed: bad line 7");
}

#[test]
fn guarded_transforms_keep_their_id_and_records_on_a_crash() {
    struct Explodes;
    impl RecordTransform for Explodes {
        fn id(&self) -> &str {
            "explodes"
        }
        fn apply(&self, _: &mut Record, _: &TransformCtx) -> Verdict {
            std::panic::panic_any(String::from("transform exploded"))
        }
    }
    let health = Arc::new(Health::default());
    let t = GuardedTransform {
        inner: Arc::new(Explodes),
        guard: Guard {
            owner: "p".into(),
            health: health.clone(),
        },
    };
    assert_eq!(t.id(), "explodes");
    let mut r = Record::new(0);
    r.message = "kept".into();
    assert_eq!(t.apply(&mut r, &TransformCtx::new("f")), Verdict::Keep);
    assert_eq!(r.message, "kept");
    assert_eq!(health.get("p").failures, 1);
    assert!(health
        .get("p")
        .last_error
        .unwrap()
        .contains("(transform) crashed: transform exploded"));
}

/// An importer that can append, is single-line and counts its tail calls.
struct Tailing {
    tails: std::sync::atomic::AtomicUsize,
    panic_tail: bool,
}

impl Importer for Tailing {
    fn id(&self) -> &'static str {
        "tailing"
    }
    fn sniff(&self, _: &ImportCtx, _: &str) -> u8 {
        50
    }
    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        // The detached copy carries the caller's path, date and parse settings.
        assert_eq!(ctx.path, std::path::Path::new("x.log"));
        let mut r = Record::new(1);
        r.message = format!("{}:{content}", ctx.profile.profile.name);
        Ok(Imported {
            records: vec![r],
            skipped: 0,
            ..Default::default()
        })
    }
    fn import_tail(&self, _: &ImportCtx, head: &str, tail: &str) -> Option<Result<Imported>> {
        self.tails.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.panic_tail {
            panic!("tail exploded");
        }
        let mut r = Record::new(1);
        r.message = format!("{head}+{tail}");
        Some(Ok(Imported {
            records: vec![r],
            skipped: 0,
            ..Default::default()
        }))
    }
    fn multiline(&self) -> bool {
        if self.panic_tail {
            panic!("multiline exploded");
        }
        false
    }
}

fn quietly<T>(f: impl FnOnce() -> T) -> T {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = f();
    std::panic::set_hook(prev);
    out
}

fn ctx_run<T>(f: impl FnOnce(&ImportCtx) -> T) -> T {
    let mut p = Profile::default_profile();
    p.name = "prof".into();
    p.pipeline = vec![crate::pipeline::StepCfg {
        step: "plugin".into(),
        id: Some("needs.a.plugin".into()),
        ..Default::default()
    }];
    let mut transforms: HashMap<String, Arc<dyn RecordTransform>> = HashMap::new();
    struct Noop;
    impl RecordTransform for Noop {
        fn id(&self) -> &str {
            "needs.a.plugin"
        }
        fn apply(&self, _: &mut Record, _: &TransformCtx) -> Verdict {
            Verdict::Keep
        }
    }
    transforms.insert("needs.a.plugin".into(), Arc::new(Noop));
    let compiled = CompiledProfile::compile_with(p, &transforms).unwrap();
    f(&ImportCtx {
        profile: &compiled,
        path: std::path::Path::new("x.log"),
        file_date: None,
    })
}

#[test]
fn guarded_importers_forward_import_tail_and_multiline() {
    let health = Arc::new(Health::default());
    let inner = Arc::new(Tailing {
        tails: Default::default(),
        panic_tail: false,
    });
    let g = GuardedImporter {
        inner: inner.clone(),
        guard: Guard::new("p", health.clone()),
    };
    ctx_run(|ctx| {
        let out = g.import_tail(ctx, "H", "T").unwrap().unwrap();
        assert_eq!(out.records[0].message, "H+T");
        assert!(!g.multiline(), "the inner importer's answer, not the default");
        assert_eq!(g.sniff(ctx, "x"), 50);
        // The import runs detached with an owned copy of the profile (its
        // plugin pipeline removed), and returns the inner result.
        assert_eq!(g.import(ctx, "body").unwrap().records[0].message, "prof:body");
        // Without an import budget it runs on this thread.
        health.set_budgets(Budgets::unlimited());
        assert_eq!(g.import(ctx, "again").unwrap().records[0].message, "prof:again");
    });
    assert_eq!(inner.tails.load(std::sync::atomic::Ordering::SeqCst), 1);

    let crashing = GuardedImporter {
        inner: Arc::new(Tailing {
            tails: Default::default(),
            panic_tail: true,
        }),
        guard: Guard::new("q", health.clone()),
    };
    quietly(|| {
        ctx_run(|ctx| {
            let e = crashing.import_tail(ctx, "H", "T").unwrap().err().unwrap().to_string();
            assert!(e.contains("(import_tail) crashed: tail exploded"), "{e}");
            assert!(crashing.multiline(), "falls back to the safe answer");
            assert!(health.is_quarantined("q") || health.get("q").failures == 2);
            health.fail("q", "x");
            // Quarantined: refused, reported as an error result.
            assert!(crashing.import_tail(ctx, "H", "T").unwrap().is_err());
            assert!(crashing.import(ctx, "x").err().unwrap().to_string().contains("repeated crashes"));
        })
    });
}

/// Declares identity fields, or crashes doing so.
struct Ident(bool);

impl Importer for Ident {
    fn id(&self) -> &'static str {
        "ident"
    }
    fn sniff(&self, _: &ImportCtx, _: &str) -> u8 {
        0
    }
    fn import(&self, _: &ImportCtx, _: &str) -> Result<Imported> {
        Ok(Imported::default())
    }
    fn identity_fields(&self) -> Vec<String> {
        if self.0 {
            panic!("identity exploded");
        }
        vec!["Host".into()]
    }
}

#[test]
fn guarded_identity_fields_fall_back_to_none() {
    let health = Arc::new(Health::default());
    let ok = GuardedImporter {
        inner: Arc::new(Ident(false)),
        guard: Guard::new("i", health.clone()),
    };
    assert_eq!(ok.identity_fields(), ["Host"]);
    let crashing = GuardedImporter {
        inner: Arc::new(Ident(true)),
        guard: Guard::new("j", health.clone()),
    };
    assert!(quietly(|| crashing.identity_fields()).is_empty());
    assert_eq!(health.get("j").failures, 1, "the crash is attributed to the plugin");
}

struct Bin;
impl BinaryImporter for Bin {
    fn id(&self) -> &'static str {
        "bin"
    }
    fn sniff(&self, _: &ImportCtx, head: &[u8]) -> u8 {
        if head.first() == Some(&0) {
            100
        } else {
            0
        }
    }
    fn import(&self, _: &ImportCtx, bytes: &[u8]) -> Result<Imported> {
        let mut r = Record::new(1);
        r.message = format!("{} bytes", bytes.len());
        Ok(Imported {
            records: vec![r],
            skipped: 0,
            ..Default::default()
        })
    }
}

#[test]
fn guarded_binary_importers_run_detached_or_inline() {
    let health = Arc::new(Health::default());
    let g = GuardedBinaryImporter {
        inner: Arc::new(Bin),
        guard: Guard::new("b", health.clone()),
    };
    ctx_run(|ctx| {
        assert_eq!(g.id(), "bin");
        assert!(g.identity_fields().is_empty(), "the trait's default");
        assert_eq!(g.sniff(ctx, &[0, 1]), 100);
        assert_eq!(g.import(ctx, &[0, 1, 2]).unwrap().records[0].message, "3 bytes");
        health.set_budgets(Budgets::unlimited());
        assert_eq!(g.import(ctx, &[0]).unwrap().records[0].message, "1 bytes");
        for _ in 0..QUARANTINE_AFTER {
            health.fail("b", "x");
        }
        assert!(g.import(ctx, &[0]).is_err());
    });
}

#[test]
fn budgets_labels_and_durations() {
    let b = Budgets::default();
    for kind in [
        CallKind::Sniff,
        CallKind::Import,
        CallKind::Tail,
        CallKind::Transform,
        CallKind::Hook,
        CallKind::Export,
        CallKind::Collector,
        CallKind::Annotate,
        CallKind::Service,
        CallKind::Event,
        CallKind::Lifecycle,
    ] {
        assert!(!kind.label().is_empty());
        assert_eq!(Budgets::unlimited().get(kind), None);
        let _ = b.get(kind);
    }
    assert_eq!(b.get(CallKind::Import), Some(Duration::from_secs(120)));
    let h = Health::default();
    assert_eq!(h.budgets(), Budgets::default());
    h.set_budgets(Budgets::unlimited());
    assert_eq!(h.budgets(), Budgets::unlimited());
    assert_eq!(b.get(CallKind::Export), None);
    assert_eq!(fmt_duration(Duration::from_secs(120)), "120 s");
    assert_eq!(fmt_duration(Duration::from_millis(1500)), "1.5 s");
    assert_eq!(fmt_duration(Duration::from_millis(50)), "50 ms");
}

#[test]
fn admission_refuses_hung_and_inactive_plugins_and_notes_are_kept() {
    let h = Arc::new(Health::default());
    let g = Guard::new("p", h.clone());
    assert_eq!(g.owner(), "p");
    h.mark_hung("p", "slow call");
    let e = g.call(CallKind::Service, "x", || Ok(())).unwrap_err().to_string();
    assert_eq!(e, "plugin \"p\" is not responding: slow call");
    assert!(h.get("p").hung);
    h.unhang("p");
    h.unhang("p"); // harmless
    h.unhang("other");
    assert!(!h.get("p").hung);
    h.set_blocked(HashMap::from([("p".to_string(), "needs x".to_string())]));
    assert_eq!(h.availability("p").unwrap_err(), "plugin \"p\" is inactive: needs x");
    // Lifecycle calls and settings (`run`) are still allowed while inactive.
    g.call(CallKind::Lifecycle, "activate", || Ok(())).unwrap();
    g.run("settings", || Ok(())).unwrap();
    assert!(format!("{h:?}").contains("needs x"));
    for i in 0..MAX_NOTES + 5 {
        h.note("p", &format!("note {i}"));
    }
    h.note("p", "note 24");
    let notes = h.notes("p");
    assert_eq!(notes.len(), MAX_NOTES);
    assert_eq!(notes.last().unwrap(), "note 24");
    h.reset("p");
    assert!(h.notes("p").is_empty());
}

#[test]
fn detached_calls_finish_time_out_or_report_panics() {
    let h = Arc::new(Health::default());
    let g = Guard::new("d", h.clone());
    h.set_budgets(Budgets {
        service: Some(Duration::from_millis(80)),
        ..Budgets::default()
    });
    assert_eq!(g.call_detached(CallKind::Service, "fast", || Ok(5)).unwrap(), 5);
    let e = quietly(|| g.call_detached::<()>(CallKind::Service, "boom", || panic!("detached exploded")))
        .unwrap_err()
        .to_string();
    assert!(e.contains("(boom) crashed: detached exploded"), "{e}");
    let e = g
        .call_detached(CallKind::Service, "slow", || {
            std::thread::sleep(Duration::from_millis(300));
            Ok(1)
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("did not finish within 80 ms; the call was abandoned"), "{e}");
    assert!(h.get("d").hung);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while h.get("d").hung && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!h.get("d").hung, "cleared when the abandoned call ends");
    // No budget: inline.
    h.set_budgets(Budgets::unlimited());
    assert_eq!(g.call_detached(CallKind::Service, "inline", || Ok(2)).unwrap(), 2);
}

#[test]
fn quarantine_notifies_the_observer_once() {
    let h = Health::default();
    let n = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = n.clone();
    h.set_observer(Arc::new(move || {
        c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));
    for _ in 0..QUARANTINE_AFTER + 2 {
        h.fail("p", "x");
    }
    assert_eq!(n.load(std::sync::atomic::Ordering::SeqCst), 1);
    for _ in 0..QUARANTINE_AFTER {
        h.mark_hung("h", "slow");
    }
    assert_eq!(n.load(std::sync::atomic::Ordering::SeqCst), 2);
    h.reset("p");
    assert_eq!(n.load(std::sync::atomic::Ordering::SeqCst), 3);
}
