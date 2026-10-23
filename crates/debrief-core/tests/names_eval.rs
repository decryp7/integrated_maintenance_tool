//! Evaluation of human-like name matching on a labelled set of invented names: spelling variants, run-together
//! words, abbreviations, typos, synonyms and genuine ambiguities. Reports per layer top-1 accuracy, top-3 recall and the
//! false-suggestion rate (a wrong entity suggested without a question), and asserts the numbers do not regress.
//! Layer 2 uses fixed alias fixtures in place of a model, so the run is reproducible and offline.
//! Run `cargo test -p debrief-core --test names_eval -- --nocapture` to see the table.

use debrief_core::catalogue::{Candidate, Scope};
use debrief_core::entities::EntityKind;
use debrief_core::names::build::vocabulary_from;
use debrief_core::names::group::group;
use debrief_core::names::resolve::{NameSettings, Resolver};
use debrief_core::names::{NameBook, SearchContext, Vocabulary};
use debrief_core::names::book::AiEntry;
use debrief_core::words::{phrase_in, WordQuery};
use std::collections::HashSet;

struct Ent {
    words: &'static str,
    spellings: &'static [&'static str],
    typo: &'static str,
    abbrev: &'static str,
    synonym: &'static str,
}

const ENTS: &[Ent] = &[
    Ent { words: "graphic builder", spellings: &["CompanyA.Graphic.Builder.exe", "graphicbuilder.exe", "graphic-builder.exe"], typo: "grafic bilder", abbrev: "gfxbldr", synonym: "image editor" },
    Ent { words: "tray agent", spellings: &["CompanyA.Tray.Agent.exe", "trayagent.exe"], typo: "trai agent", abbrev: "tryagnt", synonym: "system tray helper" },
    Ent { words: "report builder", spellings: &["reportbuilder.exe", "CompanyA.Report.Builder.exe"], typo: "reprot builder", abbrev: "rptbldr", synonym: "pdf generator" },
    Ent { words: "print spooler", spellings: &["printspooler.exe", "print_spooler.exe"], typo: "print spoller", abbrev: "prtspl", synonym: "printing service" },
    Ent { words: "update helper", spellings: &["update-helper.exe", "updatehelper.exe"], typo: "updte helper", abbrev: "updhlp", synonym: "auto updater" },
    Ent { words: "sync worker", spellings: &["syncworker.exe", "Sync.Worker.exe"], typo: "sync wroker", abbrev: "synwrk", synonym: "cloud sync" },
    Ent { words: "crash reporter", spellings: &["crashreporter.exe", "CompanyA.Crash.Reporter.exe"], typo: "crash reportr", abbrev: "crshrpt", synonym: "error uploader" },
    Ent { words: "media player", spellings: &["mediaplayer.exe", "media-player.exe"], typo: "meida player", abbrev: "mplayr", synonym: "video viewer" },
    Ent { words: "backup agent", spellings: &["backupagent.exe", "Backup.Agent.exe"], typo: "bakup agent", abbrev: "bkpagt", synonym: "archive service" },
    Ent { words: "license checker", spellings: &["licensechecker.exe", "license_checker.exe"], typo: "license chekcer", abbrev: "lickchk", synonym: "activation validator" },
    Ent { words: "config loader", spellings: &["configloader.exe", "config-loader.exe"], typo: "confg loader", abbrev: "cfgldr", synonym: "settings reader" },
    Ent { words: "network monitor", spellings: &["networkmonitor.exe", "NetworkMonitor.exe"], typo: "netwrok monitor", abbrev: "netmon", synonym: "traffic watcher" },
    Ent { words: "disk indexer", spellings: &["diskindexer.exe", "disk-indexer.exe"], typo: "disk indxer", abbrev: "dskidx", synonym: "file crawler" },
    Ent { words: "event collector", spellings: &["eventcollector.exe", "Event.Collector.exe"], typo: "event colector", abbrev: "evtcol", synonym: "log gatherer" },
    Ent { words: "session broker", spellings: &["sessionbroker.exe", "session_broker.exe"], typo: "sesion broker", abbrev: "sessbrk", synonym: "login manager" },
    Ent { words: "video encoder", spellings: &["videoencoder.exe", "Video.Encoder.exe"], typo: "vidio encoder", abbrev: "videnc", synonym: "movie converter" },
    Ent { words: "search service", spellings: &["searchservice.exe", "search-service.exe"], typo: "serch service", abbrev: "srchsvc", synonym: "finder daemon" },
    Ent { words: "font cache", spellings: &["fontcache.exe", "font_cache.exe"], typo: "font cahce", abbrev: "fntcache", synonym: "glyph store" },
    Ent { words: "thumbnail host", spellings: &["thumbnailhost.exe", "Thumbnail.Host.exe"], typo: "thumbnial host", abbrev: "thumbhst", synonym: "preview renderer" },
    Ent { words: "device watcher", spellings: &["devicewatcher.exe", "device-watcher.exe"], typo: "devise watcher", abbrev: "devwtch", synonym: "hardware sentinel" },
    Ent { words: "spell checker", spellings: &["spellchecker.exe", "spell-checker.exe"], typo: "spel checker", abbrev: "splchk", synonym: "grammar tool" },
    Ent { words: "cache cleaner", spellings: &["cachecleaner.exe", "Cache.Cleaner.exe"], typo: "cach cleaner", abbrev: "cchcln", synonym: "temp purger" },
    Ent { words: "mail relay", spellings: &["mailrelay.exe", "mail_relay.exe"], typo: "mail rellay", abbrev: "mlrelay", synonym: "smtp forwarder" },
    Ent { words: "window manager", spellings: &["windowmanager.exe", "Window.Manager.exe"], typo: "windo manager", abbrev: "wndmgr", synonym: "desktop compositor" },
    // Distractors that make "builder", "agent", "player", "monitor", "manager" and "indexer" genuinely ambiguous.
    Ent { words: "audio player", spellings: &["audioplayer.exe"], typo: "", abbrev: "", synonym: "" },
    Ent { words: "event monitor", spellings: &["eventmonitor.exe"], typo: "", abbrev: "", synonym: "" },
    Ent { words: "task manager", spellings: &["taskmanager.exe"], typo: "", abbrev: "", synonym: "" },
    Ent { words: "search indexer", spellings: &["searchindexer.exe"], typo: "", abbrev: "", synonym: "" },
];

