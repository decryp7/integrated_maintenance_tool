//! Where a dialog window opens (ADR 0054, placement policy): always on the monitor of the window that opened it.
//!
//! Pure geometry in physical pixels, so it holds across monitors with different scale factors (logical coordinates of
//! two monitors with different DPI do not line up, and a secondary display left of or above the primary one has
//! negative coordinates). The shell owns the decision; the UI only remembers what the shell reported when the dialog
//! closed: its size, and its place as an offset from its parent window (never an absolute screen position, which
//! would pull the dialog onto whatever display the parent was on last time).

use crate::dialog_contracts::Kind;
use serde::{Deserialize, Serialize};

/// The height (logical px) a native title bar adds to a dialog's inner size; the whole window must fit the work area.
pub const TITLE_BAR: f64 = 32.0;
/// The share of the work area a dialog opens at most when it has no remembered size.
pub const SCREEN_SHARE: f64 = 0.75;
/// Remembered offsets beyond this (logical px) are nonsense and ignored.
const MAX_OFFSET: f64 = 100_000.0;

/// A rectangle in physical screen pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn right(&self) -> f64 {
        self.x + self.width
    }

    fn bottom(&self) -> f64 {
        self.y + self.height
    }

    fn contains(&self, r: &Rect) -> bool {
        r.x >= self.x && r.y >= self.y && r.right() <= self.right() && r.bottom() <= self.bottom()
    }

    pub(crate) fn is_usable(&self) -> bool {
        [self.x, self.y, self.width, self.height].iter().all(|v| v.is_finite()) && self.width > 0.0 && self.height > 0.0
    }

    fn overlap(&self, r: &Rect) -> f64 {
        let w = (self.right().min(r.right()) - self.x.max(r.x)).max(0.0);
        let h = (self.bottom().min(r.bottom()) - self.y.max(r.y)).max(0.0);
        w * h
    }
}

/// A monitor's usable area (without menu bar, dock or task bar), physical px, and its scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub work: Rect,
    pub scale: f64,
}

impl Monitor {
    /// The scale factor, or 1 when the platform reported nonsense.
    pub fn scale(&self) -> f64 {
        if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        }
    }
}

/// What the UI remembers of a dialog (logical px): its inner size and the offset of its top-left corner from its
/// parent window's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Remembered {
    pub dx: f64,
    pub dy: f64,
    pub width: f64,
    pub height: f64,
}

impl Remembered {
    /// `None` when a value is not a finite number or the size is not positive.
    pub fn sanitized(self) -> Option<Remembered> {
        let finite = [self.dx, self.dy, self.width, self.height].iter().all(|v| v.is_finite());
        (finite && self.width > 0.0 && self.height > 0.0).then(|| Remembered {
            dx: self.dx.clamp(-MAX_OFFSET, MAX_OFFSET),
            dy: self.dy.clamp(-MAX_OFFSET, MAX_OFFSET),
            ..self
        })
    }
}

/// Where a dialog goes: its outer top-left corner and inner size in physical px, and the scale of the monitor it is on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    /// The remembered offset was used (it kept the dialog on the parent's monitor).
    pub remembered: bool,
}

impl Placement {
    /// The inner size in logical px of its monitor.
    pub fn logical_size(&self) -> (f64, f64) {
        (self.width / self.scale, self.height / self.scale)
    }
}

/// The index of the monitor a parent window is on: the one it overlaps most; when it overlaps none, the nearest one.
/// Never "the primary display" unless that is where the parent is.
pub fn monitor_of(parent: &Rect, monitors: &[Monitor]) -> Option<usize> {
    let usable = || monitors.iter().enumerate().filter(|(_, m)| m.work.is_usable());
    let (best, m) = usable().max_by(|a, b| a.1.work.overlap(parent).total_cmp(&b.1.work.overlap(parent)))?;
    if m.work.overlap(parent) > 0.0 {
        return Some(best);
    }
    let (cx, cy) = (parent.x + parent.width / 2.0, parent.y + parent.height / 2.0);
    let distance = |m: &Monitor| {
        let dx = (m.work.x - cx).max(cx - m.work.right()).max(0.0);
        let dy = (m.work.y - cy).max(cy - m.work.bottom()).max(0.0);
        dx * dx + dy * dy
    };
    usable().min_by(|a, b| distance(a.1).total_cmp(&distance(b.1))).map(|(i, _)| i)
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi.max(lo))
}

