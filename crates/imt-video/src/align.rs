//! When a video started, in the logs' clock (ADR 0032 §5). Pure: from the evidence in order of trust — what the user
//! set, a clock on screen the vision model read, a date and time in the file name (screen recorders name files by
//! their start), the container's `creation_time`, the file's modification time minus the duration — each with where it
//! came from and how far it can be trusted. Times without a zone are read in the profile's zone, like screenshots
//! (`imt_core::related::time`).

use imt_core::importers::video::{AlignedBy, Confidence};
use imt_core::related::time::resolve;
use imt_core::related::{ShownTime, TimeEvidence, TimeSource};
use serde::{Deserialize, Serialize};

/// The user's say over the alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UserAlignment {
    /// The automatic alignment.
    #[default]
    Auto,
    /// Video time 0 is this moment (epoch ms).
    Start {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        start_ms: i64,
    },
    /// The automatic alignment shifted by this much (ms; the recording machine's clock was off).
    Offset {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        offset_ms: i64,
    },
}

/// A clock the vision model read on one frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ClockReading {
    /// The frame's time in the video.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub frame_ms: u64,
    /// The times the model saw, as shown.
    #[cfg_attr(feature = "ts", ts(type = "Array<{ text: string; hasDate: boolean }>"))]
    pub shown: Vec<ShownTime>,
}

/// What the alignment is based on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AlignEvidence {
    /// The user set the start.
    UserStart,
    /// The user shifted the automatic alignment (which `base` describes).
    UserOffset {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        offset_ms: i64,
        base: Box<AlignEvidence>,
    },
    /// The clock in the frame at `frame_ms` read `text`.
    Clock {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        frame_ms: u64,
        text: String,
        has_date: bool,
    },
    /// The file name says `text`.
    FileName { text: String },
    /// The container says the video was created then.
    Container {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        created_ms: i64,
    },
    /// The file was last written then (taken as the end of the recording).
    FileTime {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        modified_ms: i64,
    },
    /// Nothing tells when the video was recorded.
    Nothing,
}

/// When video time 0 happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Alignment {
    /// Epoch ms (UTC) of video time 0.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub start_ms: i64,
    #[cfg_attr(feature = "ts", ts(type = "'user' | 'clock' | 'fileName' | 'container' | 'fileTime' | 'none'"))]
    pub by: AlignedBy,
    #[cfg_attr(feature = "ts", ts(type = "'low' | 'medium' | 'high'"))]
    pub confidence: Confidence,
    pub evidence: AlignEvidence,
}

impl Alignment {
    /// The moment of a frame at `video_ms`.
    pub fn at(&self, video_ms: u64) -> i64 {
        self.start_ms.saturating_add(i64::try_from(video_ms).unwrap_or(i64::MAX))
    }
}

/// Everything known about when a video was recorded.
#[derive(Debug, Clone, Default)]
pub struct AlignInput<'a> {
    pub user: UserAlignment,
    pub clock: Option<&'a ClockReading>,
    pub file_name: &'a str,
    pub creation_ms: Option<i64>,
    pub file_modified_ms: Option<i64>,
    pub duration_ms: u64,
    /// The profile's zone for times shown without one (`UTC`, `local`, `+02:00`).
    pub tz: &'a str,
}