const NEGATIVES: &[&str] = &[
    "quantum flux", "banana", "database", "zzzz", "red balloon", "xylophone", "kernel panic", "weather report", "coffee machine",
    "blue sky", "lorem ipsum", "monkey", "violin", "hockey stick", "mountain", "tablecloth", "grape builder", "tree agent",
];

/// (query, ids the user may mean when context is absent, inspected text, expected id when the context decides)
const AMBIGUOUS: &[(&str, &[&str], &str, &str)] = &[
    ("builder", &["graphic builder", "report builder"], "crash in reportbuilder.exe at 10:01", "report builder"),
    ("agent", &["tray agent", "backup agent"], "backupagent.exe stopped", "backup agent"),
    ("player", &["media player", "audio player"], "audioplayer.exe opened device", "audio player"),
    ("monitor", &["network monitor", "event monitor"], "eventmonitor.exe lagging", "event monitor"),
    ("manager", &["window manager", "task manager"], "taskmanager.exe not responding", "task manager"),
    ("indexer", &["disk indexer", "search indexer"], "searchindexer.exe started", "search indexer"),
];

/// Harder phrasings that no layer is built for: plurals, extra words, word order, partial abbreviations, a typo of
/// an AI alias, and look-alike words. They keep the numbers honest.
const HARD: &[(&str, Option<&str>)] = &[
    ("graphics builder", Some("graphic builder")),
    ("the graphic builder", Some("graphic builder")),
    ("builder graphic", Some("graphic builder")),
    ("gfx builder", Some("graphic builder")),
    ("image editr", Some("graphic builder")),
    ("trayagent v3", Some("tray agent")),
    ("tray-agent x64", Some("tray agent")),
    ("backup", Some("backup agent")),
    ("spool", Some("print spooler")),
    ("printer service", None),
    ("sync", Some("sync worker")),
    ("mail", Some("mail relay")),
    ("video", Some("video encoder")),
    ("license server", None),
];

/// Feedback the user gave earlier: (term, entity words); queries by those terms must resolve to that entity.
const TAUGHT: &[(&str, &str)] = &[("pix", "graphic builder"), ("pdf maker", "report builder"), ("tray thing", "tray agent")];

