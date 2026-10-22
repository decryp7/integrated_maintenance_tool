//! The notes as text for the AI (ADR 0021 §6): the notes about what the user looks at first (the selected events,
//! the time range, the open image), then the project's own notes, then the rest in time order, within a byte
//! budget, with a visible line naming how many were left out. The shell hands the block to the assistant as one
//! more project knowledge file, so it gets the knowledge preamble and budgets (ADR 0029).

use crate::{Anchor, Note};
use chrono::{DateTime, SecondsFormat, Utc};

/// Most bytes of notes in one AI request.
pub const MAX_CONTEXT_BYTES: usize = 16 * 1024;
/// Most bytes of one note's text in the block.
const MAX_NOTE_BYTES: usize = 2 * 1024;

/// What the user looks at: notes about it come first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Focus {
    pub events: Vec<u64>,
    /// Epoch ms.
    pub range: Option<(i64, i64)>,
    /// The content key of the open image.
    pub image: Option<String>,
}

impl Focus {
    fn covers(&self, n: &Note) -> bool {
        if n.anchor.event_id().is_some_and(|id| self.events.contains(&id)) {
            return true;
        }
        if let (Some((from, to)), Some((a, b))) = (self.range, n.anchor.time_span()) {
            if a <= to && b >= from {
                return true;
            }
        }
        matches!((&n.anchor, &self.image), (Anchor::Image { image, .. }, Some(k)) if &image.key == k)
    }
}