fn ms(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// The automatic alignment (everything but the user).
fn automatic(i: &AlignInput) -> Alignment {
    // A dated screen clock, or a time of day dated by the other evidence.
    if let Some(c) = i.clock.filter(|c| !c.shown.is_empty()) {
        let ev = TimeEvidence {
            shown: c.shown.clone(),
            file_modified_ms: i.creation_ms.or(i.file_modified_ms),
            ..TimeEvidence::default()
        };
        if let Some(t) = resolve(&ev, i.tz, (None, None)).filter(|t| matches!(t.source, TimeSource::Screen | TimeSource::ScreenTimeOfDay)) {
            let dated = t.source == TimeSource::Screen;
            return Alignment {
                start_ms: t.at_ms.saturating_sub(ms(c.frame_ms)),
                by: AlignedBy::Clock,
                confidence: if dated { Confidence::High } else { Confidence::Medium },
                evidence: AlignEvidence::Clock {
                    frame_ms: c.frame_ms,
                    text: t.text,
                    has_date: dated,
                },
            };
        }
    }
    let named = TimeEvidence {
        file_name: Some(i.file_name.to_string()),
        ..TimeEvidence::default()
    };
    if let Some(t) = resolve(&named, i.tz, (None, None)).filter(|t| t.source == TimeSource::FileName) {
        return Alignment {
            start_ms: t.at_ms,
            by: AlignedBy::FileName,
            confidence: Confidence::Medium,
            evidence: AlignEvidence::FileName { text: t.text },
        };
    }
    if let Some(c) = i.creation_ms {
        return Alignment {
            start_ms: c,
            by: AlignedBy::Container,
            confidence: Confidence::Medium,
            evidence: AlignEvidence::Container { created_ms: c },
        };
    }
    if let Some(m) = i.file_modified_ms {
        return Alignment {
            start_ms: m.saturating_sub(ms(i.duration_ms)),
            by: AlignedBy::FileTime,
            confidence: Confidence::Low,
            evidence: AlignEvidence::FileTime { modified_ms: m },
        };
    }
    Alignment {
        start_ms: 0,
        by: AlignedBy::None,
        confidence: Confidence::Low,
        evidence: AlignEvidence::Nothing,
    }
}

/// Decides when the video started (see the module docs).
pub fn align(i: &AlignInput) -> Alignment {
    match i.user {
        UserAlignment::Start { start_ms } => Alignment {
            start_ms,
            by: AlignedBy::User,
            confidence: Confidence::High,
            evidence: AlignEvidence::UserStart,
        },
        UserAlignment::Offset { offset_ms } => {
            let base = automatic(i);
            Alignment {
                start_ms: base.start_ms.saturating_add(offset_ms),
                by: AlignedBy::User,
                confidence: Confidence::High,
                evidence: AlignEvidence::UserOffset {
                    offset_ms,
                    base: Box::new(base.evidence),
                },
            }
        }
        UserAlignment::Auto => automatic(i),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, NaiveDateTime};

    fn utc(s: &str) -> i64 {
        DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis()
    }

    fn local(s: &str) -> i64 {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap().and_utc().timestamp_millis()
    }

    fn input<'a>(clock: Option<&'a ClockReading>) -> AlignInput<'a> {
        AlignInput {
            user: UserAlignment::Auto,
            clock,
            file_name: "recording.mp4",
            creation_ms: Some(utc("2026-10-21T08:40:00Z")),
            file_modified_ms: Some(utc("2026-10-21T08:45:00Z")),
            duration_ms: 90_000,
            tz: "UTC",
        }
    }

    fn reading(text: &str, has_date: bool) -> ClockReading {
        ClockReading {
            frame_ms: 12_000,
            shown: vec![ShownTime {
                text: text.into(),
                has_date,
            }],
        }
    }

    #[test]
    fn a_dated_clock_on_screen_wins_over_metadata() {
        let c = reading("2026-10-21 10:42:07", true);
        let a = align(&input(Some(&c)));
        assert_eq!(a.start_ms, local("2026-10-21 10:41:55"));
        assert_eq!((a.by, a.confidence), (AlignedBy::Clock, Confidence::High));
        assert_eq!(
            a.evidence,
            AlignEvidence::Clock {
                frame_ms: 12_000,
                text: "2026-10-21 10:42:07".into(),
                has_date: true
            }
        );
        assert_eq!(a.at(2_000), local("2026-10-21 10:41:57"));
    }

    #[test]
    fn a_time_of_day_takes_the_date_of_the_container_in_the_profile_zone() {
        let c = reading("10:42:07", false);
        let mut i = input(Some(&c));
        i.tz = "+02:00";
        let a = align(&i);
        assert_eq!(a.start_ms, utc("2026-10-21T10:41:55+02:00"));
        assert_eq!((a.by, a.confidence), (AlignedBy::Clock, Confidence::Medium));
    }

    #[test]
    fn without_a_readable_clock_the_file_name_then_the_container_then_the_file_time() {
        let unreadable = reading("soon", false);
        let mut i = input(Some(&unreadable));
        i.file_name = "Screen Recording 2026-10-21 at 09.14.03.mov";
        let a = align(&i);
        assert_eq!((a.by, a.start_ms, a.confidence), (AlignedBy::FileName, local("2026-10-21 09:14:03"), Confidence::Medium));
        i.file_name = "recording.mp4";
        let a = align(&i);
        assert_eq!((a.by, a.start_ms), (AlignedBy::Container, utc("2026-10-21T08:40:00Z")));
        i.creation_ms = None;
        let a = align(&i);
        assert_eq!((a.by, a.confidence), (AlignedBy::FileTime, Confidence::Low));
        assert_eq!(a.start_ms, utc("2026-10-21T08:43:30Z"), "the end minus the duration");
        i.file_modified_ms = None;
        let a = align(&i);
        assert_eq!((a.by, a.start_ms, a.evidence), (AlignedBy::None, 0, AlignEvidence::Nothing));
    }

    #[test]
    fn the_user_sets_the_start_or_shifts_the_automatic_one() {
        let mut i = input(None);
        i.user = UserAlignment::Start { start_ms: 42 };
        let a = align(&i);
        assert_eq!((a.start_ms, a.by, a.confidence, a.evidence), (42, AlignedBy::User, Confidence::High, AlignEvidence::UserStart));
        i.user = UserAlignment::Offset { offset_ms: -3_000 };
        let a = align(&i);
        assert_eq!(a.start_ms, utc("2026-10-21T08:39:57Z"));
        assert_eq!(
            a.evidence,
            AlignEvidence::UserOffset {
                offset_ms: -3_000,
                base: Box::new(AlignEvidence::Container {
                    created_ms: utc("2026-10-21T08:40:00Z")
                })
            }
        );
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(json["by"], "user");
        assert_eq!(json["evidence"]["kind"], "userOffset");
        assert_eq!(serde_json::to_value(UserAlignment::Offset { offset_ms: 5 }).unwrap(), serde_json::json!({"kind": "offset", "offsetMs": 5}));
    }
}