/// Moves `r` (an outer rectangle) inside `work`; it is no larger than `work` by the time this is called.
fn inside(mut r: Rect, work: &Rect) -> Rect {
    r.x = clamp(r.x, work.x, work.right() - r.width);
    r.y = clamp(r.y, work.y, work.bottom() - r.height);
    r
}

/// Where dialog of inner size `size` (logical px, already within its minimum) opens for a parent window at `parent`
/// (outer rectangle, physical px) on `monitor`:
///
/// * the size is the remembered one when there is one (never below `min`), else `size` limited to
///   [`SCREEN_SHARE`] of the work area; either way no larger than the work area;
/// * the remembered offset from the parent is used only when the whole dialog then lies within the parent monitor's
///   work area; otherwise the dialog is centred on the parent (a third from its top) and moved into the work area.
///
/// With no parent rectangle it is centred in the work area; with no monitor it is centred on the parent unclamped.
/// Only when the platform reports neither does it fall back to (100, 100) logical.
pub fn place(
    parent: Option<Rect>,
    monitor: Option<Monitor>,
    remembered: Option<Remembered>,
    size: (f64, f64),
    min: (f64, f64),
) -> Placement {
    let parent = parent.filter(Rect::is_usable);
    let monitor = monitor.filter(|m| m.work.is_usable());
    let s = monitor.map_or(1.0, |m| m.scale());
    let remembered = remembered.and_then(Remembered::sanitized);
    let (mut w, mut h) = match (remembered, monitor) {
        (Some(r), _) => (r.width.max(min.0), r.height.max(min.1)),
        (None, Some(m)) => (
            size.0.min(m.work.width / s * SCREEN_SHARE).max(min.0),
            size.1.min(m.work.height / s * SCREEN_SHARE).max(min.1),
        ),
        (None, None) => size,
    };
    if let Some(m) = monitor {
        // A dialog bigger than the work area is shrunk to it (title bar included).
        w = w.min(m.work.width / s);
        h = h.min(m.work.height / s - TITLE_BAR).max(1.0);
    }
    let outer = |x: f64, y: f64| Rect {
        x,
        y,
        width: w * s,
        height: (h + TITLE_BAR) * s,
    };
    let done = |r: Rect, used: bool| Placement {
        x: r.x,
        y: r.y,
        width: w * s,
        height: h * s,
        scale: s,
        remembered: used,
    };
    if let (Some(p), Some(r)) = (parent, remembered) {
        let at = outer(p.x + r.dx * s, p.y + r.dy * s);
        if monitor.is_none_or(|m| m.work.contains(&at)) {
            return done(at, true);
        }
    }
    let centred = match (parent, monitor) {
        (Some(p), _) => {
            let o = outer(0.0, 0.0);
            outer(p.x + (p.width - o.width) / 2.0, (p.y + (p.height - o.height) / 3.0).max(p.y))
        }
        (None, Some(m)) => {
            let o = outer(0.0, 0.0);
            outer(
                m.work.x + (m.work.width - o.width) / 2.0,
                m.work.y + (m.work.height - o.height) / 3.0,
            )
        }
        (None, None) => outer(100.0, 100.0),
    };
    done(monitor.map_or(centred, |m| inside(centred, &m.work)), false)
}

/// After a re-fit to `inner_height` (logical px): the window's new outer top-left (physical px) so that it stays inside
/// its monitor's work area. `None` when it already does (or nothing is known).
pub fn keep_on_monitor(window: Rect, inner_height: f64, monitor: Option<Monitor>) -> Option<(f64, f64)> {
    let m = monitor.filter(|m| m.work.is_usable())?;
    let s = m.scale();
    let r = Rect {
        height: ((inner_height + TITLE_BAR) * s).min(m.work.height),
        width: window.width.min(m.work.width),
        ..window
    };
    let moved = inside(r, &m.work);
    (moved.x != window.x || moved.y != window.y).then_some((moved.x, moved.y))
}

