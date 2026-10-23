//! Ranking candidate events against a screenshot's clues and time. Pure: the same candidates give the same
//! order, every link carries its reasons.
//!
//! Score = time + clues + level:
//! - time: 3 within the image time's precision, falling linearly to 0 at the edge of the window;
//! - each matched clue: exception name, error code or id 3; error text 2.5 (1.5 when most of its words match);
//!   title or application matching the source, the lane or a lane field 2; machine, user or server name 1.5;
//!   version 1;
//! - an error or fatal event +0.5 when the screen shows an error.
//!
//! An event needs a clue, or to be within the precision of the image's time, to be linked at all.

use super::{ClueKind, Clues, ImageTime, Reason, RelatedEvent, RelatedLink, RelatedNote, TitleTarget};
use crate::api::SourceInfo;
use crate::model::Level;
use std::cmp::Ordering;

const W_STRONG: f64 = 3.0;
const W_TEXT: f64 = 2.5;
const W_TEXT_PARTIAL: f64 = 1.5;
const W_TITLE: f64 = 2.0;
const W_NAME: f64 = 1.5;
const W_VERSION: f64 = 1.0;
const W_TIME: f64 = 3.0;
const W_ERROR: f64 = 0.5;
/// Fields whose values a title or application name can match (lane fields of the default profile and common
/// Serilog/NLog/Windows properties).
pub const LANE_FIELDS: &[&str] = &[
    "Application",
    "ApplicationName",
    "ProcessName",
    "Process",
    "MachineName",
    "Host",
    "ServiceName",
    "Service",
    "SourceContext",
    "Logger",
    "Provider",
    "Channel",
];
/// Words of a window title that say nothing about which program it is.
const TITLE_NOISE: &[&str] = &[
    "error",
    "warning",
    "exception",
    "failed",
    "failure",
    "the",
    "and",
    "not",
    "responding",
    "exe",
    "window",
    "dialog",
    "microsoft",
    "windows",
    "information",
    "critical",
    "fatal",
    "unhandled",
    "application",
    "message",
    "alert",
    "with",
    "for",
    "has",
    "stopped",
    "working",
];

/// "±1.4 s", "+3 min 5 s", "−2 h 10 min": how far an event is from the image's time.
pub fn format_delta(ms: i64) -> String {
    let sign = match ms.cmp(&0) {
        Ordering::Less => "−",
        Ordering::Greater => "+",
        Ordering::Equal => "±",
    };
    let a = ms.unsigned_abs();
    if a < 60_000 {
        let s = a as f64 / 1000.0;
        if a.is_multiple_of(1000) {
            format!("{sign}{} s", a / 1000)
        } else {
            format!("{sign}{s:.1} s")
        }
    } else if a < 3_600_000 {
        format!("{sign}{} min {} s", a / 60_000, (a % 60_000) / 1000)
    } else if a < 86_400_000 {
        format!("{sign}{} h {} min", a / 3_600_000, (a % 3_600_000) / 60_000)
    } else {
        format!("{sign}{} d {} h", a / 86_400_000, (a % 86_400_000) / 3_600_000)
    }
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.chars().count() >= 3)
        .map(str::to_lowercase)
        .collect()
}

/// The distinctive words of a title or application name.
fn title_words(clues: &Clues) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in [&clues.application, &clues.window_title].into_iter().flatten() {
        for w in words(t) {
            if !TITLE_NOISE.contains(&w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()) && !out.contains(&w) {
                out.push(w);
            }
        }
    }
    out
}

/// Clues worth a search of their own, strongest first: exception names, error codes, ids, error texts (cut
/// to their first 80 characters at a word boundary), then the application name.
pub fn search_terms(clues: &Clues) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let short_names = clues.exception_names.iter().map(|n| n.rsplit('.').next().unwrap_or(n).to_string());
    let texts = clues.error_texts.iter().map(|t| {
        if t.chars().count() <= 80 {
            t.clone()
        } else {
            let cut: String = t.chars().take(80).collect();
            cut.rsplit_once(' ').map(|(a, _)| a.to_string()).unwrap_or(cut)
        }
    });
    let all = short_names
        .chain(clues.error_codes.iter().cloned())
        .chain(clues.ids.iter().cloned())
        .chain(texts)
        .chain(clues.application.iter().cloned());
    for t in all {
        let t = t.trim().to_string();
        if t.chars().count() >= 3 && !out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
    }
    out
}

/// Ids of the sources whose name or group holds a distinctive word of the title or application.
pub fn title_sources(clues: &Clues, sources: &[SourceInfo]) -> Vec<u32> {
    let tw = title_words(clues);
    sources
        .iter()
        .filter(|s| {
            let names = [words(&s.name), words(&s.group)].concat();
            tw.iter().any(|w| names.contains(w))
        })
        .map(|s| s.id)
        .collect()
}