fn cand(name: &str, count: u64) -> Candidate {
    Candidate {
        scope: Scope::Entity(EntityKind::Executable),
        name: name.into(),
        count,
        first_ts: Some(100),
        last_ts: Some(900),
        seen_in: vec!["app.log".into()],
    }
}

fn all_cands() -> Vec<Candidate> {
    let mut cands = Vec::new();
    for (i, e) in ENTS.iter().enumerate() {
        for (j, s) in e.spellings.iter().enumerate() {
            cands.push(cand(s, 40 + (i as u64 * 7) % 31 + 10 / (j as u64 + 1)));
        }
    }
    cands
}

fn book(with_ai: bool, taught: bool) -> NameBook {
    let cands = all_cands();
    let mut b = NameBook::default();
    b.rebuild(group(&cands, &vocabulary_from(&cands)));
    if with_ai {
        for e in ENTS.iter().filter(|e| !e.abbrev.is_empty()) {
            let id = format!("executable|{}", e.words);
            b.set_ai(&id, AiEntry { model: "fixture".into(), human: title(e.words), aliases: vec![e.abbrev.into(), e.synonym.into()] });
        }
    }
    if taught {
        for (term, words) in TAUGHT {
            b.accept(term, &format!("executable|{words}"), false);
        }
    }
    b
}

fn title(s: &str) -> String {
    s.split(' ').map(|w| w[..1].to_uppercase() + &w[1..]).collect::<Vec<_>>().join(" ")
}

/// What a query should give.
enum Expect {
    One(&'static str),
    Nothing,
    /// Several entities tie: a question is right, a guess is a false suggestion.
    Tie(&'static [&'static str]),
}

struct Q {
    text: String,
    expect: Expect,
    kind: &'static str,
    inspected: Option<&'static str>,
}

fn queries() -> Vec<Q> {
    let mut v = Vec::new();
    for e in ENTS.iter().filter(|e| !e.abbrev.is_empty()) {
        let mk = |t: &str, kind| Q { text: t.into(), expect: Expect::One(e.words), kind, inspected: None };
        v.push(mk(e.words, "spaced"));
        v.push(mk(&e.words.replace(' ', ""), "joined"));
        v.push(mk(e.typo, "typo"));
        v.push(mk(e.abbrev, "abbreviation"));
        v.push(mk(e.synonym, "synonym"));
    }
    for n in NEGATIVES {
        v.push(Q { text: (*n).into(), expect: Expect::Nothing, kind: "negative", inspected: None });
    }
    for (t, ids, _, _) in AMBIGUOUS {
        v.push(Q { text: (*t).into(), expect: Expect::Tie(ids), kind: "ambiguous", inspected: None });
    }
    for (t, _, ins, id) in AMBIGUOUS {
        v.push(Q { text: (*t).into(), expect: Expect::One(id), kind: "context", inspected: Some(ins) });
    }
    for (t, e) in HARD {
        let expect = match e {
            Some(id) => Expect::One(id),
            None => Expect::Nothing,
        };
        v.push(Q { text: (*t).into(), expect, kind: "hard", inspected: None });
    }
    v.push(Q { text: "cache".into(), expect: Expect::Tie(&["font cache", "cache cleaner"]), kind: "ambiguous", inspected: None });
    for (t, id) in TAUGHT {
        v.push(Q { text: (*t).into(), expect: Expect::One(id), kind: "taught", inspected: None });
    }
    v
}

#[derive(Default, Debug, Clone)]
struct Score {
    n: usize,
    top1: usize,
    top3: usize,
    false_suggestions: usize,
    questions: usize,
    asked_on_non_ties: usize,
}

impl Score {
    fn pct(a: usize, b: usize) -> f64 {
        if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 }
    }
}

/// One answer: the entity words of the best matches, whether a question was asked.
struct Answer {
    ids: Vec<String>,
    asked: bool,
}