/// The offset to remember for a dialog at `dialog` (outer top-left and inner size, logical px of its own monitor)
/// whose parent's outer top-left is `parent` (logical px of the parent's monitor).
pub fn remembered_of(dialog: crate::panel_windows::Bounds, parent: (f64, f64)) -> Option<Remembered> {
    Remembered {
        dx: dialog.x - parent.0,
        dy: dialog.y - parent.1,
        width: dialog.width,
        height: dialog.height,
    }
    .sanitized()
}

/// What of a remembered place a dialog of `kind` keeps (the kind's size rule): a decision is exactly as big as its
/// question and centred every time, so it keeps nothing; a fitted form (no declared height) keeps its width and offset
/// but its height comes from the content (`default_height` until the first fit); everything else keeps it all.
pub fn memory_for(kind: Kind, fitted: bool, remembered: Option<Remembered>, default_height: f64) -> Option<Remembered> {
    match kind {
        Kind::Decision => None,
        Kind::Form if fitted => remembered.map(|r| Remembered {
            height: default_height,
            ..r
        }),
        _ => remembered,
    }
}

/// How the shell hands a placement to the window system. macOS positions windows in points (one global logical
/// space: a monitor's physical origin divided by its own scale), so the logical position is exact there; on Windows
/// and Linux the logical space is not global across mixed-DPI monitors, so the physical position is set as well.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenPosition {
    /// For the builder: logical px (exact on macOS; on the parent's monitor elsewhere when the scales agree).
    pub logical: (f64, f64),
    /// Set after building where the logical position is not exact (`None` on macOS).
    pub physical: Option<(i32, i32)>,
}