fn haystack(e: &RelatedEvent) -> String {
    let mut h = norm(&e.message);
    if let Some(x) = &e.exception {
        h.push('\n');
        h.push_str(&norm(x));
    }
    h
}

fn field_text(e: &RelatedEvent) -> String {
    e.fields
        .values()
        .map(|v| v.as_str().map(norm).unwrap_or_else(|| norm(&v.to_string())))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Most words of `clue` (the distinctive ones) appear in `hay`.
fn mostly(hay: &str, clue: &str) -> bool {
    let w = words(clue);
    if w.len() < 3 {
        return false;
    }
    let found = w.iter().filter(|x| hay.contains(x.as_str())).count();
    found * 10 >= w.len() * 7
}

fn title_target(e: &RelatedEvent, tw: &[String], sources: &[SourceInfo]) -> Option<(String, TitleTarget)> {
    let src = sources.iter().find(|s| s.id == e.source);
    for w in tw {
        if let Some(s) = src {
            if words(&s.name).contains(w) || words(&s.group).contains(w) {
                return Some((w.clone(), TitleTarget::Source { name: s.name.clone() }));
            }
        }
        if words(&e.group).contains(w) {
            return Some((w.clone(), TitleTarget::Lane { lane: e.group.clone() }));
        }
        for f in LANE_FIELDS {
            if let Some(v) = e.fields.get(*f).and_then(|v| v.as_str()) {
                if words(v).contains(w) {
                    return Some((
                        w.clone(),
                        TitleTarget::Field {
                            field: (*f).to_string(),
                            value: v.chars().take(80).collect(),
                        },
                    ));
                }
            }
        }
    }
    None
}

/// Scores one event: its score and reasons, or `None` when nothing links it.
fn score(
    e: &RelatedEvent,
    clues: &Clues,
    tw: &[String],
    t: Option<&ImageTime>,
    window: i64,
    sources: &[SourceInfo],
    prefer_errors: bool,
) -> Option<(f64, Vec<Reason>)> {
    let hay = haystack(e);
    let fields = field_text(e);
    let mut reasons = Vec::new();
    let mut facts = 0.0;
    let mut text = |list: &[String], kind: ClueKind, w: f64, in_fields: bool, reasons: &mut Vec<Reason>| {
        for c in list {
            let n = norm(c);
            let key = match kind {
                ClueKind::ExceptionName => n.rsplit('.').next().unwrap_or(&n).to_string(),
                _ => n.clone(),
            };
            if key.chars().count() < 2 {
                continue;
            }
            if hay.contains(&key) || (in_fields && fields.contains(&key)) {
                facts += w;
                reasons.push(Reason::Text {
                    clue: c.clone(),
                    clue_kind: kind,
                    partial: false,
                });
            } else if kind == ClueKind::ErrorText && mostly(&hay, c) {
                facts += W_TEXT_PARTIAL;
                reasons.push(Reason::Text {
                    clue: c.clone(),
                    clue_kind: kind,
                    partial: true,
                });
            }
        }
    };
    text(&clues.exception_names, ClueKind::ExceptionName, W_STRONG, false, &mut reasons);
    text(&clues.error_codes, ClueKind::ErrorCode, W_STRONG, true, &mut reasons);
    text(&clues.ids, ClueKind::Id, W_STRONG, true, &mut reasons);
    text(&clues.error_texts, ClueKind::ErrorText, W_TEXT, false, &mut reasons);
    text(&clues.machine_names, ClueKind::Machine, W_NAME, true, &mut reasons);
    text(&clues.user_names, ClueKind::User, W_NAME, true, &mut reasons);
    text(&clues.server_names, ClueKind::Server, W_NAME, true, &mut reasons);
    text(&clues.versions, ClueKind::Version, W_VERSION, true, &mut reasons);
    if let Some((text, target)) = title_target(e, tw, sources) {
        facts += W_TITLE;
        reasons.insert(0, Reason::Title { text, target });
    }
    let mut total = facts;
    let mut close = false;
    if let (Some(t), Some(ts)) = (t, e.ts) {
        let delta = ts.saturating_sub(t.at_ms);
        let d = delta.saturating_abs();
        let slack = t.precision_ms.max(1_000);
        if d <= window.max(slack) {
            let w = if d <= slack {
                close = true;
                W_TIME
            } else {
                W_TIME * (1.0 - (d - slack) as f64 / (window - slack).max(1) as f64).max(0.0)
            };
            total += w;
            reasons.insert(0, Reason::Time { delta_ms: delta });
        }
    }
    if facts == 0.0 && !close {
        return None;
    }
    if prefer_errors && matches!(e.level, Level::Error | Level::Fatal) {
        total += W_ERROR;
    }
    Some((total, reasons))
}

/// Ranks `candidates` (best first). Ties go to the event nearer the image's time, then the lower id.
pub fn rank(
    candidates: Vec<RelatedEvent>,
    clues: &Clues,
    t: Option<&ImageTime>,
    window: i64,
    sources: &[SourceInfo],
    prefer_errors: bool,
) -> Vec<RelatedLink> {
    let tw = title_words(clues);
    let mut links: Vec<RelatedLink> = candidates
        .into_iter()
        .filter_map(|e| {
            score(&e, clues, &tw, t, window, sources, prefer_errors).map(|(score, reasons)| RelatedLink {
                event: e,
                score: (score * 100.0).round() / 100.0,
                reasons,
            })
        })
        .collect();
    let dist = |l: &RelatedLink| match (t, l.event.ts) {
        (Some(t), Some(ts)) => ts.saturating_sub(t.at_ms).saturating_abs(),
        _ => i64::MAX,
    };
    links.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| dist(a).cmp(&dist(b)))
            .then_with(|| a.event.id.cmp(&b.event.id))
    });
    links
}

