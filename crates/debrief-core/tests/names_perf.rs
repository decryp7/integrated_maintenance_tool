//! Timings of the name layers on 50,000 invented entities. Ignored by default (timings, not behaviour); run with
//! `cargo test -p debrief-core --release --test names_perf -- --ignored --nocapture`.

use debrief_core::catalogue::{Candidate, Scope};
use debrief_core::entities::EntityKind;
use debrief_core::names::alias_job::{self, AliasJobConfig, AliasModel};
use debrief_core::names::build::vocabulary_from;
use debrief_core::names::group::group;
use debrief_core::names::normalise::normalise;
use debrief_core::names::resolve::{NameSettings, Resolver};
use debrief_core::names::{NameBook, SearchContext, Vocabulary};
use std::collections::HashSet;
use std::time::Instant;

const A: &[&str] = &[
    "graphic", "report", "tray", "backup", "media", "sync", "print", "disk", "event", "session", "video", "font", "mail", "cache",
    "device", "update",
];
const B: &[&str] = &[
    "builder",
    "agent",
    "worker",
    "player",
    "spooler",
    "indexer",
    "collector",
    "broker",
    "encoder",
    "relay",
    "cleaner",
    "watcher",
    "helper",
    "host",
    "service",
    "manager",
];

fn names(n: usize) -> Vec<Candidate> {
    (0..n)
        .map(|i| {
            let (a, b) = (A[i % 16], B[(i / 16) % 16]);
            let (c, d) = (A[(i / 256) % 16], B[(i / 4096) % 16]);
            let name = match i % 3 {
                0 => format!("{c}{d}{a}{b}.exe"),
                1 => format!("Acme.{c}.{d}.{a}.{b}.exe"),
                _ => format!("{c}-{d}-{a}-{b}_x64.exe"),
            };
            Candidate {
                scope: Scope::Entity(EntityKind::Executable),
                name,
                count: 1 + (i as u64 % 97),
                first_ts: None,
                last_ts: None,
                seen_in: vec![],
            }
        })
        .collect()
}

struct Echo;
impl AliasModel for Echo {
    fn version(&self) -> String {
        "echo".into()
    }
    fn complete(&self, prompt: &str) -> Result<String, String> {
        let items: Vec<String> = (1..=prompt.lines().filter(|l| l.contains(". [")).count())
            .map(|i| format!("{{\"id\":{i},\"name\":\"Thing {i}\",\"aliases\":[\"alias one\",\"alias two\"]}}"))
            .collect();
        Ok(format!("{{\"items\":[{}]}}", items.join(",")))
    }
}

#[test]
#[ignore]
fn timings_at_fifty_thousand_entities() {
    let cands = names(50_000);
    let vocab = vocabulary_from(&cands);
    let t = Instant::now();
    let vendors = HashSet::new();
    for c in &cands {
        std::hint::black_box(normalise(&c.name, &vocab, &vendors));
    }
    let per_name = t.elapsed() / cands.len() as u32;
    let t = Instant::now();
    let groups = group(&cands, &vocab);
    let grouping = t.elapsed();
    let mut book = NameBook::default();
    book.rebuild(groups);
    let n_groups = book.groups().len();
    let ctx = SearchContext::default();
    let r = Resolver {
        book: &book,
        vocab: &vocab,
        ctx: &ctx,
        settings: NameSettings::default(),
    };
    let mut worst = std::time::Duration::ZERO;
    let mut total = std::time::Duration::ZERO;
    let queries = [
        "graphic builder",
        "graphicbuilder",
        "grafic bilder",
        "builder",
        "zzzz qqqq",
        "tray agent 17",
    ];
    for q in queries {
        let t = Instant::now();
        std::hint::black_box(r.resolve(q, 5, None));
        let d = t.elapsed();
        worst = worst.max(d);
        total += d;
    }
    let cfg = AliasJobConfig {
        min_count: 1,
        max_groups: 2_000,
        pause: std::time::Duration::ZERO,
        ..AliasJobConfig::default()
    };
    let t = Instant::now();
    let rep = alias_job::run(&mut book, Some(&Echo), &cfg, &|| false);
    let job = t.elapsed();
    println!("segmentation+normalise per name: {per_name:?}");
    println!("grouping 50k names -> {n_groups} groups: {grouping:?}");
    println!(
        "resolve over {n_groups} groups: mean {:?}, worst {worst:?}",
        total / queries.len() as u32
    );
    println!(
        "alias job (fake model, excluding model time): {} groups in {job:?} = {:.0} groups/s",
        rep.stored,
        rep.stored as f64 / job.as_secs_f64()
    );
    let _ = Vocabulary::builtin();
}
