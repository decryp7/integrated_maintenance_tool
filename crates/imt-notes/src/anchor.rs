//! What a note is attached to. Every anchor is checked ([`Anchor::check`]) before a note keeps it: times in order,
//! labels and paths bounded, image regions inside their image.

use crate::error::{invalid, Result};
use crate::{is_image_key, label, MAX_IMAGE_SIDE, MAX_LABEL_CHARS, MAX_PATH_CHARS, MAX_REGIONS};
use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// A rectangle in an image's own pixels (of the original file, not of a scaled copy).
#[cfg_attr(feature = "ts", derive(TS), ts(rename = "NoteRegion"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Region {
    /// Whether the region has an area and lies inside a `width` × `height` image.
    pub fn fits(&self, width: u32, height: u32) -> bool {
        self.w > 0
            && self.h > 0
            && self.x.checked_add(self.w).is_some_and(|r| r <= width)
            && self.y.checked_add(self.h).is_some_and(|b| b <= height)
    }
}

/// The image a note marks: its content key ([`crate::image_key`]), file name, original size and the file it was
/// opened from (none when it opens only from the project's copy).
#[cfg_attr(feature = "ts", derive(TS), ts(rename = "NoteImageRef"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageRef {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    pub width: u32,
    pub height: u32,
}

impl ImageRef {
    fn check(&self) -> Result<ImageRef> {
        if !is_image_key(&self.key) {
            return Err(invalid("unknown image"));
        }
        if self.width == 0 || self.height == 0 || self.width > MAX_IMAGE_SIDE || self.height > MAX_IMAGE_SIDE {
            return Err(invalid("the image's size is not valid"));
        }
        Ok(ImageRef {
            key: self.key.clone(),
            name: non_empty(label(&self.name, MAX_LABEL_CHARS), "image"),
            path: self.path.as_deref().map(check_path).transpose()?,
            width: self.width,
            height: self.height,
        })
    }
}

/// What a note is about (`kind` on the wire).
#[cfg_attr(feature = "ts", derive(TS), ts(rename = "NoteAnchor"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Anchor {
    /// A log event: the store's id, with its time, source and an excerpt the engine captured when the note was made
    /// (shown when the event no longer resolves).
    #[serde(rename_all = "camelCase")]
    Event {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        event_id: u64,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(type = "number | null"))]
        ts: Option<i64>,
        #[serde(default)]
        source: String,
        #[serde(default)]
        excerpt: String,
    },
    /// A moment of the timeline (epoch ms).
    Instant {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        at: i64,
    },
    /// A range of the timeline (epoch ms, `from` before `to`).
    Range {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        from: i64,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        to: i64,
    },
    /// A source (file) of the workspace, by its path as the workspace lists it.
    Source { path: String },
    /// A marker (`to` none) or a span (`to` its end) by name.
    Marker {
        name: String,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        from: i64,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(type = "number | null"))]
        to: Option<i64>,
    },
    /// A point of a chart: its title, series, time and value.
    #[serde(rename_all = "camelCase")]
    ChartPoint {
        chart: String,
        #[serde(default)]
        series: Option<String>,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        at: i64,
        #[serde(default)]
        value: Option<f64>,
    },
    /// The project (the investigation) as a whole.
    Project,
    /// Regions of an image (one to [`MAX_REGIONS`]).
    Image { image: ImageRef, regions: Vec<Region> },
}