fn time(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms).map_or_else(|| format!("{ms} ms"), |t| t.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// What a note is attached to, in a few words.
pub fn describe(anchor: &Anchor) -> String {
    match anchor {
        Anchor::Event {
            event_id,
            ts,
            source,
            excerpt,
        } => {
            let at = ts.map(|t| format!(" at {}", time(t))).unwrap_or_default();
            let src = if source.is_empty() {
                String::new()
            } else {
                format!(" in {source}")
            };
            format!("event #{event_id}{at}{src}: \"{excerpt}\"")
        }
        Anchor::Instant { at } => format!("the moment {}", time(*at)),
        Anchor::Range { from, to } => format!("the time range {} – {}", time(*from), time(*to)),
        Anchor::Source { path } => format!("the source {path}"),
        Anchor::Marker { name, from, to: None } => format!("the marker {name} at {}", time(*from)),
        Anchor::Marker { name, from, to: Some(to) } => format!("the span {name} {} – {}", time(*from), time(*to)),
        Anchor::ChartPoint { chart, series, at, value } => {
            let s = series.as_deref().map(|s| format!(" ({s})")).unwrap_or_default();
            let v = value.map(|v| format!(" = {v}")).unwrap_or_default();
            format!("the chart {chart}{s} at {}{v}", time(*at))
        }
        Anchor::Project => "the investigation as a whole".to_string(),
        Anchor::Image { image, regions } => {
            let areas: Vec<String> = regions.iter().map(|r| format!("{}×{} at {},{}", r.w, r.h, r.x, r.y)).collect();
            format!(
                "the image {} ({}×{} px), areas {}",
                image.name,
                image.width,
                image.height,
                areas.join("; ")
            )
        }
    }
}

fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn entry(n: &Note) -> String {
    let text = n.text.trim();
    let body = if text.len() > MAX_NOTE_BYTES {
        format!("{} […]", cut(text, MAX_NOTE_BYTES))
    } else {
        text.to_string()
    };
    let tags = if n.tags.is_empty() {
        String::new()
    } else {
        format!(" Tags: {}.", n.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "))
    };
    // Indented so a note's own Markdown headings and lists stay inside its entry.
    let body = body.lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n");
    format!("- [{}] On {}.{tags}\n{body}\n", n.severity.label(), describe(&n.anchor))
}

/// The block of notes for the AI within `max_bytes` (at most [`MAX_CONTEXT_BYTES`]); none without notes.
pub fn render(notes: &[Note], focus: &Focus, max_bytes: usize) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    let budget = max_bytes.min(MAX_CONTEXT_BYTES);
    let tier = |n: &Note| {
        if focus.covers(n) {
            0
        } else if matches!(n.anchor, Anchor::Project) {
            1
        } else {
            2
        }
    };
    let mut ordered: Vec<&Note> = notes.iter().collect();
    ordered.sort_by_key(|n| (tier(n), n.anchor.time_span().map_or(i64::MAX, |t| t.0), n.created_ms));
    let mut out = String::from("The user's notes in this project (their own observations; the ones about what they look at come first):\n");
    let reserve = 80;
    for (i, n) in ordered.iter().enumerate() {
        let e = entry(n);
        if out.len() + e.len() + reserve > budget {
            out.push_str(&format!("[… {} more notes were left out to fit]\n", ordered.len() - i));
            return Some(out);
        }
        out.push_str(&e);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{image_key, ImageRef, NoteSeverity, Region};

    fn note(id: &str, anchor: Anchor, text: &str) -> Note {
        Note {
            id: id.into(),
            anchor,
            text: text.into(),
            tags: vec![],
            severity: NoteSeverity::Info,
            author: String::new(),
            created_ms: 0,
            updated_ms: 0,
        }
    }

    fn img(key: &str) -> Anchor {
        Anchor::Image {
            image: ImageRef {
                key: key.into(),
                name: "screen.png".into(),
                path: None,
                width: 20,
                height: 10,
            },
            regions: vec![Region { x: 1, y: 2, w: 3, h: 4 }],
        }
    }

    #[test]
    fn no_notes_no_block() {
        assert_eq!(render(&[], &Focus::default(), 1000), None);
    }

    #[test]
    fn focused_notes_come_first_then_the_project_then_the_rest_by_time() {
        let k = image_key(b"x");
        let notes = vec![
            note("n1", Anchor::Instant { at: 50 }, "late"),
            note("n2", Anchor::Project, "overall"),
            note("n3", Anchor::Instant { at: 10 }, "early"),
            note(
                "n4",
                Anchor::Event {
                    event_id: 7,
                    ts: None,
                    source: "a.log".into(),
                    excerpt: "boom".into(),
                },
                "selected",
            ),
            note("n5", Anchor::Range { from: 100, to: 200 }, "in range"),
            note("n6", img(&k), "on the image"),
            note("n7", img(&image_key(b"y")), "other image"),
        ];
        let focus = Focus {
            events: vec![7],
            range: Some((150, 300)),
            image: Some(k),
        };
        let text = render(&notes, &focus, MAX_CONTEXT_BYTES).unwrap();
        let pos = |s: &str| text.find(s).unwrap();
        assert!(pos("selected") < pos("overall") && pos("in range") < pos("overall") && pos("on the image") < pos("overall"));
        assert!(pos("overall") < pos("early") && pos("early") < pos("late"));
        assert!(text.contains("event #7 in a.log: \"boom\""));
        assert!(text.contains("areas 3×4 at 1,2"));
    }

    #[test]
    fn the_block_is_bounded_and_says_what_was_left_out() {
        let notes: Vec<Note> = (0..100)
            .map(|i| note(&format!("n{i}"), Anchor::Instant { at: i }, &"word ".repeat(200)))
            .collect();
        let text = render(&notes, &Focus::default(), 4_000).unwrap();
        assert!(text.len() <= 4_000);
        assert!(text.contains("more notes were left out"));
        let huge = vec![note("n1", Anchor::Project, &"é".repeat(5_000))];
        let t = render(&huge, &Focus::default(), usize::MAX).unwrap();
        assert!(t.contains("[…]") && t.len() < 3 * 1024);
    }

    #[test]
    fn anchors_are_described() {
        let mut n = note("n1", Anchor::Project, "# Heading\n- item");
        n.tags = vec!["disk".into()];
        let e = entry(&n);
        assert!(e.contains("Tags: #disk.") && e.contains("\n  # Heading\n  - item"));
        assert_eq!(describe(&Anchor::Instant { at: 0 }), "the moment 1970-01-01T00:00:00.000Z");
        assert!(describe(&Anchor::Range { from: 0, to: 1 }).contains(" – "));
        assert_eq!(describe(&Anchor::Source { path: "/a.log".into() }), "the source /a.log");
        assert!(describe(&Anchor::Marker {
            name: "boot".into(),
            from: 0,
            to: None
        })
        .starts_with("the marker boot"));
        assert!(describe(&Anchor::Marker {
            name: "run".into(),
            from: 0,
            to: Some(5)
        })
        .starts_with("the span run"));
        let c = describe(&Anchor::ChartPoint {
            chart: "CPU".into(),
            series: Some("host-a".into()),
            at: 0,
            value: Some(93.5),
        });
        assert!(c.contains("CPU (host-a)") && c.ends_with("= 93.5"));
        assert!(describe(&Anchor::ChartPoint {
            chart: "CPU".into(),
            series: None,
            at: 0,
            value: None
        })
        .ends_with(".000Z"));
        let e = describe(&Anchor::Event {
            event_id: 1,
            ts: Some(0),
            source: String::new(),
            excerpt: "x".into(),
        });
        assert_eq!(e, "event #1 at 1970-01-01T00:00:00.000Z: \"x\"");
        assert_eq!(time(i64::MAX), format!("{} ms", i64::MAX));
    }
}