fn strong(l: &RelatedLink) -> bool {
    l.reasons.iter().any(|r| {
        matches!(
            r,
            Reason::Text {
                clue_kind: ClueKind::ExceptionName | ClueKind::ErrorCode | ClueKind::Id | ClueKind::ErrorText,
                partial: false,
                ..
            }
        )
    })
}

/// The clocks likely disagree: the best link holding a strong clue (exception, code, id, error text) is outside
/// the window of the image's time. The note names its distance and source (`links` are ranked, best first).
pub fn clock_disagreement(links: &[RelatedLink], t: &ImageTime, window: i64) -> Option<RelatedNote> {
    let best = links.iter().find(|l| strong(l) && l.event.ts.is_some())?;
    let delta = best.event.ts?.saturating_sub(t.at_ms);
    (delta.saturating_abs() > window.max(t.precision_ms)).then(|| RelatedNote::ClockDisagreement {
        delta_ms: delta,
        source_id: best.event.source,
        source_name: best.event.source_name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_read_well() {
        assert_eq!(format_delta(1_400), "+1.4 s");
        assert_eq!(format_delta(-2_000), "−2 s");
        assert_eq!(format_delta(0), "±0 s");
        assert_eq!(format_delta(185_000), "+3 min 5 s");
        assert_eq!(format_delta(-7_800_000), "−2 h 10 min");
        assert_eq!(format_delta(90_000_000), "+1 d 1 h");
        assert_eq!(format_delta(i64::MIN).chars().next(), Some('−'));
    }

    #[test]
    fn search_terms_put_the_strong_clues_first() {
        let c = Clues {
            application: Some("OrderWorker".into()),
            exception_names: vec!["System.Net.Http.HttpRequestException".into()],
            error_codes: vec!["0x80004005".into(), "0X80004005".into()],
            ids: vec!["ORD-1042".into()],
            error_texts: vec![format!("{} tail", "word ".repeat(30))],
            ..Clues::default()
        };
        let t = search_terms(&c);
        assert_eq!(t[0], "HttpRequestException");
        assert_eq!(t[1], "0x80004005");
        assert_eq!(t[2], "ORD-1042");
        assert!(t[3].chars().count() <= 80 && !t[3].ends_with(' '));
        assert_eq!(t[4], "OrderWorker");
        assert_eq!(t.len(), 5);
        assert!(search_terms(&Clues::default()).is_empty());
    }

    fn candidate(id: u64, message: &str, exception: Option<&str>) -> RelatedEvent {
        RelatedEvent {
            id,
            ts: Some(1_000),
            level: Level::Error,
            message: message.into(),
            exception: exception.map(Into::into),
            source: 0,
            source_name: "app.log".into(),
            group: String::new(),
            fields: Default::default(),
        }
    }

    #[test]
    fn a_clue_is_found_in_the_exception_text_and_a_one_letter_name_is_no_clue() {
        let clues = Clues {
            exception_names: vec!["System.TimeoutException".into()],
            ..Clues::default()
        };
        let links = rank(
            vec![
                candidate(1, "request failed", Some("System.TimeoutException: the operation timed out")),
                candidate(2, "request failed", None),
            ],
            &clues,
            None,
            0,
            &[],
            false,
        );
        assert_eq!(
            links.iter().map(|l| l.event.id).collect::<Vec<_>>(),
            [1],
            "only the exception text carries it"
        );
        // The last name part after the dots is the clue; one letter proves nothing and is skipped.
        let short = Clues {
            exception_names: vec!["Company.X".into()],
            ..Clues::default()
        };
        assert!(rank(vec![candidate(1, "x marks the spot", Some("X"))], &short, None, 0, &[], false).is_empty());
    }
}