fn non_empty(s: String, fallback: &str) -> String {
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

fn check_path(p: &str) -> Result<String> {
    if p.trim().is_empty() || p.chars().count() > MAX_PATH_CHARS || p.chars().any(char::is_control) {
        return Err(invalid("the path is not valid"));
    }
    Ok(p.to_string())
}

/// Checks regions of a `width` × `height` image.
pub(crate) fn check_regions(regions: &[Region], width: u32, height: u32) -> Result<Vec<Region>> {
    if regions.is_empty() {
        return Err(invalid("mark at least one area of the image"));
    }
    if regions.len() > MAX_REGIONS {
        return Err(invalid(format!("mark at most {MAX_REGIONS} areas on one note")));
    }
    if let Some(bad) = regions.iter().find(|r| !r.fits(width, height)) {
        return Err(invalid(format!(
            "the area {}×{} at {},{} is not inside the image ({width}×{height})",
            bad.w, bad.h, bad.x, bad.y
        )));
    }
    Ok(regions.to_vec())
}

impl Anchor {
    /// The anchor as kept: checked, with labels cleaned and bounded.
    pub fn check(&self) -> Result<Anchor> {
        Ok(match self {
            Anchor::Event {
                event_id,
                ts,
                source,
                excerpt,
            } => Anchor::Event {
                event_id: *event_id,
                ts: *ts,
                source: label(source, MAX_LABEL_CHARS),
                excerpt: label(excerpt, MAX_LABEL_CHARS),
            },
            Anchor::Instant { at } => Anchor::Instant { at: *at },
            Anchor::Range { from, to } => {
                if from >= to {
                    return Err(invalid("the range must end after it starts"));
                }
                Anchor::Range { from: *from, to: *to }
            }
            Anchor::Source { path } => Anchor::Source { path: check_path(path)? },
            Anchor::Marker { name, from, to } => {
                if to.is_some_and(|t| t < *from) {
                    return Err(invalid("the span must end after it starts"));
                }
                Anchor::Marker {
                    name: non_empty(label(name, MAX_LABEL_CHARS), "marker"),
                    from: *from,
                    to: *to,
                }
            }
            Anchor::ChartPoint { chart, series, at, value } => {
                if value.is_some_and(|v| !v.is_finite()) {
                    return Err(invalid("the chart value is not a number"));
                }
                Anchor::ChartPoint {
                    chart: non_empty(label(chart, MAX_LABEL_CHARS), "chart"),
                    series: series.as_deref().map(|s| label(s, MAX_LABEL_CHARS)).filter(|s| !s.is_empty()),
                    at: *at,
                    value: *value,
                }
            }
            Anchor::Project => Anchor::Project,
            Anchor::Image { image, regions } => {
                let image = image.check()?;
                let regions = check_regions(regions, image.width, image.height)?;
                Anchor::Image { image, regions }
            }
        })
    }

    /// The anchor's kind as on the wire.
    pub fn kind(&self) -> &'static str {
        match self {
            Anchor::Event { .. } => "event",
            Anchor::Instant { .. } => "instant",
            Anchor::Range { .. } => "range",
            Anchor::Source { .. } => "source",
            Anchor::Marker { .. } => "marker",
            Anchor::ChartPoint { .. } => "chartPoint",
            Anchor::Project => "project",
            Anchor::Image { .. } => "image",
        }
    }

    /// The time the anchor covers (start, end; equal for a moment), if it has one.
    pub fn time_span(&self) -> Option<(i64, i64)> {
        match self {
            Anchor::Event { ts, .. } => ts.map(|t| (t, t)),
            Anchor::Instant { at } | Anchor::ChartPoint { at, .. } => Some((*at, *at)),
            Anchor::Range { from, to } => Some((*from, *to)),
            Anchor::Marker { from, to, .. } => Some((*from, to.unwrap_or(*from))),
            Anchor::Source { .. } | Anchor::Project | Anchor::Image { .. } => None,
        }
    }

    /// The event the anchor names, if any.
    pub fn event_id(&self) -> Option<u64> {
        match self {
            Anchor::Event { event_id, .. } => Some(*event_id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key() -> String {
        crate::image_key(b"png")
    }

    fn image(regions: Vec<Region>) -> Anchor {
        Anchor::Image {
            image: ImageRef {
                key: key(),
                name: "screen.png".into(),
                path: None,
                width: 100,
                height: 50,
            },
            regions,
        }
    }

    #[test]
    fn anchors_round_trip_with_kind_tags() {
        let a: Anchor = serde_json::from_value(json!({"kind": "event", "eventId": 7})).unwrap();
        assert_eq!(a.event_id(), Some(7));
        assert_eq!(serde_json::to_value(&a).unwrap()["eventId"], 7);
        let c: Anchor = serde_json::from_value(json!({"kind": "chartPoint", "chart": "CPU", "at": 5, "value": 1.5})).unwrap();
        assert_eq!(c.kind(), "chartPoint");
        assert_eq!(
            serde_json::from_value::<Anchor>(json!({"kind": "project"})).unwrap(),
            Anchor::Project
        );
        assert!(serde_json::from_value::<Anchor>(json!({"kind": "nope"})).is_err());
    }

    #[test]
    fn times_must_be_in_order() {
        assert!(Anchor::Range { from: 5, to: 5 }.check().is_err());
        assert_eq!(Anchor::Range { from: 1, to: 5 }.check().unwrap().time_span(), Some((1, 5)));
        assert!(Anchor::Marker {
            name: "m".into(),
            from: 5,
            to: Some(4)
        }
        .check()
        .is_err());
        let m = Anchor::Marker {
            name: "".into(),
            from: 5,
            to: None,
        }
        .check()
        .unwrap();
        assert_eq!(m.time_span(), Some((5, 5)));
        assert!(matches!(m, Anchor::Marker { ref name, .. } if name == "marker"));
        assert_eq!(Anchor::Instant { at: 3 }.check().unwrap().time_span(), Some((3, 3)));
        assert_eq!(Anchor::Project.check().unwrap().time_span(), None);
    }

    #[test]
    fn labels_and_paths_are_bounded() {
        let e = Anchor::Event {
            event_id: 1,
            ts: Some(9),
            source: "a\u{7}.log".into(),
            excerpt: "x".repeat(500),
        }
        .check()
        .unwrap();
        let Anchor::Event { source, excerpt, .. } = &e else { panic!() };
        assert_eq!(source, "a .log");
        assert_eq!(excerpt.chars().count(), MAX_LABEL_CHARS + 1);
        assert_eq!(e.time_span(), Some((9, 9)));
        assert!(Anchor::Source { path: " ".into() }.check().is_err());
        assert!(Anchor::Source { path: "a\nb".into() }.check().is_err());
        assert!(Anchor::Source {
            path: "x".repeat(MAX_PATH_CHARS + 1)
        }
        .check()
        .is_err());
        assert_eq!(
            Anchor::Source {
                path: "/logs/a.log".into()
            }
            .check()
            .unwrap()
            .kind(),
            "source"
        );
    }

    #[test]
    fn chart_values_must_be_numbers() {
        let p = |value| Anchor::ChartPoint {
            chart: " ".into(),
            series: Some(" ".into()),
            at: 1,
            value,
        };
        assert!(p(Some(f64::NAN)).check().is_err());
        let ok = p(Some(2.0)).check().unwrap();
        assert!(matches!(ok, Anchor::ChartPoint { ref chart, series: None, .. } if chart == "chart"));
        assert_eq!(ok.time_span(), Some((1, 1)));
    }

    #[test]
    fn image_regions_lie_inside_the_image() {
        let r = |x, y, w, h| Region { x, y, w, h };
        assert!(image(vec![r(0, 0, 100, 50)]).check().is_ok());
        assert!(image(vec![]).check().unwrap_err().to_string().contains("at least one"));
        assert!(image(vec![r(1, 0, 100, 50)])
            .check()
            .unwrap_err()
            .to_string()
            .contains("not inside"));
        assert!(image(vec![r(0, 0, 0, 5)]).check().is_err());
        assert!(image(vec![r(u32::MAX, 0, 2, 2)]).check().is_err(), "no overflow");
        assert!(image(vec![r(0, 0, 1, 1); MAX_REGIONS + 1]).check().is_err());
        let a = image(vec![r(10, 10, 5, 5)]);
        assert_eq!(a.kind(), "image");
        assert_eq!(a.time_span(), None);
        assert_eq!(a.event_id(), None);
    }

    #[test]
    fn image_refs_are_checked() {
        let mk = |key: String, width, height, path: Option<&str>| Anchor::Image {
            image: ImageRef {
                key,
                name: "".into(),
                path: path.map(String::from),
                width,
                height,
            },
            regions: vec![Region { x: 0, y: 0, w: 1, h: 1 }],
        };
        assert!(mk("../x".into(), 10, 10, None).check().is_err());
        assert!(mk(key(), 0, 10, None).check().is_err());
        assert!(mk(key(), 10, MAX_IMAGE_SIDE + 1, None).check().is_err());
        assert!(mk(key(), 10, 10, Some("")).check().is_err());
        let ok = mk(key(), 10, 10, Some("/a.png")).check().unwrap();
        assert!(matches!(ok, Anchor::Image { ref image, .. } if image.name == "image"));
    }
}
