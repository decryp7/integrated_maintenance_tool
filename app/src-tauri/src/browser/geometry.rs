//! Where the page webview goes (ADR 0024): the Browser panel reports its page area in CSS pixels of the IMT webview
//! with `window.devicePixelRatio` (which includes the OS scale and the UI zoom), and whether the page may show
//! (its tab is active, no IMT overlay covers it, the window is not minimised). Rust turns that into the child
//! webview's bounds in physical pixels, or hides it. Pure: tested without a window.

use serde::{Deserialize, Serialize};

/// The smallest and largest device pixel ratio accepted (anything else is a broken report).
pub const MIN_SCALE: f64 = 0.25;
pub const MAX_SCALE: f64 = 8.0;
/// The largest coordinate or size accepted, in CSS pixels (far beyond any screen).
pub const MAX_CSS: f64 = 100_000.0;

/// A rectangle in CSS pixels of the IMT webview (`getBoundingClientRect`).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CssRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What the Browser panel reports (`browser_place`).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    /// The page may show now: the panel's tab is active, nothing of IMT covers it and the window is not minimised.
    pub visible: bool,
    /// The page area; `None` while the panel is not laid out (closed, or hidden by the dock).
    pub rect: Option<CssRect>,
    /// `window.devicePixelRatio` of the reporting webview.
    pub scale: f64,
}

/// A rectangle in physical pixels of the window's client area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Where the page webview is: hidden, or shown at a place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Hidden,
    At(PhysicalRect),
}

impl Placement {
    /// Parses a report from the webview: every number finite, the scale within [`MIN_SCALE`]..=[`MAX_SCALE`] and the
    /// rectangle within ±[`MAX_CSS`]; why not otherwise.
    pub fn validated(self) -> Result<Placement, String> {
        if !self.scale.is_finite() || !(MIN_SCALE..=MAX_SCALE).contains(&self.scale) {
            return Err(format!("the scale {} is out of range", self.scale));
        }
        if let Some(r) = self.rect {
            let ok = |v: f64| v.is_finite() && v.abs() <= MAX_CSS;
            if !(ok(r.x) && ok(r.y) && ok(r.width) && ok(r.height)) || r.width < 0.0 || r.height < 0.0 {
                return Err("the page area is not a valid rectangle".into());
            }
        }
        Ok(self)
    }

    /// The page webview's view for this report: shown only while visible and laid out with an area of at least one
    /// physical pixel each way.
    pub fn view(&self) -> View {
        match (self.visible, self.rect.and_then(|r| to_physical(r, self.scale))) {
            (true, Some(r)) => View::At(r),
            _ => View::Hidden,
        }
    }
}

/// A CSS rectangle in physical pixels at `scale`: the edges are rounded (not the size), so neighbouring areas meet
/// without a gap; the part left of or above the client area is cut off. `None` when nothing of it is left.
pub fn to_physical(r: CssRect, scale: f64) -> Option<PhysicalRect> {
    let (left, top) = ((r.x * scale).round().max(0.0), (r.y * scale).round().max(0.0));
    let (right, bottom) = (((r.x + r.width) * scale).round(), ((r.y + r.height) * scale).round());
    let (width, height) = (right - left, bottom - top);
    if !(width >= 1.0 && height >= 1.0) {
        return None;
    }
    Some(PhysicalRect {
        x: left as i32,
        y: top as i32,
        width: width as u32,
        height: height as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> CssRect {
        CssRect { x, y, width, height }
    }

    fn placed(visible: bool, r: Option<CssRect>, scale: f64) -> Placement {
        Placement { visible, rect: r, scale }
    }

    #[test]
    fn css_pixels_scale_to_physical_pixels_with_rounded_edges() {
        assert_eq!(
            to_physical(rect(100.0, 50.0, 800.0, 600.0), 1.0),
            Some(PhysicalRect {
                x: 100,
                y: 50,
                width: 800,
                height: 600
            })
        );
        assert_eq!(
            to_physical(rect(100.0, 50.0, 800.0, 600.0), 1.5),
            Some(PhysicalRect {
                x: 150,
                y: 75,
                width: 1200,
                height: 900
            })
        );
        // Fractional CSS pixels (a 125% scale and a zoomed UI): the edges round, so two panels side by side meet.
        let a = to_physical(rect(0.0, 0.0, 333.3, 10.0), 1.25).unwrap();
        let b = to_physical(rect(333.3, 0.0, 333.3, 10.0), 1.25).unwrap();
        assert_eq!(a.x + a.width as i32, b.x, "no gap and no overlap");
        assert_eq!((a.width, b.width), (417, 416));
    }

    #[test]
    fn what_is_off_the_client_area_or_too_small_does_not_show() {
        assert_eq!(
            to_physical(rect(-10.0, -5.0, 110.0, 105.0), 1.0),
            Some(PhysicalRect {
                x: 0,
                y: 0,
                width: 100,
                height: 100
            }),
            "cut at the top left"
        );
        assert_eq!(to_physical(rect(-200.0, 0.0, 100.0, 100.0), 1.0), None, "entirely to the left");
        assert_eq!(to_physical(rect(0.0, 0.0, 0.0, 100.0), 2.0), None, "no width (display: none)");
        assert_eq!(to_physical(rect(0.0, 0.0, 0.2, 100.0), 1.0), None, "less than a pixel");
    }

    #[test]
    fn the_page_shows_only_while_visible_and_laid_out() {
        let r = Some(rect(10.0, 20.0, 300.0, 200.0));
        assert_eq!(
            placed(true, r, 2.0).view(),
            View::At(PhysicalRect {
                x: 20,
                y: 40,
                width: 600,
                height: 400
            })
        );
        assert_eq!(
            placed(false, r, 2.0).view(),
            View::Hidden,
            "a background tab, an overlay or a minimised window"
        );
        assert_eq!(placed(true, None, 1.0).view(), View::Hidden, "the panel is closed");
        assert_eq!(
            placed(true, Some(rect(0.0, 0.0, 0.0, 0.0)), 1.0).view(),
            View::Hidden,
            "another panel is maximised"
        );
    }

    #[test]
    fn broken_reports_are_refused() {
        assert!(placed(true, None, 1.0).validated().is_ok());
        for s in [0.0, -1.0, f64::NAN, f64::INFINITY, 9.0, 0.1] {
            assert!(placed(true, None, s).validated().is_err(), "scale {s}");
        }
        for r in [
            rect(f64::NAN, 0.0, 1.0, 1.0),
            rect(0.0, 0.0, f64::INFINITY, 1.0),
            rect(0.0, 0.0, -1.0, 1.0),
            rect(0.0, MAX_CSS * 2.0, 1.0, 1.0),
        ] {
            assert!(placed(true, Some(r), 1.0).validated().is_err(), "{r:?}");
        }
        assert!(placed(false, Some(rect(-5.0, -5.0, 10.0, 10.0)), 1.0).validated().is_ok());
    }

    #[test]
    fn reports_come_as_camel_case_json() {
        let p: Placement = serde_json::from_value(serde_json::json!({
            "visible": true, "rect": {"x": 1, "y": 2, "width": 3, "height": 4}, "scale": 1.25
        }))
        .unwrap();
        assert_eq!(p, placed(true, Some(rect(1.0, 2.0, 3.0, 4.0)), 1.25));
        let hidden: Placement = serde_json::from_value(serde_json::json!({"visible": false, "rect": null, "scale": 1})).unwrap();
        assert_eq!(hidden.view(), View::Hidden);
        assert!(serde_json::from_value::<Placement>(serde_json::json!({"visible": true, "scale": 1, "extra": 1})).is_ok());
    }
}