fn score(qs: &[Q], mut answer: impl FnMut(&Q) -> Answer, only: &[&str]) -> Score {
    let mut s = Score::default();
    for q in qs.iter().filter(|q| only.contains(&q.kind)) {
        let a = answer(q);
        s.n += 1;
        match &q.expect {
            Expect::One(id) => {
                s.top1 += usize::from(a.ids.first().is_some_and(|x| x == id) && !a.asked);
                s.top3 += usize::from(a.ids.iter().take(3).any(|x| x == id));
                if a.asked {
                    s.asked_on_non_ties += 1;
                }
                if !a.asked && a.ids.first().is_some_and(|x| x != id) {
                    s.false_suggestions += 1;
                }
            }
            Expect::Nothing => {
                s.top1 += usize::from(a.ids.is_empty());
                s.top3 += usize::from(a.ids.is_empty());
                if !a.ids.is_empty() && !a.asked {
                    s.false_suggestions += 1;
                }
            }
            Expect::Tie(ids) => {
                let right = a.asked && a.ids.iter().take(ids.len()).all(|x| ids.contains(&x.as_str()));
                s.top1 += usize::from(right);
                s.top3 += usize::from(ids.iter().all(|i| a.ids.iter().take(3).any(|x| x == i)));
                s.questions += usize::from(a.asked);
                if !a.asked && !a.ids.is_empty() {
                    s.false_suggestions += 1;
                }
            }
        }
    }
    s
}

fn entity_of(id: &str) -> String {
    id.strip_prefix("executable|").unwrap_or(id).to_string()
}

fn layered(b: &NameBook, settings: NameSettings, use_context: bool) -> impl FnMut(&Q) -> Answer + '_ {
    let vocab: Vocabulary = vocabulary_from(&all_cands());
    move |q| {
        let ctx = match (use_context, q.inspected) {
            (true, Some(t)) => SearchContext { inspected: Some(t.into()), ..SearchContext::default() },
            _ => SearchContext::default(),
        };
        let r = Resolver { book: b, vocab: &vocab, ctx: &ctx, settings }.resolve(&q.text, 5, None);
        Answer { ids: r.matches.iter().map(|m| entity_of(&m.id)).collect(), asked: r.clarify.is_some() }
    }
}

/// The matching before the name layer: spellings as the catalogue lists them, no segmentation, no groups.
fn baseline(b: &NameBook) -> impl FnMut(&Q) -> Answer + '_ {
    move |q| {
        let words = WordQuery::parse(&q.text);
        let squash = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
        let qs = squash(&q.text);
        let tri = |s: &str| -> HashSet<String> { s.chars().collect::<Vec<_>>().windows(3).map(|w| w.iter().collect()).collect() };
        let wanted = tri(&qs);
        let mut hits: Vec<(u8, u64, String)> = Vec::new();
        for g in b.groups() {
            for s in &g.spellings {
                let sq = squash(&s.name);
                let class = if sq == qs {
                    0
                } else if phrase_in(&q.text, &s.name) {
                    1
                } else if words.matches(&[s.name.as_str()]) {
                    2
                } else if wanted.len() >= 4 && (tri(&sq).iter().filter(|t| wanted.contains(*t)).count() as f64 / wanted.len() as f64) >= 0.5 {
                    3
                } else {
                    continue;
                };
                hits.push((class, s.count, entity_of(&g.id())));
            }
        }
        let best = hits.iter().map(|h| h.0).min();
        hits.retain(|h| Some(h.0) == best && (h.0 < 3 || best == Some(3)));
        hits.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let mut ids: Vec<String> = Vec::new();
        for h in hits {
            if !ids.contains(&h.2) {
                ids.push(h.2);
            }
        }
        Answer { ids, asked: false }
    }
}

const ENTITY_KINDS: &[&str] = &["spaced", "joined", "typo", "abbreviation", "synonym", "taught", "context"];
const ALL_KINDS: &[&str] = &["hard", "spaced", "joined", "typo", "abbreviation", "synonym", "taught", "context", "negative", "ambiguous"];

fn report(name: &str, s: &Score) {
    println!(
        "{name:<34} n={:<3} top-1 {:>5.1}%  top-3 {:>5.1}%  false suggestions {:>5.1}% ({})  questions {}",
        s.n,
        Score::pct(s.top1, s.n),
        Score::pct(s.top3, s.n),
        Score::pct(s.false_suggestions, s.n),
        s.false_suggestions,
        s.questions
    );
}