pub fn screen_position(p: &Placement, os: &str) -> ScreenPosition {
    let logical = (p.x / p.scale, p.y / p.scale);
    // Within i32: a physical coordinate comes from a monitor rectangle (i32) plus a clamped offset.
    let physical = (os != "macos").then(|| (p.x.round() as i32, p.y.round() as i32));
    ScreenPosition { logical, physical }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect { x, y, width, height }
    }

    fn monitor(x: f64, y: f64, width: f64, height: f64, scale: f64) -> Monitor {
        Monitor {
            work: rect(x, y, width, height),
            scale,
        }
    }

    fn rem(dx: f64, dy: f64, width: f64, height: f64) -> Option<Remembered> {
        Some(Remembered { dx, dy, width, height })
    }

    /// The whole outer window of `p` lies inside `m`'s work area.
    fn on(p: &Placement, m: &Monitor) -> bool {
        m.work.contains(&rect(p.x, p.y, p.width, p.height + TITLE_BAR * p.scale))
    }

    // Primary 2560x1440 @1 at the origin (menu bar 25), a Retina 3024x1964 @2 LEFT of it (negative x), and a
    // 1920x1080 @1 ABOVE the primary (negative y).
    fn primary() -> Monitor {
        monitor(0.0, 25.0, 2560.0, 1415.0, 1.0)
    }
    fn left_retina() -> Monitor {
        monitor(-3024.0, 0.0, 3024.0, 1964.0, 2.0)
    }
    fn above() -> Monitor {
        monitor(0.0, -1080.0, 1920.0, 1080.0, 1.0)
    }

    #[test]
    fn a_new_dialog_is_centred_on_its_parent_on_the_parents_monitor() {
        let m = primary();
        let p = place(
            Some(rect(100.0, 100.0, 1000.0, 700.0)),
            Some(m),
            None,
            (600.0, 400.0),
            (320.0, 200.0),
        );
        assert_eq!((p.x, p.y, p.width, p.height), (300.0, 100.0 + (700.0 - 432.0) / 3.0, 600.0, 400.0));
        assert!(on(&p, &m) && !p.remembered);
    }

    #[test]
    fn a_parent_on_a_secondary_display_left_of_the_primary_gets_its_dialog_there_at_that_displays_scale() {
        let m = left_retina();
        let parent = rect(-2800.0, 200.0, 2400.0, 1600.0);
        let p = place(Some(parent), Some(m), None, (600.0, 400.0), (320.0, 200.0));
        assert_eq!((p.width, p.height, p.scale), (1200.0, 800.0, 2.0));
        assert!(on(&p, &m), "{p:?}");
        assert!(p.x < 0.0);
        // Logical px of that display for the builder.
        assert_eq!(screen_position(&p, "macos").logical, (p.x / 2.0, p.y / 2.0));
        assert_eq!(screen_position(&p, "macos").physical, None);
        assert_eq!(
            screen_position(&p, "windows").physical,
            Some((p.x.round() as i32, p.y.round() as i32))
        );
    }

    #[test]
    fn a_parent_on_a_display_above_the_primary_keeps_its_dialog_above() {
        let m = above();
        let p = place(
            Some(rect(100.0, -1000.0, 1200.0, 900.0)),
            Some(m),
            None,
            (600.0, 400.0),
            (320.0, 200.0),
        );
        assert!(on(&p, &m) && p.y < 0.0, "{p:?}");
    }

    #[test]
    fn a_remembered_offset_is_used_only_when_the_whole_dialog_stays_on_the_parents_monitor() {
        let m = primary();
        let parent = rect(200.0, 200.0, 1200.0, 800.0);
        let p = place(Some(parent), Some(m), rem(50.0, 60.0, 700.0, 500.0), (600.0, 400.0), (320.0, 200.0));
        assert_eq!((p.x, p.y, p.width, p.height, p.remembered), (250.0, 260.0, 700.0, 500.0, true));
        // The parent moved to the right edge since: the remembered offset would hang off the display, so it is centred.
        let parent = rect(1800.0, 200.0, 700.0, 800.0);
        let p = place(
            Some(parent),
            Some(m),
            rem(600.0, 60.0, 700.0, 500.0),
            (600.0, 400.0),
            (320.0, 200.0),
        );
        assert!(!p.remembered && on(&p, &m), "{p:?}");
    }

    #[test]
    fn a_parent_moved_to_another_display_since_the_size_was_remembered_takes_the_dialog_with_it() {
        // Remembered on the primary at offset (40, 40), 800x600 logical; the parent is now on the Retina display.
        let m = left_retina();
        let parent = rect(-2000.0, 400.0, 1800.0, 1200.0);
        let p = place(Some(parent), Some(m), rem(40.0, 40.0, 800.0, 600.0), (600.0, 400.0), (320.0, 200.0));
        // The offset is in logical px: 40 logical are 80 physical at 2x.
        assert_eq!((p.x, p.y, p.width, p.height, p.remembered), (-1920.0, 480.0, 1600.0, 1200.0, true));
        assert!(on(&p, &m));
    }

    #[test]
    fn an_offset_that_would_cross_into_the_neighbouring_display_is_not_used() {
        // The parent is at the primary's left edge; the remembered offset puts the dialog over the border to the left.
        let m = primary();
        let p = place(
            Some(rect(0.0, 100.0, 1200.0, 800.0)),
            Some(m),
            rem(-500.0, 0.0, 700.0, 500.0),
            (600.0, 400.0),
            (320.0, 200.0),
        );
        assert!(!p.remembered && on(&p, &m) && p.x >= 0.0, "{p:?}");
    }

    #[test]
    fn a_dialog_bigger_than_the_work_area_is_shrunk_to_it_and_placed_inside() {
        let m = above();
        let p = place(
            Some(rect(0.0, -1080.0, 1920.0, 1080.0)),
            Some(m),
            rem(0.0, 0.0, 4000.0, 3000.0),
            (600.0, 400.0),
            (680.0, 560.0),
        );
        assert_eq!((p.width, p.height), (1920.0, 1080.0 - TITLE_BAR));
        assert!(on(&p, &m), "{p:?}");
        // A default size is bounded by three quarters of the work area.
        let p = place(
            Some(rect(0.0, -1080.0, 1920.0, 1080.0)),
            Some(m),
            None,
            (5000.0, 5000.0),
            (320.0, 200.0),
        );
        assert_eq!((p.width, p.height), (1440.0, 810.0));
    }

    #[test]
    fn without_parent_bounds_it_centres_on_the_parents_monitor_never_the_primary() {
        let m = left_retina();
        let p = place(None, Some(m), None, (600.0, 400.0), (320.0, 200.0));
        assert!(on(&p, &m) && p.x < 0.0, "{p:?}");
        // Nothing known at all: the last resort.
        let p = place(None, None, None, (600.0, 400.0), (320.0, 200.0));
        assert_eq!((p.x, p.y, p.scale), (100.0, 100.0, 1.0));
        // A parent but no monitor: centred on the parent, not clamped anywhere.
        let p = place(Some(rect(-3000.0, 0.0, 1000.0, 800.0)), None, None, (600.0, 400.0), (320.0, 200.0));
        assert_eq!(p.x, -2800.0);
    }

    #[test]
    fn nonsense_from_the_platform_or_the_ui_is_ignored() {
        let bad = Monitor {
            work: rect(f64::NAN, 0.0, 10.0, 10.0),
            scale: 0.0,
        };
        let p = place(
            Some(rect(10.0, 10.0, 800.0, 600.0)),
            Some(bad),
            rem(f64::INFINITY, 0.0, 500.0, 400.0),
            (600.0, 400.0),
            (320.0, 200.0),
        );
        assert!(!p.remembered && p.x.is_finite() && p.scale == 1.0);
        assert_eq!(
            Monitor {
                scale: f64::NAN,
                ..primary()
            }
            .scale(),
            1.0
        );
        assert_eq!(rem(1.0, 1.0, 0.0, 5.0).and_then(Remembered::sanitized), None);
        assert_eq!(
            rem(1e9, -1e9, 5.0, 5.0).and_then(Remembered::sanitized).map(|r| (r.dx, r.dy)),
            Some((MAX_OFFSET, -MAX_OFFSET))
        );
    }

    #[test]
    fn the_parents_monitor_is_the_one_it_overlaps_most_or_the_nearest() {
        let all = [primary(), left_retina(), above()];
        assert_eq!(monitor_of(&rect(-2900.0, 100.0, 2000.0, 1000.0), &all), Some(1));
        // Mostly on the primary, a little on the left display.
        assert_eq!(monitor_of(&rect(-200.0, 100.0, 1800.0, 1000.0), &all), Some(0));
        assert_eq!(monitor_of(&rect(100.0, -900.0, 800.0, 600.0), &all), Some(2));
        // Off every display (a monitor was unplugged): the nearest one.
        assert_eq!(monitor_of(&rect(-9000.0, 300.0, 800.0, 600.0), &all), Some(1));
        assert_eq!(monitor_of(&rect(0.0, 0.0, 10.0, 10.0), &[]), None);
    }

    #[test]
    fn a_refit_that_grows_past_the_bottom_moves_the_window_up_on_its_own_monitor() {
        let m = left_retina();
        // At 2x: 400 logical inner + title bar = 864 physical; from y=1500 it would end at 2364 > 1964.
        let at = keep_on_monitor(rect(-2000.0, 1500.0, 1200.0, 500.0), 400.0, Some(m));
        assert_eq!(at, Some((-2000.0, 1964.0 - 864.0)));
        // Already inside: nothing to do.
        assert_eq!(keep_on_monitor(rect(-2000.0, 100.0, 1200.0, 500.0), 400.0, Some(m)), None);
        assert_eq!(keep_on_monitor(rect(-2000.0, 100.0, 1200.0, 500.0), 400.0, None), None);
    }

    #[test]
    fn the_remembered_place_is_an_offset_from_the_parent() {
        let b = crate::panel_windows::Bounds {
            x: -1400.0,
            y: 300.0,
            width: 700.0,
            height: 500.0,
        };
        assert_eq!(remembered_of(b, (-1500.0, 200.0)), rem(100.0, 100.0, 700.0, 500.0));
    }

    #[test]
    fn the_kind_decides_what_of_a_remembered_place_is_kept() {
        let r = rem(10.0, 20.0, 900.0, 700.0);
        assert_eq!(memory_for(Kind::Decision, true, r, 300.0), None);
        assert_eq!(memory_for(Kind::Form, true, r, 300.0), rem(10.0, 20.0, 900.0, 300.0));
        assert_eq!(memory_for(Kind::Form, false, r, 300.0), r);
        assert_eq!(memory_for(Kind::Viewer, true, r, 300.0), r);
        assert_eq!(memory_for(Kind::Tool, false, None, 300.0), None);
    }
}