#[test]
fn evaluation() {
    let qs = queries();
    assert!(qs.len() >= 100, "{}", qs.len());
    let no_ai = NameSettings { ai_aliases: false, context: false };
    let ai = NameSettings { ai_aliases: true, context: false };
    let full = NameSettings::default();
    let plain = book(false, false);
    let with_ai = book(true, false);
    let learned = book(true, true);

    println!("\n{} queries over {} entities", qs.len(), ENTS.len());
    let base = score(&qs, baseline(&plain), ALL_KINDS);
    let l1 = score(&qs, layered(&plain, no_ai, false), ALL_KINDS);
    let l12 = score(&qs, layered(&with_ai, ai, false), ALL_KINDS);
    let all = score(&qs, layered(&learned, full, true), ALL_KINDS);
    report("baseline (spellings, no layers)", &base);
    report("layer 1 (segmentation, groups)", &l1);
    report("layer 1 + 2 (AI aliases)", &l12);
    report("combined (+ context + feedback)", &all);
    for kind in ALL_KINDS {
        let k = [*kind];
        report(&format!("  by kind: {kind} (combined)"), &score(&qs, layered(&learned, full, true), &k));
    }
    if std::env::var("NAMES_EVAL_MISSES").is_ok() {
        let mut f = layered(&learned, full, true);
        for q in &qs {
            let a = f(q);
            let ok = match &q.expect {
                Expect::One(id) => a.ids.first().is_some_and(|x| x == id) && !a.asked,
                Expect::Nothing => a.ids.is_empty(),
                Expect::Tie(_) => a.asked,
            };
            if !ok {
                println!("  miss [{}] '{}': got {:?} asked={}", q.kind, q.text, a.ids, a.asked);
            }
        }
    }
    let ents = score(&qs, layered(&learned, full, true), ENTITY_KINDS);
    report("combined, entity queries only", &ents);

    // Layer 1 is what makes spaced/joined/typo variants work without any model.
    for kind in ["spaced", "joined", "typo"] {
        let k = [kind];
        let s = score(&qs, layered(&plain, no_ai, false), &k);
        assert!(Score::pct(s.top1, s.n) >= 95.0, "layer 1 {kind}: {s:?}");
    }
    // Abbreviations and synonyms need layer 2; without it they find nothing, never a wrong entity.
    let k = ["abbreviation", "synonym"];
    let without = score(&qs, layered(&plain, no_ai, false), &k);
    let with = score(&qs, layered(&with_ai, ai, false), &k);
    assert_eq!(without.false_suggestions, 0, "{without:?}");
    assert!(without.top1 <= without.n / 4, "{without:?}");
    assert!(Score::pct(with.top1, with.n) >= 95.0, "{with:?}");
    // The combined result is better than each step before it and its false suggestions stay low.
    assert!(l1.top1 >= base.top1, "layer 1 never answers fewer queries than the baseline");
    assert!(l1.false_suggestions * 4 <= base.false_suggestions, "layer 1 cuts false suggestions to a quarter or less");
    assert!(Score::pct(l12.top1, l12.n) > Score::pct(l1.top1, l1.n) + 15.0);
    assert!(all.top1 >= l12.top1);
    assert!(Score::pct(all.false_suggestions, all.n) <= 5.0, "{all:?}");
    assert!(Score::pct(ents.top1, ents.n) >= 97.0, "{ents:?}");
    assert!(Score::pct(all.top3, all.n) >= 97.0, "{all:?}");
    // Questions are asked on the true ties and nowhere else.
    assert_eq!(all.asked_on_non_ties, 0, "{all:?}");
    let ties = score(&qs, layered(&learned, full, true), &["ambiguous"]);
    assert_eq!(ties.questions, ties.n, "{ties:?}");
    assert_eq!(ties.top1, ties.n, "{ties:?}");
    // Context turns a tie into the right answer.
    let ctx = score(&qs, layered(&learned, full, true), &["context"]);
    assert_eq!(ctx.top1, ctx.n, "{ctx:?}");
    if std::env::var("NAMES_EVAL_MISSES").is_ok() {
        let mut f = layered(&learned, full, false);
        for q in qs.iter().filter(|q| q.kind == "context") {
            let a = f(q);
            println!("  no-context '{}': {:?} asked={}", q.text, a.ids, a.asked);
        }
    }
    let no_ctx = score(&qs, layered(&learned, full, false), &["context"]);
    assert_eq!(no_ctx.top1, 0, "without context these are ties: {no_ctx:?}");
}
